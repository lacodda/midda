//! Records into a tree.
//!
//! The MFT lists files by record number, each naming its parent. A tree needs
//! the opposite — each directory listing its children — and it needs them in
//! the order the walk would have pushed them, so that the two scanners produce
//! the same arena node for node. This is that inversion, and nothing in it
//! touches a volume.

use super::{FIRST_USER_RECORD, Record, Records, record_of, sequence_of};
use crate::error::{Error, Result};
use crate::links;
use crate::platform::{FileIdentity, filetime_to_system_time};
use crate::scanner::Progress;
use crate::size::Size;
use crate::tree::{Kind, Node, NodeId, ROOT, Tree, listing_order};

/// One name in one directory, pointing at the record it names.
struct Edge<'a> {
    parent: u64,
    child: u64,
    name: &'a str,
}

/// Builds the tree under `root_record` from what the table said.
///
/// `root` is the path the reader asked for, `volume` the serial the walk's file
/// identities carry, so that a hard link seen by both scanners has the same
/// identity in both trees.
///
/// # Errors
///
/// When `root_record` is not a directory in use, or the scan was cancelled.
pub fn assemble(records: &Records, root_record: u64, root: &std::path::Path, volume: u64, cluster_bytes: Option<u64>, progress: &Progress) -> Result<Tree> {
    let root_entry = records
        .get(root_record)
        .filter(|record| record.directory)
        .ok_or_else(|| Error::Unavailable(format!("record {root_record} is not a directory on this volume")))?;

    let mut edges = edges(records);
    // By directory first, so each directory's children are one contiguous run
    // found by binary search; then by the tree's listing order, which is the
    // order the walk pushes them in.
    edges.sort_unstable_by(|a, b| a.parent.cmp(&b.parent).then_with(|| listing_order(a.name, b.name)));

    let mut tree = Tree::new(root.to_path_buf(), cluster_bytes);
    let cluster = cluster_bytes.unwrap_or(crate::platform::ASSUMED_CLUSTER_BYTES);
    tree.stamp_root(root_entry.modified.and_then(filetime_to_system_time));

    // Breadth-first by level, as the walk does it: the node order is the order
    // the deduplication hands shared bytes out in, and it has to be the walk's.
    let mut visited = std::collections::HashSet::new();
    visited.insert(root_record);
    let mut level: Vec<(NodeId, u64)> = vec![(ROOT, root_record)];
    while !level.is_empty() {
        if progress.cancelled() {
            return Err(Error::Cancelled);
        }

        let mut next = Vec::new();
        for (parent_id, directory) in level {
            let start = edges.partition_point(|edge| edge.parent < directory);
            let end = edges.partition_point(|edge| edge.parent <= directory);

            let mut counted_bytes = 0_u64;
            for edge in &edges[start..end] {
                let Some(record) = records.get(edge.child) else {
                    continue;
                };
                let node = node_for(edge, record, parent_id, volume, cluster);
                counted_bytes += node.size.allocated;
                let is_directory = node.is_directory();
                let id = tree.push(parent_id, node);
                // A directory is entered once. On a sound volume that is
                // already true — a directory has one name — but a corrupt
                // record naming an ancestor as its child would otherwise turn a
                // scan into an endless one.
                if is_directory && visited.insert(edge.child) {
                    next.push((id, edge.child));
                }
            }
            progress.advance((end - start) as u64, counted_bytes);
        }
        level = next;
    }

    // The same two passes, in the same order, as the walk: shared bytes moved
    // off second names before the totals are folded upward.
    let shared = links::count_shared_once(&mut tree);
    tree.record_shared(shared);
    tree.roll_up();
    Ok(tree)
}

/// Every name that hangs a record under a live directory.
fn edges(records: &Records) -> Vec<Edge<'_>> {
    let mut edges = Vec::new();
    for (child, record) in records.iter() {
        // The volume's own metadata files: the walk never sees them, because
        // the volume does not list them.
        if child < FIRST_USER_RECORD && child != super::ROOT_RECORD {
            continue;
        }
        for name in &record.names {
            let parent = record_of(name.parent);
            // The root names itself as its own parent.
            if parent == child {
                continue;
            }
            let Some(directory) = records.get(parent) else {
                continue;
            };
            // A name pointing at a record that has been reused since is an
            // orphan, not a child of whatever lives there now. A sequence of
            // zero is a reference that does not say, and is taken at its word.
            let expected = sequence_of(name.parent);
            if !directory.directory || (expected != 0 && expected != directory.sequence) {
                continue;
            }
            edges.push(Edge {
                parent,
                child,
                name: &name.name,
            });
        }
    }
    edges
}

/// The node the walk would have pushed for this name.
fn node_for(edge: &Edge<'_>, record: &Record, parent: NodeId, volume: u64, cluster: u64) -> Node {
    let modified = record.modified.and_then(filetime_to_system_time);
    let base = Node {
        modified,
        subtree_modified: modified,
        ..Node::new(edge.name.to_owned(), parent, Kind::File)
    };

    if record.is_link() {
        // A symbolic link or a junction: an entry, costing what the walk says
        // it costs, and never followed.
        return base;
    }
    if record.directory {
        // A directory's own index is not recoverable by deleting the directory
        // alone; the walk counts nothing for it, and neither does this.
        return Node { kind: Kind::Directory, ..base };
    }

    Node {
        size: record.occupied(cluster),
        traits: record.traits(),
        links: Some(u32::try_from(record.names.len()).unwrap_or(u32::MAX)),
        identity: Some(FileIdentity {
            volume,
            // The 128-bit file id NTFS reports for a file is its 64-bit file
            // reference, sequence number included, widened with zeroes — which
            // is what the walk reads through `FileIdInfo`.
            file: u128::from((u64::from(record.sequence) << 48) | edge.child),
        }),
        ..base
    }
}

/// The size a resident stream of `bytes` occupies: inside the record, rounded
/// to the eight-byte alignment of an attribute.
///
/// Measured through `AllocationSize` on NTFS: 100 bytes occupy 104, one byte
/// occupies 8, none occupy none (see [`crate::platform`]).
#[must_use]
#[cfg_attr(not(windows), allow(dead_code, reason = "only the volume reader, which is Windows-only, sizes resident streams"))]
pub const fn resident_allocation(bytes: u64) -> u64 {
    bytes.div_ceil(8) * 8
}

/// A resident unnamed stream of `bytes`, both ways.
#[must_use]
#[cfg_attr(not(windows), allow(dead_code, reason = "only the volume reader, which is Windows-only, sizes resident streams"))]
pub const fn resident_size(bytes: u64) -> Size {
    Size {
        logical: bytes,
        allocated: resident_allocation(bytes),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::mft::{Name, ROOT_RECORD};
    use crate::platform::UNIX_EPOCH_AS_FILETIME;
    use crate::platform::traits_from_attributes;
    use crate::traits::Traits;

    const SERIAL: u64 = 0xABCD;
    /// Sequence numbers are part of a reference; the fixtures give each record
    /// its own so a test that ignored them could not pass by accident.
    fn reference(record: u64, sequence: u16) -> u64 {
        (u64::from(sequence) << 48) | record
    }

    struct Table {
        records: Records,
    }

    impl Table {
        fn new() -> Self {
            let mut records = Records::new();
            let root = records.entry(ROOT_RECORD);
            *root = Record {
                in_use: true,
                sequence: 5,
                directory: true,
                names: vec![Name {
                    parent: reference(ROOT_RECORD, 5),
                    name: ".".into(),
                }],
                modified: Some(UNIX_EPOCH_AS_FILETIME + 10_000_000),
                ..Record::default()
            };
            Self { records }
        }

        fn directory(&mut self, number: u64, parent: u64, name: &str) -> &mut Self {
            let sequence = self.records.get(parent).map(|record| record.sequence).expect("the parent exists");
            *self.records.entry(number) = Record {
                in_use: true,
                sequence: 1,
                directory: true,
                names: vec![Name {
                    parent: reference(parent, sequence),
                    name: name.into(),
                }],
                ..Record::default()
            };
            self
        }

        fn file(&mut self, number: u64, names: &[(u64, &str)], size: Size) -> &mut Self {
            let names = names
                .iter()
                .map(|&(parent, name)| Name {
                    parent: reference(parent, self.records.get(parent).map_or(0, |record| record.sequence)),
                    name: name.into(),
                })
                .collect();
            *self.records.entry(number) = Record {
                in_use: true,
                sequence: 3,
                names,
                size,
                ..Record::default()
            };
            self
        }

        fn build(&self, root: u64) -> Tree {
            assemble(&self.records, root, Path::new("C:/scan"), SERIAL, Some(4096), &Progress::new()).expect("the tree assembles")
        }
    }

    fn names(tree: &Tree, id: NodeId) -> Vec<&str> {
        tree.node(id).children.iter().map(|&child| tree.node(child).name.as_str()).collect()
    }

    fn find(tree: &Tree, name: &str) -> NodeId {
        tree.nodes().find(|(_, node)| node.name == name).map(|(id, _)| id).expect("the node exists")
    }

    #[test]
    fn a_table_of_records_becomes_the_tree_they_describe() {
        let mut table = Table::new();
        table
            .directory(30, ROOT_RECORD, "project")
            .file(31, &[(30, "main.rs")], resident_size(1000))
            .directory(32, 30, "target")
            .file(
                33,
                &[(32, "build.bin")],
                Size {
                    logical: 50_000,
                    allocated: 53_248,
                },
            )
            .file(34, &[(ROOT_RECORD, "note.txt")], resident_size(5));
        let tree = table.build(ROOT_RECORD);

        assert_eq!(tree.root().entries, 6, "root, project, main.rs, target, build.bin, note.txt");
        assert_eq!(tree.root().size.logical, 51_005);
        assert_eq!(tree.node(find(&tree, "project")).size.logical, 51_000);
        assert_eq!(tree.path_of(find(&tree, "build.bin")), Path::new("C:/scan/project/target/build.bin"));
    }

    #[test]
    fn children_are_pushed_in_the_listing_order_not_the_record_order() {
        let mut table = Table::new();
        table
            .file(40, &[(ROOT_RECORD, "delta")], resident_size(1))
            .file(41, &[(ROOT_RECORD, "Bravo")], resident_size(1))
            .file(42, &[(ROOT_RECORD, "alpha")], resident_size(1))
            .file(43, &[(ROOT_RECORD, "Charlie")], resident_size(1));
        let tree = table.build(ROOT_RECORD);
        assert_eq!(names(&tree, ROOT), ["alpha", "Bravo", "Charlie", "delta"]);
    }

    #[test]
    fn the_tree_is_laid_out_level_by_level_like_the_walk() {
        // A deep child pushed before a shallow cousin would put it first in
        // node order, and the shallow cousin would lose shared bytes the walk
        // gives it.
        let mut table = Table::new();
        table
            .directory(50, ROOT_RECORD, "a")
            .directory(51, 50, "deep")
            .file(52, &[(51, "leaf")], resident_size(1))
            .directory(53, ROOT_RECORD, "b")
            .file(54, &[(53, "shallow")], resident_size(1));
        let tree = table.build(ROOT_RECORD);
        assert!(find(&tree, "shallow") < find(&tree, "leaf"), "a level is finished before the next one starts");
    }

    #[test]
    fn a_subfolder_scan_holds_only_what_is_under_it() {
        let mut table = Table::new();
        table
            .directory(60, ROOT_RECORD, "inside")
            .file(61, &[(60, "kept")], resident_size(10))
            .file(62, &[(ROOT_RECORD, "outside")], resident_size(99));
        let tree = table.build(60);
        assert_eq!(tree.len(), 2, "the scanned folder and its one file");
        assert_eq!(tree.root().size.logical, 10);
    }

    #[test]
    fn every_hard_link_is_an_entry_and_the_bytes_are_counted_once() {
        let mut table = Table::new();
        table.directory(70, ROOT_RECORD, "store").directory(71, ROOT_RECORD, "project").file(
            72,
            &[(70, "pkg.bin"), (71, "pkg.bin")],
            Size {
                logical: 50_000,
                allocated: 53_248,
            },
        );
        let tree = table.build(ROOT_RECORD);

        assert_eq!(tree.root().entries, 5, "root, two folders, and the file under each of its names");
        assert_eq!(tree.root().size.allocated, 53_248, "the bytes are on the volume once");
        assert_eq!(tree.shared().shared_names, 1);

        // `project` sorts before `store`, so its name keeps the bytes — the
        // listing order, stated on screen, decides.
        let project = tree.node(find(&tree, "project"));
        assert_eq!(project.size.allocated, 53_248);
        let under_project = tree.node(project.children[0]);
        assert_eq!(under_project.links, Some(2));
        assert!(under_project.traits.has(Traits::LINKED | Traits::COUNTED_HERE));
    }

    #[test]
    fn a_hard_link_carries_the_identity_the_walk_reads() {
        let mut table = Table::new();
        table.file(0x1234, &[(ROOT_RECORD, "one")], resident_size(1));
        let tree = table.build(ROOT_RECORD);
        let identity = tree.node(find(&tree, "one")).identity.expect("an identity");
        assert_eq!(identity.volume, SERIAL);
        assert_eq!(identity.file, (3_u128 << 48) | 0x1234, "the file reference with its sequence number");
    }

    #[test]
    fn a_link_is_an_entry_that_costs_nothing_and_is_never_followed() {
        let mut table = Table::new();
        table.directory(80, ROOT_RECORD, "real").file(81, &[(80, "big.bin")], resident_size(700));
        // A junction to `real`: a directory record, with a name-surrogate tag.
        table.directory(82, ROOT_RECORD, "junction");
        table.records.entry(82).reparse_tag = Some(0xA000_0003);
        // Something that claims to be inside the junction must not be reached
        // through it.
        table.file(83, &[(82, "ghost")], resident_size(900));

        let tree = table.build(ROOT_RECORD);
        let junction = tree.node(find(&tree, "junction"));
        assert_eq!(junction.kind, Kind::File, "the walk reports a link as an entry, not a folder");
        assert_eq!(junction.size, Size::zero());
        assert_eq!(junction.links, None);
        assert_eq!(junction.identity, None);
        assert!(tree.nodes().all(|(_, node)| node.name != "ghost"));
        assert_eq!(tree.root().size.logical, 700);
    }

    #[test]
    fn a_placeholder_is_a_file_and_says_so() {
        let mut table = Table::new();
        table.file(
            90,
            &[(ROOT_RECORD, "cloud.docx")],
            Size {
                logical: 5_898_024,
                allocated: 0,
            },
        );
        let record = table.records.entry(90);
        record.reparse_tag = Some(0x9000_601A);
        record.attributes = 0x0040_1620;

        let tree = table.build(ROOT_RECORD);
        let cloud = tree.node(find(&tree, "cloud.docx"));
        assert_eq!(cloud.kind, Kind::File);
        assert_eq!(cloud.size.allocated, 0);
        assert_eq!(
            cloud.traits,
            traits_from_attributes(0x0040_0020),
            "the same traits the walk reads through the filter"
        );
    }

    #[test]
    fn the_volumes_own_files_are_not_in_the_tree() {
        let mut table = Table::new();
        table.file(0, &[(ROOT_RECORD, "$MFT")], Size::flat(1 << 30));
        table.file(6, &[(ROOT_RECORD, "$Bitmap")], Size::flat(1 << 20));
        table.file(100, &[(ROOT_RECORD, "mine.txt")], resident_size(3));
        let tree = table.build(ROOT_RECORD);
        assert_eq!(names(&tree, ROOT), ["mine.txt"]);
    }

    #[test]
    fn a_name_in_a_reused_directory_record_is_an_orphan() {
        let mut table = Table::new();
        table.directory(110, ROOT_RECORD, "now");
        // A name that remembers record 110 at sequence 9; the record is at 1.
        *table.records.entry(111) = Record {
            in_use: true,
            sequence: 1,
            names: vec![Name {
                parent: reference(110, 9),
                name: "stale".into(),
            }],
            size: resident_size(4),
            ..Record::default()
        };
        let tree = table.build(ROOT_RECORD);
        assert!(tree.nodes().all(|(_, node)| node.name != "stale"));
    }

    #[test]
    fn a_directory_that_names_its_ancestor_does_not_loop() {
        let mut table = Table::new();
        table.directory(120, ROOT_RECORD, "a");
        // Corrupt: the root claims a second name inside its own child.
        table.records.entry(ROOT_RECORD).names.push(Name {
            parent: reference(120, 1),
            name: "loop".into(),
        });
        let tree = table.build(ROOT_RECORD);
        assert!(tree.len() < 10, "the scan finished with {} nodes", tree.len());
    }

    #[test]
    fn a_root_that_is_not_a_directory_is_refused() {
        let mut table = Table::new();
        table.file(130, &[(ROOT_RECORD, "file")], resident_size(1));
        let error = assemble(&table.records, 130, Path::new("C:/file"), SERIAL, None, &Progress::new()).expect_err("a file is not a root");
        assert!(matches!(error, Error::Unavailable(_)));
    }

    #[test]
    fn a_cancelled_assembly_returns_no_tree() {
        let table = Table::new();
        let progress = Progress::new();
        progress.cancel();
        let error = assemble(&table.records, ROOT_RECORD, Path::new("C:/"), SERIAL, None, &progress).expect_err("cancelled");
        assert!(matches!(error, Error::Cancelled));
    }

    #[test]
    fn progress_ends_where_the_tree_does() {
        let mut table = Table::new();
        table.directory(140, ROOT_RECORD, "d").file(
            141,
            &[(140, "x")],
            Size {
                logical: 5000,
                allocated: 8192,
            },
        );
        let progress = Progress::new();
        let tree = assemble(&table.records, ROOT_RECORD, Path::new("C:/"), SERIAL, None, &progress).expect("assembles");
        assert_eq!(progress.entries(), tree.root().entries - 1);
        assert_eq!(progress.bytes(), tree.root().size.allocated);
    }

    #[test]
    fn a_filetime_converts_to_the_same_instant_to_the_hundred_nanoseconds() {
        let after = filetime_to_system_time(UNIX_EPOCH_AS_FILETIME + 12_345_678_901).expect("representable");
        assert_eq!(
            after.duration_since(SystemTime::UNIX_EPOCH).expect("after the epoch"),
            Duration::from_nanos(1_234_567_890_100)
        );
        assert_eq!(filetime_to_system_time(UNIX_EPOCH_AS_FILETIME), Some(SystemTime::UNIX_EPOCH));
        assert!(filetime_to_system_time(0).is_some_and(|time| time < SystemTime::UNIX_EPOCH));
    }

    #[test]
    fn a_resident_stream_occupies_its_length_rounded_to_eight() {
        assert_eq!(resident_allocation(0), 0);
        assert_eq!(resident_allocation(1), 8);
        assert_eq!(resident_allocation(100), 104);
        assert_eq!(resident_allocation(104), 104);
    }
}
