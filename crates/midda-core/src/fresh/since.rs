//! What changed since a moment — "who ate ten gigabytes overnight" — from the
//! change journal, with no snapshot taken beforehand.
//!
//! The journal says which files were created or written, and when; the tree,
//! brought up to date, says what each of them holds now. Together they answer
//! where the recently written bytes are. They do not answer how much a file
//! grew: the journal records that a file was written, not its size before. So
//! the answer is a tree of the files written since the moment, at what they
//! occupy now, with each one marked as new or as written to — and the window
//! says which of the two numbers it is showing (ADR 0009). The difference
//! between two moments is v0.8's, with snapshots.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::usn::{self, Record};
use crate::platform::UNIX_EPOCH_AS_FILETIME;
use crate::tree::{Node, NodeId, ROOT, Tree};

/// How a file came to be in the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Change {
    /// It did not exist at the moment asked about.
    Created,
    /// It existed, and was written to since.
    Written,
}

/// What was written since a moment.
#[derive(Debug, Clone)]
pub struct Changes {
    /// The files created or written since, under their folders, at what they
    /// occupy now. Folder totals add them up, so the biggest folder is where
    /// the recent bytes went.
    pub tree: Tree,
    /// How each file in [`Changes::tree`] came to be there.
    marks: HashMap<NodeId, Change>,
    /// Files that appeared since and are still there.
    pub created: u64,
    /// Files that existed and were written to since.
    pub written: u64,
    /// Files and folders that existed and are gone. What they held is not
    /// known — they are not there to be measured.
    pub deleted: u64,
    /// The moment asked about.
    pub since: SystemTime,
    /// The oldest change the journal still holds. Later than [`Changes::since`]
    /// means the answer covers a shorter span than was asked for.
    pub reaches_back_to: Option<SystemTime>,
}

impl Changes {
    /// How `id` — a file in [`Changes::tree`] — came to be there.
    #[must_use]
    pub fn mark(&self, id: NodeId) -> Option<Change> {
        self.marks.get(&id).copied()
    }

    /// Whether the journal covers the whole span asked about.
    #[must_use]
    pub fn covers(&self) -> bool {
        self.reaches_back_to.is_none_or(|oldest| oldest <= self.since)
    }
}

/// What one file did over the span.
#[derive(Debug, Default)]
struct Fate<'a> {
    created: bool,
    written: bool,
    /// Whether the last thing that happened to it was going.
    gone: bool,
    directory: bool,
    /// The newest record that names where the file is: not a rename's record
    /// of the name it left.
    last: Option<&'a Record>,
}

/// What was written since `since`, given the journal's records — oldest
/// first — and a way to find where a record's file is now.
///
/// `tree` is the tree as it is now: brought up to date with the same journal,
/// so that every file the records name is measured as it stands.
#[must_use]
pub fn written_since(tree: &Tree, records: &[Record], since: SystemTime, mut place: impl FnMut(&Record) -> Option<PathBuf>) -> Changes {
    let from = filetime(since);
    let mut fates: HashMap<u64, Fate<'_>> = HashMap::new();
    let mut order = Vec::new();
    for record in records.iter().filter(|record| record.time >= from) {
        let fate = fates.entry(record.file).or_insert_with(|| {
            order.push(record.file);
            Fate::default()
        });
        if record.reason & usn::FILE_CREATE != 0 {
            fate.created = true;
            fate.gone = false;
        }
        if record.reason & (usn::DATA_OVERWRITE | usn::DATA_EXTEND | usn::DATA_TRUNCATION) != 0 {
            fate.written = true;
        }
        if record.reason & usn::FILE_DELETE != 0 {
            fate.gone = true;
        }
        if record.reason & usn::RENAME_OLD_NAME == 0 {
            fate.last = Some(record);
        }
        fate.directory = record.attributes & usn::DIRECTORY != 0;
    }

    let mut changes = Changes {
        tree: Tree::new(tree.root_path().to_path_buf(), tree.cluster_bytes()),
        marks: HashMap::new(),
        created: 0,
        written: 0,
        deleted: 0,
        since,
        reaches_back_to: records.first().and_then(|record| crate::platform::filetime_to_system_time(record.time)),
    };
    changes.tree.stamp_root(tree.root().modified, tree.root().identity);
    let mut placed: HashMap<NodeId, NodeId> = HashMap::from([(ROOT, ROOT)]);

    for file in order {
        let fate = &fates[&file];
        if fate.gone {
            // Something that came and went within the span changed nothing.
            if !fate.created {
                changes.deleted += 1;
            }
            continue;
        }
        if fate.directory || !(fate.created || fate.written) {
            continue;
        }
        let Some(id) = fate.last.and_then(&mut place).and_then(|path| find(tree, &path)) else {
            continue;
        };
        if tree.node(id).is_directory() {
            continue;
        }
        let change = if fate.created { Change::Created } else { Change::Written };
        let copied = copy(tree, id, &mut changes.tree, &mut placed);
        changes.marks.insert(copied, change);
        match change {
            Change::Created => changes.created += 1,
            Change::Written => changes.written += 1,
        }
    }
    changes.tree.roll_up();
    changes
}

/// The node at `path` in `tree`, when the tree holds it under that name.
fn find(tree: &Tree, path: &Path) -> Option<NodeId> {
    let names = super::below(tree.root_path(), path)?;
    names.iter().try_fold(ROOT, |at, name| tree.child(at, name))
}

/// Copies the file `id` of `tree` into `into`, with the folders above it,
/// and returns its id there.
fn copy(tree: &Tree, id: NodeId, into: &mut Tree, placed: &mut HashMap<NodeId, NodeId>) -> NodeId {
    let parent = tree.node(id).parent;
    let folder = match placed.get(&parent) {
        Some(&folder) => folder,
        None => {
            let above = copy_folder(tree, parent, into, placed);
            placed.insert(parent, above);
            above
        }
    };
    let node = tree.node(id);
    into.insert(
        folder,
        Node {
            size: node.size,
            modified: node.modified,
            subtree_modified: node.modified,
            traits: node.traits,
            links: node.links,
            identity: node.identity,
            ..Node::new(node.name.clone(), folder, node.kind)
        },
    )
}

/// A folder of `tree`, empty, in `into` — and the folders above it.
fn copy_folder(tree: &Tree, id: NodeId, into: &mut Tree, placed: &mut HashMap<NodeId, NodeId>) -> NodeId {
    if let Some(&there) = placed.get(&id) {
        return there;
    }
    let parent = copy_folder(tree, tree.node(id).parent, into, placed);
    let node = tree.node(id);
    let there = into.insert(
        parent,
        Node {
            modified: node.modified,
            subtree_modified: node.modified,
            identity: node.identity,
            ..Node::new(node.name.clone(), parent, node.kind)
        },
    );
    placed.insert(id, there);
    there
}

/// A moment as a `FILETIME`, which is how the journal stamps its records.
fn filetime(moment: SystemTime) -> u64 {
    match moment.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => UNIX_EPOCH_AS_FILETIME
            .saturating_add(since.as_secs().saturating_mul(10_000_000))
            .saturating_add(u64::from(since.subsec_nanos() / 100)),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::size::Size;
    use crate::tree::Kind;

    const HOUR: u64 = 3600 * 10_000_000;

    /// The moment of a test's `now`, and the same as a `FILETIME`.
    fn now() -> (SystemTime, u64) {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        (now, filetime(now))
    }

    fn file(name: &str, parent: NodeId, bytes: u64) -> Node {
        Node {
            size: Size::flat(bytes),
            ..Node::new(name.into(), parent, Kind::File)
        }
    }

    /// root / { downloads / { big.iso, old.zip }, keep.txt, logs / app.log },
    /// pushed level by level in the listing order, as a scan pushes them.
    fn tree() -> Tree {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let downloads = tree.push(ROOT, Node::new("downloads".into(), ROOT, Kind::Directory));
        tree.push(ROOT, file("keep.txt", ROOT, 5));
        let logs = tree.push(ROOT, Node::new("logs".into(), ROOT, Kind::Directory));
        tree.push(downloads, file("big.iso", downloads, 9_000_000_000));
        tree.push(downloads, file("old.zip", downloads, 1_000));
        tree.push(logs, file("app.log", logs, 50_000));
        tree.roll_up();
        tree
    }

    fn record(usn: i64, time: u64, file: u64, reason: u32, name: &str) -> Record {
        Record {
            usn,
            time,
            file,
            parent: 1,
            reason,
            attributes: 0x20,
            name: name.into(),
        }
    }

    /// Where each test file is: by the record's file number.
    fn place(record: &Record) -> Option<PathBuf> {
        let folder = match record.file {
            10 | 11 | 15 => "C:/scan/downloads",
            20 => "C:/scan/logs",
            _ => "C:/scan",
        };
        Some(Path::new(folder).join(&record.name))
    }

    #[test]
    fn a_file_downloaded_overnight_is_where_the_bytes_went() {
        let (now, at) = now();
        let records = [
            record(1, at - 30 * HOUR, 11, usn::DATA_EXTEND, "old.zip"),
            record(2, at - 8 * HOUR, 10, usn::FILE_CREATE, "big.iso"),
            record(3, at - 8 * HOUR, 10, usn::DATA_EXTEND, "big.iso"),
            record(4, at - 7 * HOUR, 10, usn::DATA_EXTEND | 0x8000_0000, "big.iso"),
            record(5, at - 2 * HOUR, 20, usn::DATA_EXTEND, "app.log"),
        ];
        let changes = written_since(&tree(), &records, now - Duration::from_secs(24 * 3600), place);

        assert_eq!((changes.created, changes.written, changes.deleted), (1, 1, 0));
        assert_eq!(changes.tree.root().size.logical, 9_000_050_000, "big.iso and app.log, at their size now");
        let downloads = changes.tree.child(ROOT, "downloads").expect("the folder of the download");
        assert_eq!(changes.tree.node(downloads).children.len(), 1, "old.zip was written before the span");
        let big = changes.tree.child(downloads, "big.iso").expect("the download");
        assert_eq!(changes.mark(big), Some(Change::Created));
        let log = changes.tree.child(changes.tree.child(ROOT, "logs").expect("logs"), "app.log").expect("the log");
        assert_eq!(changes.mark(log), Some(Change::Written), "it existed: what it holds now is not what it grew by");
        assert!(changes.covers());
    }

    #[test]
    fn what_came_and_went_within_the_span_is_not_counted() {
        let (now, at) = now();
        let records = [
            record(1, at - 3 * HOUR, 30, usn::FILE_CREATE, "temp.tmp"),
            record(2, at - 2 * HOUR, 30, usn::FILE_DELETE, "temp.tmp"),
            record(3, at - HOUR, 31, usn::FILE_DELETE, "was-here.txt"),
        ];
        let changes = written_since(&tree(), &records, now - Duration::from_secs(24 * 3600), place);
        assert_eq!((changes.created, changes.written, changes.deleted), (0, 0, 1));
        assert!(changes.tree.is_empty());
    }

    #[test]
    fn a_rename_alone_writes_nothing() {
        let (now, at) = now();
        let records = [
            record(1, at - HOUR, 40, usn::RENAME_OLD_NAME, "draft.txt"),
            record(2, at - HOUR, 40, usn::RENAME_NEW_NAME, "keep.txt"),
        ];
        let changes = written_since(&tree(), &records, now - Duration::from_secs(24 * 3600), place);
        assert_eq!((changes.created, changes.written), (0, 0));
    }

    #[test]
    fn a_file_found_under_its_new_name_after_a_rename() {
        // Written, then renamed: the record of the old name is not where it is.
        let (now, at) = now();
        let records = [
            record(1, at - 2 * HOUR, 41, usn::FILE_CREATE | usn::DATA_EXTEND, "draft.txt"),
            record(2, at - HOUR, 41, usn::RENAME_OLD_NAME, "draft.txt"),
            record(3, at - HOUR, 41, usn::RENAME_NEW_NAME, "keep.txt"),
        ];
        let changes = written_since(&tree(), &records, now - Duration::from_secs(24 * 3600), place);
        assert_eq!(changes.created, 1);
        assert!(changes.tree.child(ROOT, "keep.txt").is_some());
    }

    #[test]
    fn a_journal_shorter_than_the_span_says_so() {
        let (now, at) = now();
        let records = [record(1, at - HOUR, 20, usn::DATA_EXTEND, "app.log")];
        let changes = written_since(&tree(), &records, now - Duration::from_secs(24 * 3600), place);
        assert!(!changes.covers(), "the journal reaches back an hour, not a day");
        assert!(changes.reaches_back_to.is_some_and(|oldest| oldest > changes.since));
    }
}
