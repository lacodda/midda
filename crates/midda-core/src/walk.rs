//! The scanner that works everywhere: a recursive read of the filesystem.
//!
//! This is the half of ADR 0001 that needs no elevation and no NTFS. It is the
//! slow one — a syscall per directory, several million of them on a system
//! volume — which is why it runs on a pool rather than on one thread, and why
//! it reports progress while it does.
//!
//! The parallel shape is deliberate. A directory's children can be read
//! independently of its siblings', so the natural unit of work is one
//! directory, and `rayon`'s work-stealing pool is what keeps a tree that is
//! deep in one branch and wide in another from serializing on the deep one. The
//! result comes back as an unordered pile of directory readings which is then
//! stitched into the flat arena on one thread: building the arena in parallel
//! would mean locking the `Vec` that every push appends to, and that lock is
//! the whole scan.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;

use crate::error::{Error, Result};
use crate::platform::{self, ASSUMED_CLUSTER_BYTES};
use crate::scanner::{Progress, Scanner};
use crate::size::Size;
use crate::tree::{Kind, Node, NodeId, ROOT, Tree};

/// Reads a tree by walking the filesystem.
#[derive(Debug, Default, Clone, Copy)]
pub struct WalkScanner;

impl WalkScanner {
    /// A new scanner. It holds nothing; the state of a scan lives in its
    /// arguments.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Scanner for WalkScanner {
    fn name(&self) -> &'static str {
        "walk"
    }

    fn available(&self) -> bool {
        // Reading directories is what every process may do. This is the
        // scanner that is always there, which is the whole point of it.
        true
    }

    fn scan(&self, root: &Path, progress: &Progress) -> Result<Tree> {
        let metadata = std::fs::metadata(root).map_err(|source| Error::Unreadable {
            path: root.to_path_buf(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(Error::NotADirectory(root.to_path_buf()));
        }

        let cluster_bytes = platform::cluster_bytes_of(root);
        let mut tree = Tree::new(root.to_path_buf(), cluster_bytes);
        let cluster = cluster_bytes.unwrap_or(ASSUMED_CLUSTER_BYTES);

        // The root's own timestamp. A subtree mtime that ignored the root would
        // call an empty directory undated rather than as old as it is.
        tree.stamp_root(metadata.modified().ok());

        // Breadth-first by level: every directory at one depth is read in
        // parallel, then all of their children are attached in one pass on this
        // thread, which is what keeps the arena lock-free and its node order
        // parent-before-child.
        let mut level = vec![(ROOT, root.to_path_buf())];
        while !level.is_empty() {
            if progress.cancelled() {
                return Err(Error::Cancelled);
            }

            let readings: Vec<Reading> = level.par_iter().map(|(id, path)| read_directory(*id, path, cluster, progress)).collect();

            let mut next = Vec::new();
            for reading in readings {
                for failure in reading.failures {
                    tree.skip(failure.path, failure.reason);
                }
                for entry in reading.entries {
                    let is_directory = matches!(entry.kind, Kind::Directory);
                    let id = tree.push(
                        reading.parent,
                        Node {
                            name: entry.name,
                            parent: reading.parent,
                            kind: entry.kind,
                            size: entry.size,
                            modified: entry.modified,
                            subtree_modified: entry.modified,
                            entries: 1,
                            children: Vec::new(),
                        },
                    );
                    if is_directory {
                        next.push((id, entry.path));
                    }
                }
            }
            level = next;
        }

        tree.roll_up();
        Ok(tree)
    }
}

/// What one directory turned out to hold.
struct Reading {
    parent: NodeId,
    entries: Vec<Entry>,
    failures: Vec<Failure>,
}

struct Entry {
    name: String,
    path: PathBuf,
    kind: Kind,
    size: Size,
    modified: Option<SystemTime>,
}

struct Failure {
    path: PathBuf,
    reason: String,
}

/// Reads one directory. Never fails: what cannot be read is reported as a
/// skipped entry, because a system volume always refuses something and a scan
/// that stopped at the first refusal would never reach the end.
fn read_directory(parent: NodeId, path: &Path, cluster: u64, progress: &Progress) -> Reading {
    let mut entries = Vec::new();
    let mut failures = Vec::new();

    let iterator = match std::fs::read_dir(path) {
        Ok(iterator) => iterator,
        Err(error) => {
            failures.push(Failure {
                path: path.to_path_buf(),
                reason: error.to_string(),
            });
            return Reading { parent, entries, failures };
        }
    };

    let mut counted_entries = 0_u64;
    let mut counted_bytes = 0_u64;

    for entry in iterator {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(Failure {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                });
                continue;
            }
        };

        let child = entry.path();
        // `symlink_metadata` rather than `metadata`: following a link would
        // count its target's bytes here and again where the target really
        // lives, and on Windows a directory junction can point at an ancestor,
        // which turns a scan into an endless one.
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                failures.push(Failure {
                    path: child,
                    reason: error.to_string(),
                });
                continue;
            }
        };

        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            // A link is an entry on the volume but its target's bytes are not
            // its own. Counted as an entry costing what the link itself costs.
            entries.push(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: child,
                kind: Kind::File,
                size: Size::zero(),
                modified: metadata.modified().ok(),
            });
            counted_entries += 1;
            continue;
        }

        let kind = if file_type.is_dir() { Kind::Directory } else { Kind::File };
        let size = if matches!(kind, Kind::Directory) {
            // A directory's own entry costs something on NTFS, but it is not
            // recoverable by deleting the directory alone and counting it would
            // put bytes in a total that no action returns. A directory's size
            // is what is inside it.
            Size::zero()
        } else {
            platform::measure(&child, &metadata, cluster)
        };

        counted_entries += 1;
        counted_bytes += size.allocated;

        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: child,
            kind,
            size,
            modified: metadata.modified().ok(),
        });
    }

    // One update per directory rather than one per file: the counters are read
    // by a repaint, not by a consumer that needs every step.
    progress.advance(counted_entries, counted_bytes);

    Reading { parent, entries, failures }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::size::SizeBasis;

    /// Builds a small tree on disk and returns the temporary directory holding
    /// it. Everything is synthetic — the line does not put real paths in
    /// fixtures.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path();

        fs::create_dir(root.join("project")).expect("create project");
        fs::write(root.join("project/main.rs"), vec![b'x'; 1000]).expect("write main.rs");

        fs::create_dir(root.join("project/target")).expect("create target");
        fs::write(root.join("project/target/build.bin"), vec![b'x'; 50_000]).expect("write build.bin");

        fs::create_dir(root.join("empty")).expect("create empty");
        fs::write(root.join("note.txt"), b"hello").expect("write note.txt");

        dir
    }

    #[test]
    fn a_walk_finds_every_entry() {
        let dir = fixture();
        let progress = Progress::new();
        let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");

        // root, project, main.rs, target, build.bin, empty, note.txt
        assert_eq!(tree.root().entries, 7);
        assert_eq!(tree.len(), 7);
    }

    #[test]
    fn folder_totals_are_the_sum_of_what_is_under_them() {
        let dir = fixture();
        let progress = Progress::new();
        let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");

        let project = tree
            .nodes()
            .find(|(_, node)| node.name == "project")
            .map(|(id, _)| id)
            .expect("the fixture has a project directory");

        assert_eq!(tree.node(project).size.logical, 51_000, "1000 + 50000 bytes of content");
        assert!(
            tree.node(project).size.allocated >= tree.node(project).size.logical,
            "the space a folder takes cannot be less than what it holds"
        );
    }

    #[test]
    fn the_root_total_is_the_whole_fixture() {
        let dir = fixture();
        let progress = Progress::new();
        let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");
        assert_eq!(tree.root().size.logical, 51_005, "1000 + 50000 + 5");
    }

    #[test]
    fn progress_ends_where_the_tree_does() {
        // The counter is what the window shows while a scan runs. If it drifts
        // from the tree, the window shows a number the result then contradicts.
        let dir = fixture();
        let progress = Progress::new();
        let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");

        // The root itself is not counted by the walk: it is not an entry the
        // walk read, it is where the walk started.
        assert_eq!(progress.entries(), tree.root().entries - 1);
        assert_eq!(progress.bytes(), tree.root().size.allocated);
    }

    #[test]
    fn an_empty_directory_is_a_node_with_nothing_in_it() {
        let dir = fixture();
        let progress = Progress::new();
        let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");

        let empty = tree.nodes().find(|(_, node)| node.name == "empty").expect("the fixture has an empty directory");
        assert_eq!(empty.1.size, Size::zero());
        assert_eq!(empty.1.entries, 1);
        assert!(empty.1.children.is_empty());
    }

    #[test]
    fn the_biggest_child_is_the_one_holding_the_big_file() {
        let dir = fixture();
        let progress = Progress::new();
        let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");

        let largest = tree.children_by_size(ROOT, SizeBasis::Allocated)[0];
        assert_eq!(tree.node(largest).name, "project");
    }

    #[test]
    fn a_path_in_the_tree_points_at_the_file_on_disk() {
        let dir = fixture();
        let progress = Progress::new();
        let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");

        let (id, _) = tree.nodes().find(|(_, node)| node.name == "build.bin").expect("the fixture has build.bin");
        let path = tree.path_of(id);
        assert!(path.exists(), "{} does not exist", path.display());
        assert_eq!(fs::metadata(&path).expect("metadata").len(), 50_000);
    }

    #[test]
    fn scanning_a_file_is_refused_rather_than_answered_with_an_empty_tree() {
        let dir = fixture();
        let progress = Progress::new();
        let error = WalkScanner::new()
            .scan(&dir.path().join("note.txt"), &progress)
            .expect_err("a file is not a scan root");
        assert!(matches!(error, Error::NotADirectory(_)));
    }

    #[test]
    fn scanning_something_that_is_not_there_says_so() {
        let progress = Progress::new();
        let error = WalkScanner::new()
            .scan(Path::new("no-such-directory-anywhere-at-all"), &progress)
            .expect_err("a missing root is an error");
        assert!(matches!(error, Error::Unreadable { .. }));
    }

    #[test]
    fn a_cancelled_scan_does_not_return_half_a_tree_as_a_whole_one() {
        // A partial total looks exactly like a total, and acting on it is how
        // someone deletes the wrong thing.
        let dir = fixture();
        let progress = Progress::new();
        progress.cancel();
        let error = WalkScanner::new().scan(dir.path(), &progress).expect_err("a cancelled scan is an error");
        assert!(matches!(error, Error::Cancelled));
    }

    #[test]
    fn the_walk_is_always_available() {
        assert!(WalkScanner::new().available());
        assert_eq!(WalkScanner::new().name(), "walk");
    }

    #[test]
    fn a_deep_tree_comes_back_whole() {
        // The level-by-level shape is the part most likely to go wrong on depth
        // rather than on breadth: one level that forgets to enqueue its
        // directories truncates the scan silently.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut path = dir.path().to_path_buf();
        for level in 0..12 {
            path.push(format!("level-{level}"));
            fs::create_dir(&path).expect("create a level");
            fs::write(path.join("leaf.bin"), vec![b'x'; 10]).expect("write a leaf");
        }

        let progress = Progress::new();
        let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");

        // 12 directories + 12 leaves + the root.
        assert_eq!(tree.root().entries, 25);
        assert_eq!(tree.root().size.logical, 120);
    }
}
