//! Keeping a tree true to the volume after it was read.
//!
//! A scan is a picture of a moment. midda keeps the picture current three
//! ways, as far as the process's rights allow (ADR 0009):
//!
//! - without elevation, a watcher on the scanned folder reports what changes
//!   while the window is open ([`watch`], Windows);
//! - with elevation, the volume's change journal does the same and also says
//!   what changed while midda was closed ([`journal`], Windows);
//! - between runs the tree is saved, and opened again before any of the above
//!   has said a word ([`store`]).
//!
//! However the news arrives, it is applied one way. The news is a list of
//! paths that changed — never trusted for what it says changed. Each path is
//! looked at again on the volume, and the tree is made to match what is there
//! now: a file measured again, a folder that appeared read whole, a folder
//! that went taken out. A watcher that reported a rename as two events, a
//! journal that wrote six records for one save, a case-only rename, a name
//! given in its DOS short form — all of them come down to "look here", and the
//! volume answers.
//!
//! The looking and the changing are two steps, [`Index::observe`] and
//! [`Index::apply`], because the first reads the disk and the second holds the
//! tree: a window reading the tree waits only for the second.

#[cfg(windows)]
pub mod journal;
pub mod since;
pub mod store;
pub mod usn;
#[cfg(windows)]
pub mod watch;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::links::Groups;
use crate::platform::{self, FileIdentity, Presence};
use crate::scanner::Progress;
use crate::tree::{Node, NodeId, ROOT, Tree};
use crate::walk;

/// A scanned tree, and what it takes to keep it current.
#[derive(Debug, Clone)]
pub struct Index {
    tree: Tree,
    /// The names of every file with more than one, so a change to one of them
    /// can be settled without a pass over the whole tree.
    groups: Groups,
    /// When the tree was last read from the volume in full.
    scanned_at: SystemTime,
    /// Where in the volume's change journal the tree is up to, when it is
    /// following one.
    journal: Option<JournalPosition>,
}

/// A place in a volume's change journal.
///
/// The journal's own id is part of it: a journal deleted and created again
/// starts its numbering over, and a position in the old one means nothing in
/// the new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalPosition {
    /// Which journal.
    pub journal: u64,
    /// The first update not yet applied.
    pub next: i64,
}

impl Index {
    /// An index over a tree just read in full.
    #[must_use]
    pub fn new(tree: Tree, scanned_at: SystemTime) -> Self {
        Self {
            groups: Groups::of(&tree),
            tree,
            scanned_at,
            journal: None,
        }
    }

    /// The tree as it stands.
    #[must_use]
    pub const fn tree(&self) -> &Tree {
        &self.tree
    }

    /// When the tree was last read from the volume in full.
    #[must_use]
    pub const fn scanned_at(&self) -> SystemTime {
        self.scanned_at
    }

    /// Where in the change journal the tree is up to, when it follows one.
    #[must_use]
    pub const fn journal(&self) -> Option<JournalPosition> {
        self.journal
    }

    /// Records that the tree is up to date with the journal as far as `at`.
    pub const fn follow(&mut self, at: Option<JournalPosition>) {
        self.journal = at;
    }

    /// Lays the tree out as a fresh scan would. Every id changes. See
    /// [`Tree::compact`].
    pub fn compact(&mut self) {
        self.tree.compact();
        self.groups = Groups::of(&self.tree);
    }

    /// Looks at each of `paths` on the volume again, and says what the tree
    /// should become there. Reads the disk; changes nothing.
    ///
    /// A path outside the scanned folder is passed over: a journal reports a
    /// whole volume. A path under a folder the tree does not hold is looked at
    /// from the deepest folder it does — that folder appeared, and is read
    /// whole.
    #[must_use]
    pub fn observe(&self, paths: impl IntoIterator<Item = PathBuf>) -> Observed {
        let tree = &self.tree;
        let mut points: Vec<(NodeId, String)> = Vec::new();
        let mut seen_points = HashSet::new();
        let mut parents = Vec::new();
        let mut seen_parents = HashSet::new();

        let mut add_point = |parent: NodeId, name: String, points: &mut Vec<(NodeId, String)>| {
            if seen_points.insert((parent, name.clone())) {
                points.push((parent, name));
            }
        };

        for path in paths {
            match point_of(tree, &path) {
                None => {}
                Some((parent, None)) => {
                    if seen_parents.insert(parent) {
                        parents.push(parent);
                    }
                }
                Some((parent, Some(name))) => add_point(parent, name, &mut points),
            }
        }

        // A file's other names change with it — its link count, its bytes —
        // and nothing reports them: the change happened under one name.
        for (parent, name) in points.clone() {
            if let Some(id) = tree.child(parent, &name) {
                for &other in self.groups.names_of(tree, id) {
                    if other != id {
                        let node = tree.node(other);
                        add_point(node.parent, node.name.clone(), &mut points);
                    }
                }
            }
        }

        let cluster = tree.cluster_bytes().unwrap_or(platform::ASSUMED_CLUSTER_BYTES);
        let mut found = Vec::new();
        for (parent, name) in points {
            if seen_parents.insert(parent) {
                parents.push(parent);
            }
            look(tree, parent, &name, cluster, &mut found);
        }
        // A folder's own write time moves when an entry in it is added,
        // removed or renamed. Read after the entries, so it is no older than
        // they are.
        for id in parents {
            let own = own_of(tree, id);
            found.push(Seen::Own {
                id,
                modified: own.modified,
                identity: own.identity,
            });
        }
        Observed { found }
    }

    /// Makes the tree what [`Index::observe`] found, and brings every total
    /// above a change up to date.
    pub fn apply(&mut self, observed: Observed) -> Applied {
        let mut applied = Applied::default();
        let mut touched = Vec::new();
        let mut files = HashSet::new();

        for seen in observed.found {
            match seen {
                Seen::Gone { parent, name } => {
                    if self.tree.contains(parent)
                        && let Some(id) = self.tree.child(parent, &name)
                    {
                        applied.removed += self.tree.node(id).entries;
                        self.take_out(id, &mut files);
                        touched.push(parent);
                    }
                }
                Seen::Entry { parent, node } => {
                    if !self.tree.contains(parent) {
                        continue;
                    }
                    let id = match self.tree.child(parent, &node.name) {
                        Some(id) if !self.tree.node(id).is_directory() && measured_alike(self.tree.node(id), &node) => continue,
                        Some(id) if !self.tree.node(id).is_directory() => {
                            files.extend(self.groups.forget(&self.tree, id));
                            self.tree.mark(id, |held| {
                                held.size = node.size;
                                held.modified = node.modified;
                                held.subtree_modified = node.modified;
                                held.traits = node.traits;
                                held.links = node.links;
                                held.identity = node.identity;
                            });
                            applied.updated += 1;
                            id
                        }
                        held => {
                            if let Some(directory) = held {
                                applied.removed += self.tree.node(directory).entries;
                                self.take_out(directory, &mut files);
                            }
                            applied.added += 1;
                            self.tree.insert(parent, node)
                        }
                    };
                    files.extend(self.groups.learn(&self.tree, id));
                    touched.push(id);
                }
                Seen::Branch { parent, mut branch } => {
                    if !self.tree.contains(parent) {
                        continue;
                    }
                    if let Some(held) = self.tree.child(parent, &branch.root().name) {
                        applied.removed += self.tree.node(held).entries;
                        self.take_out(held, &mut files);
                    }
                    branch.roll_up();
                    applied.added += branch.root().entries;
                    let id = self.tree.graft(parent, branch);
                    let mut stack = vec![id];
                    while let Some(at) = stack.pop() {
                        stack.extend_from_slice(&self.tree.node(at).children);
                        files.extend(self.groups.learn(&self.tree, at));
                    }
                    touched.push(id);
                }
                Seen::Own { id, modified, identity } => {
                    if self.tree.contains(id) {
                        self.tree.mark(id, |held| {
                            held.modified = modified;
                            held.identity = identity;
                        });
                        touched.push(id);
                    }
                }
            }
        }

        touched.extend(self.groups.settle(&mut self.tree, files));
        self.tree.refresh(touched);
        self.tree.record_shared(self.groups.deduplicated());
        applied
    }

    /// Takes `id` and what is under it out of the tree, letting go of every
    /// shared name in it first.
    fn take_out(&mut self, id: NodeId, files: &mut HashSet<FileIdentity>) {
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            stack.extend_from_slice(&self.tree.node(at).children);
            files.extend(self.groups.forget(&self.tree, at));
        }
        self.tree.detach(id);
    }
}

/// Whether a file measured again is what the tree already holds.
///
/// The size is compared as measured: a second name holds zero in the tree
/// and the file's bytes when measured, so a name that is shared is never
/// "alike" and is always settled again — which is also what a change to its
/// link count needs.
fn measured_alike(held: &Node, measured: &Node) -> bool {
    held.size == measured.size
        && held.modified == measured.modified
        && held.traits == measured.traits
        && held.links == measured.links
        && held.identity == measured.identity
}

/// What looking at the changed paths found, waiting to be applied.
#[derive(Debug, Default)]
pub struct Observed {
    found: Vec<Seen>,
}

impl Observed {
    /// Whether nothing was found that would change the tree.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.found.is_empty()
    }
}

/// How much one application changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    /// Entries that appeared, counting everything inside a folder that did.
    pub added: u64,
    /// Entries that went, counting everything inside a folder that did.
    pub removed: u64,
    /// Files measured again where they were.
    pub updated: u64,
}

impl Applied {
    /// Whether the tree changed at all, beyond a folder's write time.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.added + self.removed + self.updated > 0
    }
}

/// One thing the volume said when it was looked at again.
#[derive(Debug)]
enum Seen {
    /// Nothing called `name` is in `parent` any more.
    Gone { parent: NodeId, name: String },
    /// A file or a link, measured.
    Entry { parent: NodeId, node: Node },
    /// A folder the tree does not hold, read whole.
    Branch { parent: NodeId, branch: Tree },
    /// A folder the tree holds, which is still the same folder.
    Own {
        id: NodeId,
        modified: Option<SystemTime>,
        identity: Option<FileIdentity>,
    },
}

/// Where in the tree a changed path is to be looked at: the folder that
/// holds it, and its name there — or, for the scan root itself, the root and
/// no name.
///
/// The deepest folder the tree holds on the way: a path under a folder the
/// tree does not have is a change to that folder's existence, and is looked
/// at as such.
fn point_of(tree: &Tree, path: &Path) -> Option<(NodeId, Option<String>)> {
    let names = below(tree.root_path(), path)?;
    let mut at = ROOT;
    let mut on_disk = tree.root_path().to_path_buf();
    for (index, name) in names.iter().enumerate() {
        if index + 1 == names.len() {
            return Some((at, Some(name.clone())));
        }
        let child = tree.child(at, name).or_else(|| {
            // The volume may know the folder by another spelling: another
            // case, or its DOS short name.
            match platform::presence(&on_disk.join(name)) {
                Presence::Here(real) if real != *name => tree.child(at, &real),
                _ => None,
            }
        });
        match child {
            Some(child) if tree.node(child).is_directory() => {
                on_disk.push(&tree.node(child).name);
                at = child;
            }
            _ => return Some((at, Some(name.clone()))),
        }
    }
    Some((ROOT, None))
}

/// The names leading from `root` down to `path`, when `path` is under it.
///
/// Compared component by component — a path is not a string, and `C:/scan`
/// and `C:\scan` are one folder — and, on Windows, regardless of case, since
/// that is how the volume compares them.
fn below(root: &Path, path: &Path) -> Option<Vec<String>> {
    let mut path = path.components();
    for part in root.components() {
        if !same_component(part, path.next()?) {
            return None;
        }
    }
    path.map(|component| match component {
        Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        _ => None,
    })
    .collect()
}

fn same_component(a: Component<'_>, b: Component<'_>) -> bool {
    if cfg!(windows) {
        let fold = |component: Component<'_>| component.as_os_str().to_string_lossy().chars().flat_map(char::to_uppercase).collect::<String>();
        fold(a) == fold(b)
    } else {
        a == b
    }
}

/// Looks at the entry called `name` in `parent` on the volume.
fn look(tree: &Tree, parent: NodeId, name: &str, cluster: u64, found: &mut Vec<Seen>) {
    let folder = tree.path_of(parent);
    let real = match platform::presence(&folder.join(name)) {
        Presence::Unknown => return,
        Presence::Gone => {
            found.push(Seen::Gone { parent, name: name.to_owned() });
            return;
        }
        Presence::Here(real) => real,
    };
    if real != name {
        // Asked under one spelling, held under another: the old spelling goes
        // and the volume's own takes its place.
        found.push(Seen::Gone { parent, name: name.to_owned() });
    }

    let path = folder.join(&real);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            found.push(Seen::Gone { parent, name: real });
            return;
        }
        Err(_) => return,
    };

    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        if let Some(node) = walk::measure_entry(&path, parent, cluster) {
            found.push(Seen::Entry { parent, node });
        }
        return;
    }

    let own = platform::own(&path);
    let modified = own.modified.or_else(|| metadata.modified().ok());
    let held = tree.child(parent, &real).filter(|&id| tree.node(id).is_directory());
    // The same folder when the volume says it is the same, or when either
    // side cannot say: reading a large folder again for every change in it
    // would be the price of a question nobody could answer.
    let same = held.filter(|&id| match (tree.node(id).identity, own.identity) {
        (Some(ours), Some(theirs)) => ours == theirs,
        _ => true,
    });
    if let Some(id) = same {
        found.push(Seen::Own {
            id,
            modified,
            identity: own.identity,
        });
        return;
    }

    // A folder the tree does not hold, or one that took the place of another
    // of the same name: read whole, as a scan would have read it.
    let mut branch = walk::read(&path, &Progress::default()).unwrap_or_else(|error| {
        let mut unread = Tree::new(path.clone(), tree.cluster_bytes());
        unread.skip(path.clone(), error.to_string());
        unread
    });
    // Its own facts as its listing gives them, which is how a scan reads a
    // folder inside the one it was pointed at.
    branch.stamp_root(modified, own.identity);
    found.push(Seen::Branch { parent, branch });
}

/// What a folder in the tree says about itself, read the way the scan that
/// built the tree read it: the root from its metadata, every other folder
/// from its own entry.
fn own_of(tree: &Tree, id: NodeId) -> platform::Own {
    let path = tree.path_of(id);
    let own = platform::own(&path);
    if id == ROOT {
        platform::Own {
            modified: std::fs::metadata(&path).ok().and_then(|metadata| metadata.modified().ok()),
            identity: own.identity,
        }
    } else {
        own
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_found_under_the_root_whatever_its_separators() {
        let names = below(Path::new("C:/scan"), Path::new("C:/scan/a/b.txt")).expect("under the root");
        assert_eq!(names, ["a", "b.txt"]);
        assert_eq!(below(Path::new("C:/scan"), Path::new("C:/scan")), Some(Vec::new()));
        assert_eq!(below(Path::new("C:/scan"), Path::new("C:/elsewhere/a")), None);
        assert_eq!(below(Path::new("C:/scan/a"), Path::new("C:/scan")), None, "above the root is outside it");
        assert_eq!(
            below(Path::new("C:/scan"), Path::new("C:/scan/../x")),
            None,
            "a path that climbs is not followed"
        );
    }

    #[test]
    #[cfg(windows)]
    fn a_path_is_found_under_the_root_whatever_its_case_on_windows() {
        assert_eq!(below(Path::new(r"C:\Scan"), Path::new(r"c:\SCAN\Inner")), Some(vec!["Inner".to_owned()]));
    }
}
