//! The scanned tree: what was found, how big it is, and how old it is.
//!
//! The tree is stored flat — one `Vec<Node>` with parent and child indices —
//! rather than as nested `Box`es. A system volume holds a few million entries,
//! and the difference matters in three places at once: building it costs one
//! allocation per batch instead of one per node, walking it for a treemap is a
//! linear pass over contiguous memory, and an index is `u32` where a pointer is
//! eight bytes and cannot be serialized.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::links::Deduplicated;
use crate::platform::FileIdentity;
use crate::size::{Size, SizeBasis};
use crate::traits::Traits;

/// Where a node sits in the flat arena.
///
/// A `u32` caps a tree at ~4.3 billion entries, which is two orders of
/// magnitude past the largest volume anyone points this at, and halves the
/// memory the index costs against a `usize`.
pub type NodeId = u32;

/// The root of every scan, at index 0.
pub const ROOT: NodeId = 0;

/// What a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A directory. Its size is the sum of everything beneath it.
    Directory,
    /// A regular file.
    File,
}

/// One entry in the tree.
///
/// Times are optional because a filesystem is allowed to refuse them, and a
/// scanner that invented a timestamp would poison every "what is old" question
/// the later versions are built on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// The entry's own name, not its path. The path is walked up through
    /// `parent`, so a deep tree does not store its prefix a million times.
    pub name: String,
    /// Which parent this hangs from. The root's parent is itself.
    pub parent: NodeId,
    pub kind: Kind,
    /// For a file, what it takes. For a directory, what everything beneath it
    /// takes.
    pub size: Size,
    /// When this entry itself was last written.
    pub modified: Option<SystemTime>,
    /// The most recent `modified` anywhere in this subtree, this entry
    /// included.
    ///
    /// This is the age of the *layer*, and it is what "abandoned" means later:
    /// a `target/` directory whose own mtime is today because a tool touched it
    /// is still five years old if nothing inside it has changed. A directory's
    /// own mtime cannot answer that; this can, and it costs one `max` per node
    /// during the roll-up rather than a second pass.
    pub subtree_modified: Option<SystemTime>,
    /// How many entries are in this subtree, this one included.
    pub entries: u64,
    /// Children, in scan order. Empty for a file.
    pub children: Vec<NodeId>,
    /// Why this entry's two sizes differ, when they do — and, after
    /// [`crate::links::count_shared_once`], whether its bytes were counted here
    /// or under another name.
    pub traits: Traits,
    /// How many names this file has on the volume, when the platform said.
    ///
    /// `None` means the question could not be asked, which is not the same as
    /// `Some(1)`: the second is a measurement, the first is a gap, and the
    /// deduplication treats only the measurement as evidence.
    pub links: Option<u32>,
    /// What identifies this file's bytes on its volume, when the platform said.
    ///
    /// The key the deduplication matches names on. Kept on the node rather than
    /// discarded after that pass so a later version can say *where* the other
    /// name is without rescanning — and so a test can show that two unknowns
    /// were never treated as a match.
    pub identity: Option<FileIdentity>,
}

impl Node {
    /// Whether this is a directory.
    #[must_use]
    pub const fn is_directory(&self) -> bool {
        matches!(self.kind, Kind::Directory)
    }

    /// A node with nothing unusual about it: one name, no traits, no identity
    /// asked for.
    ///
    /// The shape every directory has and most scanner fixtures want, so that
    /// adding a field to `Node` does not mean editing a hundred literals.
    #[must_use]
    pub fn new(name: String, parent: NodeId, kind: Kind) -> Self {
        Self {
            name,
            parent,
            kind,
            size: Size::zero(),
            modified: None,
            subtree_modified: None,
            entries: 1,
            children: Vec::new(),
            traits: Traits::none(),
            links: None,
            identity: None,
        }
    }
}

/// A scanned tree, flat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    /// Where the scan started. Node 0's name is this path's last component; the
    /// rest of it lives here so `path_of` can rebuild an absolute path.
    root: PathBuf,
    nodes: Vec<Node>,
    /// The cluster size of the volume the root sits on, in bytes, when it could
    /// be determined. Kept so a consumer can explain why three thousand tiny
    /// files cost twelve megabytes.
    cluster_bytes: Option<u64>,
    /// Entries the scan could not read, with the reason. A scan of a system
    /// volume always has some — `System Volume Information`, files held open,
    /// another user's profile — and hiding them would make the totals quietly
    /// wrong. See [`Tree::skipped`].
    skipped: Vec<Skipped>,
    /// What the deduplication of shared bytes found, once it has run.
    ///
    /// Kept on the tree because it explains a number the reader can see: a
    /// folder whose contents look larger than the folder is one holding second
    /// names, and without this the tree offers no way to say so.
    shared: Deduplicated,
}

/// Something the scan could not look at.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skipped {
    pub path: PathBuf,
    /// The reason, already formatted: the error itself does not survive
    /// serialization and the window only ever shows the message.
    pub reason: String,
}

impl Tree {
    /// Starts a tree at `root`. Used by scanners; consumers read.
    #[must_use]
    pub fn new(root: PathBuf, cluster_bytes: Option<u64>) -> Self {
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            // A volume root has no file name — the whole path is the name, and
            // showing an empty row would be a bug on the first screen.
            .unwrap_or_else(|| root.to_string_lossy().into_owned());

        Self {
            nodes: vec![Node::new(name, ROOT, Kind::Directory)],
            root,
            cluster_bytes,
            skipped: Vec::new(),
            shared: Deduplicated::default(),
        }
    }

    /// Where the scan started.
    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root
    }

    /// The cluster size of the volume, when it is known.
    #[must_use]
    pub const fn cluster_bytes(&self) -> Option<u64> {
        self.cluster_bytes
    }

    /// Everything the scan could not read.
    #[must_use]
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// Records something the scan could not read.
    pub fn skip(&mut self, path: PathBuf, reason: String) {
        self.skipped.push(Skipped { path, reason });
    }

    /// What the deduplication of shared bytes found.
    ///
    /// Zeroes until [`crate::links::count_shared_once`] has run, and on nearly
    /// every volume zeroes afterwards too.
    #[must_use]
    pub const fn shared(&self) -> Deduplicated {
        self.shared
    }

    /// Records what the deduplication found. Called by scanners.
    pub const fn record_shared(&mut self, shared: Deduplicated) {
        self.shared = shared;
    }

    /// Puts the scan root's own timestamp on node 0.
    ///
    /// The root is the one node a scanner does not push, so it is the one node
    /// whose timestamp needs a door of its own. Without it an empty directory
    /// would come back undated rather than as old as it is.
    pub fn stamp_root(&mut self, modified: Option<SystemTime>) {
        let root = &mut self.nodes[ROOT as usize];
        root.modified = modified;
        root.subtree_modified = modified;
    }

    /// How many nodes are in the tree.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the tree holds nothing but its root.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Reads a node.
    ///
    /// # Panics
    ///
    /// If `id` is not in this tree. Ids come from the tree itself, so this is a
    /// bug in the caller rather than a runtime condition.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    /// The root node.
    #[must_use]
    pub fn root(&self) -> &Node {
        self.node(ROOT)
    }

    /// Every node, in the order they were added: a parent always precedes its
    /// children, which is what makes a single reverse pass enough to roll sizes
    /// up.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        // A tree of four billion nodes is two orders of magnitude past any real
        // volume; the cast cannot lose.
        #[allow(clippy::cast_possible_truncation, reason = "NodeId is u32 and a tree cannot exceed it")]
        self.nodes.iter().enumerate().map(|(index, node)| (index as NodeId, node))
    }

    /// Rebuilds the absolute path of a node by walking up to the root.
    #[must_use]
    pub fn path_of(&self, id: NodeId) -> PathBuf {
        let mut parts = Vec::new();
        let mut at = id;
        while at != ROOT {
            let node = self.node(at);
            parts.push(node.name.as_str());
            at = node.parent;
        }

        let mut path = self.root.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    /// Changes one node in place.
    ///
    /// The one writable door onto a node after it has been pushed, and it exists
    /// for exactly one caller: [`crate::links::count_shared_once`], which has to
    /// move bytes off a second name. Handing out `&mut Node` freely would let a
    /// consumer rewrite a size the roll-up has already folded upward, which is a
    /// tree whose leaves and totals disagree.
    ///
    /// # Panics
    ///
    /// If `id` is not in this tree.
    pub fn mark(&mut self, id: NodeId, change: impl FnOnce(&mut Node)) {
        change(&mut self.nodes[id as usize]);
    }

    /// Appends a child under `parent` and returns its id.
    ///
    /// # Panics
    ///
    /// If the tree already holds `u32::MAX` nodes.
    pub fn push(&mut self, parent: NodeId, node: Node) -> NodeId {
        let id = NodeId::try_from(self.nodes.len()).expect("a tree of more than u32::MAX entries");
        self.nodes.push(node);
        self.nodes[parent as usize].children.push(id);
        id
    }

    /// Rolls every file's size, subtree mtime and entry count up into its
    /// ancestors.
    ///
    /// One reverse pass: because a parent is always pushed before its children,
    /// walking backwards visits every child before its parent, so each node is
    /// complete by the time it is folded into the one above.
    pub fn roll_up(&mut self) {
        for index in (1..self.nodes.len()).rev() {
            let (size, subtree_modified, entries, traits, parent) = {
                let node = &self.nodes[index];
                (node.size, node.subtree_modified, node.entries, node.traits, node.parent as usize)
            };
            let parent = &mut self.nodes[parent];

            parent.size.add(size);
            parent.entries = parent.entries.saturating_add(entries);
            // A folder inherits the fact that something under it is a second
            // name, so a directory reading `0 B` can say why. Only this one
            // trait travels upward: "compressed" or "sparse" are true of a file
            // and meaningless of the folder above it, whereas "there are shared
            // bytes in here" is exactly a statement about the folder.
            if traits.has(Traits::LINKED) || traits.has(Traits::HOLDS_SHARED) {
                parent.traits.insert(Traits::HOLDS_SHARED);
            }
            parent.subtree_modified = match (parent.subtree_modified, subtree_modified) {
                (Some(ours), Some(theirs)) => Some(ours.max(theirs)),
                (ours, theirs) => ours.or(theirs),
            };
        }
    }

    /// The children of `id`, largest first on `basis`.
    ///
    /// A convenience over [`crate::order::children`] for the one order the
    /// product treats as its default. Anything else — a column the reader
    /// picked, a direction they flipped — goes through `order`, which is where
    /// the rules about absence and ties live.
    #[must_use]
    pub fn children_by_size(&self, id: NodeId, basis: SizeBasis) -> Vec<NodeId> {
        let key = match basis {
            SizeBasis::Allocated => crate::order::SortKey::Allocated,
            SizeBasis::Logical => crate::order::SortKey::Logical,
        };
        crate::order::children(
            self,
            id,
            crate::order::Sort {
                key,
                direction: crate::order::Direction::Descending,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn at(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    fn file(name: &str, parent: NodeId, size: Size, modified: Option<SystemTime>) -> Node {
        Node {
            size,
            modified,
            subtree_modified: modified,
            ..Node::new(name.into(), parent, Kind::File)
        }
    }

    fn directory(name: &str, parent: NodeId, modified: Option<SystemTime>) -> Node {
        Node {
            modified,
            subtree_modified: modified,
            ..Node::new(name.into(), parent, Kind::Directory)
        }
    }

    /// Builds: root / { a / { one.bin, two.bin }, b / old.bin }
    fn sample() -> Tree {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));

        let a = tree.push(ROOT, directory("a", ROOT, at(500)));
        tree.push(a, file("one.bin", a, Size { logical: 10, allocated: 4096 }, at(900)));
        tree.push(a, file("two.bin", a, Size { logical: 20, allocated: 4096 }, at(100)));

        // A tool touched `b` today; nothing inside it has changed in years.
        let b = tree.push(ROOT, directory("b", ROOT, at(9_000)));
        tree.push(
            b,
            file(
                "old.bin",
                b,
                Size {
                    logical: 8192,
                    allocated: 8192,
                },
                at(50),
            ),
        );

        tree.roll_up();
        tree
    }

    #[test]
    fn a_folder_costs_what_its_contents_cost_on_disk() {
        let tree = sample();
        let a = tree.node(tree.root().children[0]);

        // Two tiny files: thirty logical bytes, two whole clusters on disk.
        // Reporting 30 here is the lie the product exists to stop telling.
        assert_eq!(a.size, Size { logical: 30, allocated: 8192 });
    }

    #[test]
    fn the_root_sums_the_whole_tree() {
        let tree = sample();
        assert_eq!(
            tree.root().size,
            Size {
                logical: 8222,
                allocated: 16_384
            }
        );
        assert_eq!(tree.root().entries, 6);
    }

    #[test]
    fn the_age_of_a_layer_is_the_newest_thing_in_it() {
        let tree = sample();
        let a = tree.node(tree.root().children[0]);
        assert_eq!(a.subtree_modified, at(900), "a folder is as fresh as its freshest file");
    }

    #[test]
    fn a_folders_own_mtime_is_kept_beside_its_subtree_one() {
        // `b` itself was written at 9000; the only thing in it was written at
        // 50. Both numbers survive the roll-up, which is what lets a later rule
        // say "touched recently, but nothing inside has moved in years".
        let tree = sample();
        let b = tree.node(tree.root().children[1]);
        assert_eq!(b.modified, at(9_000));

        let old = tree.node(b.children[0]);
        assert_eq!(old.modified, at(50));
        assert_eq!(old.subtree_modified, at(50));
    }

    #[test]
    fn a_missing_timestamp_does_not_erase_a_present_one() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        tree.push(ROOT, file("dated.bin", ROOT, Size::flat(1), at(700)));
        tree.push(ROOT, file("undated.bin", ROOT, Size::flat(1), None));
        tree.roll_up();

        assert_eq!(tree.root().subtree_modified, at(700));
    }

    #[test]
    fn a_path_is_rebuilt_from_the_names_on_the_way_up() {
        let tree = sample();
        let a = tree.root().children[0];
        let one = tree.node(a).children[0];
        assert_eq!(tree.path_of(one), PathBuf::from("C:/scan/a/one.bin"));
        assert_eq!(tree.path_of(ROOT), PathBuf::from("C:/scan"));
    }

    #[test]
    fn children_sort_by_what_the_volume_gives_up_by_default() {
        let tree = sample();
        // `b` holds one 8192-byte file; `a` holds two files worth 8192 on disk
        // but only 30 bytes logically. On disk they tie, so the name breaks it;
        // read logically, `b` wins by a mile. The two orders differing is the
        // whole reason both numbers are kept.
        let on_disk = tree.children_by_size(ROOT, SizeBasis::Allocated);
        assert_eq!(tree.node(on_disk[0]).name, "a", "a tie breaks on name, not on scan order");

        let logical = tree.children_by_size(ROOT, SizeBasis::Logical);
        assert_eq!(tree.node(logical[0]).name, "b");
    }

    #[test]
    fn a_volume_root_is_named_after_itself() {
        // A drive root has no file name. An empty first row would be a bug on
        // the first screen anyone sees.
        let tree = Tree::new(PathBuf::from("C:/"), Some(4096));
        assert!(!tree.root().name.is_empty());
    }

    #[test]
    fn what_could_not_be_read_is_kept_rather_than_hidden() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        tree.skip(PathBuf::from("C:/scan/locked"), "access is denied".into());
        assert_eq!(tree.skipped().len(), 1);
    }
}
