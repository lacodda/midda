//! Two pictures of one folder, side by side: what grew, what was freed, and
//! where.
//!
//! A snapshot says what a folder held at a moment ([`crate::snapshot`]); the
//! tree kept current says what it holds now. Set side by side they answer
//! the two questions a single picture cannot: what grew since last time, and
//! — after a cleanup — how much came back and from where.
//!
//! # What the comparison is
//!
//! A tree of its own: every entry whose size is not what it was, with the
//! folders above it, and nothing else. An entry that went is in it, at zero
//! now and at what it held then; an entry that came is in it, at nothing
//! then. A folder in it carries its whole size at both moments — not the sum
//! of the entries shown under it, which leave out everything that stayed the
//! same — so its growth is exact, and is also exactly the sum of the growth
//! shown under it, because what stayed the same grew by nothing.
//!
//! Sizes are compared, not times: a file written again with the same bytes
//! freed nothing and took nothing, and a comparison that listed it would be a
//! list of saves.
//!
//! # Two numbers per entry, and a third
//!
//! Growth is what an entry holds now less what it held then, and can be
//! negative. It does not say how much happened: a folder where one file grew
//! by five gigabytes and another was deleted at five has not grown, and is
//! where everything happened. So every entry also carries how many bytes
//! *moved* under it — grown plus freed — and that is what the picture of a
//! comparison is drawn by: the area of a tile is how much changed there, the
//! colour is which way it went on balance.

use serde::{Deserialize, Serialize};

use crate::order::{self, Sort, SortKey, Span, Value};
use crate::size::{Size, SizeBasis};
use crate::tree::{Node, NodeId, ROOT, Tree, listing_order};
use crate::treemap::{self, Delta, Layout, Tile};

/// How an entry in a comparison came to be in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mark {
    /// It was not there then.
    New,
    /// It is not there now.
    Gone,
    /// It was there both times and is not what it was: a file of another
    /// size, or a folder with something different inside.
    Changed,
    /// A file stands where a folder stood, or a folder where a file did.
    Replaced,
}

/// What a comparison adds up to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    /// What the folder held then.
    pub before: Size,
    /// What it holds now.
    pub now: Size,
    /// Every byte written in between and still there: files that appeared,
    /// files that grew, by how much they grew.
    pub grown: Size,
    /// Every byte that went in between: files deleted, files that shrank, by
    /// how much they shrank. After a cleanup, what came back.
    pub freed: Size,
    /// Entries that appeared, folders and what is in them alike.
    pub new: u64,
    /// Entries that went, folders and what was in them alike.
    pub gone: u64,
    /// Files there both times whose size is not what it was.
    pub changed: u64,
}

/// One place in a comparison: where bytes came or went, counted once.
///
/// A folder that went whole is one place — `node_modules`, gone, 8.9 GB —
/// not two hundred thousand files. Files that changed loose in a folder are
/// that folder's place. Every byte that moved belongs to exactly one place,
/// so the places add up to the totals, which is what lets a report say
/// "from here" without counting anything twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    /// The entry the place is: the folder or file that went or came whole,
    /// or the folder whose own files changed.
    pub id: NodeId,
    pub kind: PlaceKind,
    /// What went from here.
    pub freed: Size,
    /// What came here.
    pub grown: Size,
    /// How many entries came, went or changed here.
    pub entries: u64,
}

/// What kind of place it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlaceKind {
    /// The entry went, with everything in it.
    Gone,
    /// The entry came, with everything in it.
    New,
    /// A folder stands where a file stood, or the other way about.
    Replaced,
    /// Files directly in this folder came, went or changed size.
    Files,
}

/// Two pictures of one folder, compared. See the module documentation.
#[derive(Debug, Clone)]
pub struct Comparison {
    tree: Tree,
    /// What each entry held then, by id. Zero for one that is new.
    before: Vec<Size>,
    /// How many bytes moved under each entry, by id: grown and freed added.
    moved: Vec<Size>,
    marks: Vec<Mark>,
    totals: Totals,
}

impl Comparison {
    /// The entries that differ, with the folders above them. Each one's size
    /// is what it holds now — zero for one that went.
    #[must_use]
    pub const fn tree(&self) -> &Tree {
        &self.tree
    }

    /// What the comparison adds up to.
    #[must_use]
    pub const fn totals(&self) -> Totals {
        self.totals
    }

    /// What `id` held then.
    #[must_use]
    pub fn before(&self, id: NodeId) -> Size {
        self.before[id as usize]
    }

    /// How many bytes moved under `id`: grown and freed, added.
    #[must_use]
    pub fn moved(&self, id: NodeId) -> Size {
        self.moved[id as usize]
    }

    /// How `id` came to be in the comparison.
    #[must_use]
    pub fn mark(&self, id: NodeId) -> Mark {
        self.marks[id as usize]
    }

    /// What `id` holds now less what it held then, on `basis`. Negative for
    /// an entry that shrank or went.
    #[must_use]
    pub fn growth(&self, id: NodeId, basis: SizeBasis) -> i64 {
        signed(basis.of(self.tree.node(id).size), basis.of(self.before(id)))
    }

    /// One page of the children of `id`, ordered by `sort`. Growth is read on
    /// `basis`; every other key is the tree's own, the sizes being what each
    /// entry holds now.
    #[must_use]
    pub fn children_page(&self, id: NodeId, sort: Sort, basis: SizeBasis, span: Span) -> (Vec<NodeId>, usize) {
        let ordered = order::ordered(&self.tree, id, sort.direction, |child| match sort.key {
            SortKey::Growth => Value::Signed(self.growth(child, basis)),
            key => order::value_of(self.tree.node(child), key),
        });
        order::page(ordered, span)
    }

    /// The picture of the children of `id`: each tile as large as what moved
    /// under it, and carrying what it held then and holds now.
    #[must_use]
    pub fn tiles(&self, id: NodeId, layout: Layout) -> Vec<Tile> {
        let basis = layout.basis;
        treemap::weighted(
            &self.tree,
            id,
            layout,
            |child| basis.of(self.moved(child)),
            |child| {
                Some(Delta {
                    before: basis.of(self.before(child)),
                    now: basis.of(self.tree.node(child).size),
                })
            },
        )
    }

    /// Every place bytes came or went, each counted once. See [`Place`].
    ///
    /// In the order of the tree, deepest last; a report sorts them by what it
    /// is asking.
    #[must_use]
    pub fn places(&self) -> Vec<Place> {
        let mut places = Vec::new();
        // The place each folder's loose files are gathered into, by folder.
        let mut loose: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
        let mut stack = vec![ROOT];
        while let Some(id) = stack.pop() {
            let node = self.tree.node(id);
            if id != ROOT {
                let whole = match self.mark(id) {
                    Mark::Gone if node.is_directory() => Some(PlaceKind::Gone),
                    Mark::New if node.is_directory() => Some(PlaceKind::New),
                    Mark::Replaced => Some(PlaceKind::Replaced),
                    _ => None,
                };
                if let Some(kind) = whole {
                    // Everything in it is in this place; nothing under it is
                    // a place of its own. What was replaced went whole, and
                    // what replaced it came whole.
                    let (grown, freed) = match kind {
                        PlaceKind::Replaced => (node.size, self.before(id)),
                        _ => moved_apart(self.before(id), node.size),
                    };
                    places.push(Place {
                        id,
                        kind,
                        freed,
                        grown,
                        entries: node.entries,
                    });
                    continue;
                }
                if !node.is_directory() {
                    let at = *loose.entry(node.parent).or_insert_with(|| {
                        places.push(Place {
                            id: node.parent,
                            kind: PlaceKind::Files,
                            freed: Size::zero(),
                            grown: Size::zero(),
                            entries: 0,
                        });
                        places.len() - 1
                    });
                    let (grown, freed) = moved_apart(self.before(id), node.size);
                    let place = &mut places[at];
                    place.grown.add(grown);
                    place.freed.add(freed);
                    place.entries += 1;
                    continue;
                }
            }
            stack.extend(node.children.iter().rev());
        }
        places
    }
}

/// What a folder or file at two moments came to: the part that grew and the
/// part that went, on both sizes at once.
fn moved_apart(before: Size, now: Size) -> (Size, Size) {
    let apart = |then: u64, now: u64| (now.saturating_sub(then), then.saturating_sub(now));
    let (grown_logical, freed_logical) = apart(before.logical, now.logical);
    let (grown_allocated, freed_allocated) = apart(before.allocated, now.allocated);
    (
        Size {
            logical: grown_logical,
            allocated: grown_allocated,
        },
        Size {
            logical: freed_logical,
            allocated: freed_allocated,
        },
    )
}

/// `now - then`, as far as an `i64` reaches: further is not a disk.
fn signed(now: u64, then: u64) -> i64 {
    let difference = i128::from(now) - i128::from(then);
    i64::try_from(difference).unwrap_or(if difference < 0 { i64::MIN } else { i64::MAX })
}

/// Compares two pictures of one folder: `then`, read earlier, and `now`.
///
/// Entries are matched by name, folder by folder, from the root down — both
/// trees hold each folder's children in [`listing_order`], so each folder is
/// one merge of two sorted lists. A name in only one of them came or went; a
/// name in both is a file whose size may differ or a folder to look inside.
/// A file and a folder of the same name are one entry, replaced.
///
/// Every folder of both trees is looked into once, and only what differs is
/// copied out: comparing two pictures of a system volume a day apart costs a
/// pass over both and a tree the size of the day's changes.
#[must_use]
pub fn compare(then: &Tree, now: &Tree) -> Comparison {
    let mut out = Builder::new(then, now);
    // The pairs of folders being merged, root first. A folder enters the
    // comparison only once something under it is found to differ.
    let mut open = vec![Pair {
        then: ROOT,
        now: ROOT,
        at: Some(ROOT),
        next_then: 0,
        next_now: 0,
    }];

    while let Some(pair) = open.last_mut() {
        let theirs = &then.node(pair.then).children;
        let ours = &now.node(pair.now).children;
        let step = match (theirs.get(pair.next_then), ours.get(pair.next_now)) {
            (None, None) => {
                open.pop();
                continue;
            }
            (Some(&gone), None) => Step::Gone(gone),
            (None, Some(&new)) => Step::New(new),
            (Some(&old), Some(&young)) => match same_name(&then.node(old).name, &now.node(young).name) {
                std::cmp::Ordering::Less => Step::Gone(old),
                std::cmp::Ordering::Greater => Step::New(young),
                std::cmp::Ordering::Equal => Step::Both(old, young),
            },
        };
        match step {
            Step::Gone(_) => pair.next_then += 1,
            Step::New(_) => pair.next_now += 1,
            Step::Both(..) => {
                pair.next_then += 1;
                pair.next_now += 1;
            }
        }

        match step {
            Step::Gone(old) => {
                let parent = out.enter(&mut open);
                out.copy(then, old, parent, Mark::Gone);
            }
            Step::New(young) => {
                let parent = out.enter(&mut open);
                out.copy(now, young, parent, Mark::New);
            }
            Step::Both(old, young) => {
                let (was, is) = (then.node(old), now.node(young));
                match (was.is_directory(), is.is_directory()) {
                    (true, true) => open.push(Pair {
                        then: old,
                        now: young,
                        at: None,
                        next_then: 0,
                        next_now: 0,
                    }),
                    (false, false) => {
                        if was.size != is.size {
                            let parent = out.enter(&mut open);
                            out.add(parent, is, is.size, was.size, Mark::Changed);
                        }
                    }
                    _ => {
                        let parent = out.enter(&mut open);
                        out.replace(old, young, parent);
                    }
                }
            }
        }
    }

    out.finish()
}

/// Two names in [`listing_order`], with the common case — the same name in
/// both pictures — answered without folding a character.
fn same_name(a: &str, b: &str) -> std::cmp::Ordering {
    if a == b { std::cmp::Ordering::Equal } else { listing_order(a, b) }
}

/// A folder of `then` and the folder of the same name in `now`, being
/// merged.
struct Pair {
    then: NodeId,
    now: NodeId,
    /// Its id in the comparison, once something under it differs.
    at: Option<NodeId>,
    /// The next child of each not yet looked at.
    next_then: usize,
    next_now: usize,
}

/// What the next pair of children is.
#[derive(Clone, Copy)]
enum Step {
    Gone(NodeId),
    New(NodeId),
    Both(NodeId, NodeId),
}

/// The comparison being built, and the columns beside its tree.
struct Builder<'a> {
    then: &'a Tree,
    now: &'a Tree,
    tree: Tree,
    before: Vec<Size>,
    marks: Vec<Mark>,
    /// For every folder a file now stands in place of, how many entries it
    /// held: the comparison cannot show them — a file has nothing under it —
    /// and the count of what went still includes them.
    replaced_folders: Vec<u64>,
}

impl<'a> Builder<'a> {
    fn new(then: &'a Tree, now: &'a Tree) -> Self {
        let mut tree = Tree::new(now.root_path().to_path_buf(), now.cluster_bytes());
        let root = now.root();
        tree.stamp_root(root.modified, root.identity);
        tree.mark(ROOT, |held| {
            held.size = root.size;
            held.subtree_modified = root.subtree_modified;
            held.traits = root.traits;
        });
        Self {
            then,
            now,
            tree,
            before: vec![then.root().size],
            marks: vec![Mark::Changed],
            replaced_folders: Vec::new(),
        }
    }

    /// The id in the comparison of the innermost folder being merged,
    /// putting it — and every folder above it not yet there — into the
    /// comparison first.
    fn enter(&mut self, pairs: &mut [Pair]) -> NodeId {
        let from = pairs.iter().rposition(|pair| pair.at.is_some()).unwrap_or(0);
        for index in from + 1..pairs.len() {
            let parent = pairs[index - 1].at.unwrap_or(ROOT);
            let (now, then) = (self.now.node(pairs[index].now), self.then.node(pairs[index].then));
            let id = self.add(parent, now, now.size, then.size, Mark::Changed);
            pairs[index].at = Some(id);
        }
        pairs.last().and_then(|pair| pair.at).unwrap_or(ROOT)
    }

    /// Puts an entry under `parent` as `node` describes it, holding `size`
    /// now and `before` then.
    fn add(&mut self, parent: NodeId, node: &Node, size: Size, before: Size, mark: Mark) -> NodeId {
        let id = self.tree.push(
            parent,
            Node {
                size,
                modified: node.modified,
                subtree_modified: node.subtree_modified,
                traits: node.traits,
                links: node.links,
                identity: node.identity,
                ..Node::new(node.name.clone(), parent, node.kind)
            },
        );
        self.before.push(before);
        self.marks.push(mark);
        id
    }

    /// Copies `id` of `from`, and everything under it, under `parent` — as
    /// gone, from the picture of then, or as new, from the picture of now.
    fn copy(&mut self, from: &Tree, id: NodeId, parent: NodeId, mark: Mark) {
        let mut stack = vec![(id, parent)];
        while let Some((at, under)) = stack.pop() {
            let node = from.node(at);
            let copied = if mark == Mark::Gone {
                self.add(under, node, Size::zero(), node.size, mark)
            } else {
                self.add(under, node, node.size, Size::zero(), mark)
            };
            // Pushed in reverse so they come off in order, and each folder's
            // children land in the listing order they were held in.
            stack.extend(node.children.iter().rev().map(|&child| (child, copied)));
        }
    }

    /// A file where a folder was, or a folder where a file was.
    fn replace(&mut self, old: NodeId, young: NodeId, parent: NodeId) {
        let (then, now) = (self.then, self.now);
        let (was, is) = (then.node(old), now.node(young));
        let id = self.add(parent, is, is.size, was.size, Mark::Replaced);
        if was.is_directory() {
            self.replaced_folders.push(was.entries);
        }
        for &child in &is.children {
            self.copy(now, child, id, Mark::New);
        }
    }

    /// Works out what moved under every entry, how many entries each holds,
    /// and the totals.
    fn finish(self) -> Comparison {
        let Self {
            mut tree,
            before,
            marks,
            replaced_folders,
            ..
        } = self;
        let count = before.len();
        let mut moved = vec![Size::zero(); count];
        let mut entries = vec![1_u64; count];
        let mut totals = Totals {
            before: before[ROOT as usize],
            now: tree.root().size,
            ..Totals::default()
        };
        // A folder a file replaced went with all it held; the entry itself
        // is counted with the rest below.
        totals.gone = replaced_folders.iter().map(|held| held.saturating_sub(1)).sum();

        // A parent is always pushed before its children, so walking the
        // arena backwards finishes every child before its parent.
        for index in (0..count).rev() {
            let node = tree.node(NodeId::try_from(index).expect("fewer entries than u32::MAX"));
            let (then, is) = (before[index], node.size);
            let own = match (marks[index], node.is_directory()) {
                (Mark::Changed | Mark::New | Mark::Gone, false) => Some(moved_apart(then, is)),
                // A folder where a file was: the file went; what is in the
                // folder came, and is counted where it is.
                (Mark::Replaced, true) => Some((Size::zero(), then)),
                (Mark::Replaced, false) => Some((is, then)),
                (Mark::Changed | Mark::New | Mark::Gone, true) => None,
            };
            if let Some((grown, freed)) = own {
                moved[index].add(grown);
                moved[index].add(freed);
                totals.grown.add(grown);
                totals.freed.add(freed);
            }
            if index != ROOT as usize {
                match (marks[index], node.is_directory()) {
                    (Mark::New, _) => totals.new += 1,
                    (Mark::Gone, _) => totals.gone += 1,
                    (Mark::Changed, false) => totals.changed += 1,
                    (Mark::Replaced, _) => {
                        totals.new += 1;
                        totals.gone += 1;
                    }
                    (Mark::Changed, true) => {}
                }
                let parent = node.parent as usize;
                let (here, held) = (moved[index], entries[index]);
                moved[parent].add(here);
                entries[parent] = entries[parent].saturating_add(held);
            }
        }
        for (index, &held) in entries.iter().enumerate() {
            tree.mark(NodeId::try_from(index).expect("fewer entries than u32::MAX"), |node| node.entries = held);
        }

        Comparison {
            tree,
            before,
            moved,
            marks,
            totals,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tree::Kind;

    fn file(name: &str, parent: NodeId, bytes: u64) -> Node {
        Node {
            size: Size::flat(bytes),
            ..Node::new(name.into(), parent, Kind::File)
        }
    }

    fn folder(name: &str, parent: NodeId) -> Node {
        Node::new(name.into(), parent, Kind::Directory)
    }

    /// Builds a tree from `(path, bytes)` pairs, `None` for a folder. Pushed
    /// level by level in listing order, as a scan pushes them.
    fn tree(entries: &[(&str, Option<u64>)]) -> Tree {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let mut sorted: Vec<_> = entries.to_vec();
        sorted.sort_by(|a, b| {
            let (x, y) = (a.0.split('/').collect::<Vec<_>>(), b.0.split('/').collect::<Vec<_>>());
            x.len().cmp(&y.len()).then_with(|| {
                x.iter()
                    .zip(&y)
                    .map(|(p, q)| listing_order(p, q))
                    .find(|order| order.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        for (path, bytes) in sorted {
            let mut parts: Vec<&str> = path.split('/').collect();
            let name = parts.pop().expect("a name");
            let parent = parts.iter().fold(ROOT, |at, part| tree.child(at, part).expect("the folder comes first"));
            match bytes {
                Some(bytes) => tree.push(parent, file(name, parent, bytes)),
                None => tree.push(parent, folder(name, parent)),
            };
        }
        tree.roll_up();
        tree
    }

    fn find(comparison: &Comparison, path: &str) -> Option<NodeId> {
        path.split('/').try_fold(ROOT, |at, name| comparison.tree().child(at, name))
    }

    fn names(comparison: &Comparison, id: NodeId) -> Vec<String> {
        comparison
            .tree()
            .node(id)
            .children
            .iter()
            .map(|&child| comparison.tree().node(child).name.clone())
            .collect()
    }

    const A: SizeBasis = SizeBasis::Allocated;

    /// A developer's folder on two days: a dependency upgrade, a build, a
    /// download, a cleanup.
    fn days() -> (Tree, Tree) {
        let then = tree(&[
            ("app", None),
            ("app/node_modules", None),
            ("app/node_modules/react.js", Some(300)),
            ("app/node_modules/old.js", Some(200)),
            ("app/src", None),
            ("app/src/main.ts", Some(10)),
            ("app/src/util.ts", Some(5)),
            ("cache", None),
            ("cache/blob.bin", Some(1_000)),
            ("cache/index", Some(50)),
            ("notes.txt", Some(7)),
            ("site", None),
            ("site/page.html", Some(40)),
        ]);
        let now = tree(&[
            ("app", None),
            ("app/node_modules", None),
            ("app/node_modules/react.js", Some(450)),
            ("app/node_modules/vite.js", Some(600)),
            ("app/src", None),
            ("app/src/main.ts", Some(10)),
            ("app/src/util.ts", Some(5)),
            ("downloads", None),
            ("downloads/big.iso", Some(5_000)),
            ("notes.txt", Some(7)),
            ("site", None),
            ("site/page.html", Some(40)),
        ]);
        (then, now)
    }

    #[test]
    fn only_what_differs_is_in_the_comparison_with_the_folders_above_it() {
        let (then, now) = days();
        let comparison = compare(&then, &now);

        assert_eq!(names(&comparison, ROOT), ["app", "cache", "downloads"], "notes.txt and site did not change");
        let app = find(&comparison, "app").expect("app holds a change");
        assert_eq!(names(&comparison, app), ["node_modules"], "src did not change");
        let modules = find(&comparison, "app/node_modules").expect("the dependencies");
        assert_eq!(names(&comparison, modules), ["old.js", "react.js", "vite.js"]);
        assert_eq!(comparison.mark(find(&comparison, "app/node_modules/old.js").expect("old")), Mark::Gone);
        assert_eq!(comparison.mark(find(&comparison, "app/node_modules/react.js").expect("react")), Mark::Changed);
        assert_eq!(comparison.mark(find(&comparison, "app/node_modules/vite.js").expect("vite")), Mark::New);
        assert_eq!(comparison.mark(find(&comparison, "cache").expect("cache")), Mark::Gone);
        assert_eq!(
            names(&comparison, find(&comparison, "cache").expect("cache")),
            ["blob.bin", "index"],
            "what went with it is there to look into"
        );
        assert_eq!(comparison.mark(find(&comparison, "downloads/big.iso").expect("the download")), Mark::New);
    }

    #[test]
    fn a_folder_carries_its_whole_size_at_both_moments_not_the_part_shown() {
        let (then, now) = days();
        let comparison = compare(&then, &now);
        let app = find(&comparison, "app").expect("app");

        // src (15 bytes) is not shown under app, and is still in its size.
        assert_eq!(comparison.before(app), Size::flat(515));
        assert_eq!(comparison.tree().node(app).size, Size::flat(1_065));
        assert_eq!(comparison.growth(app, A), 550);
    }

    #[test]
    fn growth_adds_up_from_the_entries_shown_to_every_folder_above_them() {
        // The property the view rests on: a folder's growth is the sum of
        // the growth of what is shown in it, because what is not shown grew
        // by nothing.
        let (then, now) = days();
        let comparison = compare(&then, &now);
        let tree = comparison.tree();
        for (id, node) in tree.nodes() {
            if node.is_directory() && !node.children.is_empty() {
                let below: i64 = node.children.iter().map(|&child| comparison.growth(child, A)).sum();
                assert_eq!(comparison.growth(id, A), below, "growth under {}", tree.path_of(id).display());
            }
        }
        assert_eq!(comparison.growth(ROOT, A), 5_000 + 150 + 600 - 200 - 1_050);
    }

    #[test]
    fn the_totals_split_what_grew_from_what_went() {
        let (then, now) = days();
        let totals = compare(&then, &now).totals();
        assert_eq!(totals.before, then.root().size);
        assert_eq!(totals.now, now.root().size);
        assert_eq!(totals.grown, Size::flat(5_000 + 600 + 150));
        assert_eq!(totals.freed, Size::flat(200 + 1_050));
        assert_eq!(
            (totals.new, totals.gone, totals.changed),
            (3, 4, 1),
            "vite.js, downloads and big.iso; old.js, cache and its two; react.js"
        );
        // Net is what the folder holds now less what it held: both ways of
        // counting it agree.
        assert_eq!(
            i128::from(totals.now.allocated) - i128::from(totals.before.allocated),
            i128::from(totals.grown.allocated) - i128::from(totals.freed.allocated)
        );
    }

    #[test]
    fn what_moved_is_counted_both_ways_and_draws_the_picture() {
        let (then, now) = days();
        let comparison = compare(&then, &now);
        let modules = find(&comparison, "app/node_modules").expect("the dependencies");
        // +150 react, +600 vite, -200 old: grew 550 on balance, 950 moved.
        assert_eq!(comparison.growth(modules, A), 550);
        assert_eq!(comparison.moved(modules), Size::flat(950));
        assert_eq!(comparison.moved(ROOT), Size::flat(5_000 + 950 + 1_050));

        let tiles = comparison.tiles(ROOT, Layout::default());
        assert_eq!(tiles.iter().map(|tile| tile.name.as_str()).collect::<Vec<_>>(), ["downloads", "cache", "app"]);
        assert_eq!(tiles[1].bytes, 1_050, "a tile is as large as what moved under it");
        assert_eq!(tiles[1].change, Some(Delta { before: 1_050, now: 0 }));
    }

    #[test]
    fn a_folder_where_as_much_went_as_came_is_where_everything_happened() {
        let then = tree(&[("logs", None), ("logs/a.log", Some(500))]);
        let now = tree(&[("logs", None), ("logs/b.log", Some(500))]);
        let comparison = compare(&then, &now);
        let logs = find(&comparison, "logs").expect("logs changed inside");
        assert_eq!(comparison.growth(logs, A), 0);
        assert_eq!(comparison.moved(logs), Size::flat(1_000));
        assert_eq!(comparison.tiles(ROOT, Layout::default()).len(), 1, "drawn by what moved, not by what it netted");
    }

    #[test]
    fn two_pictures_of_the_same_disk_compare_as_nothing() {
        let (then, _) = days();
        let comparison = compare(&then, &then.clone());
        assert!(comparison.tree().is_empty());
        assert_eq!(comparison.totals().grown, Size::zero());
        assert_eq!(comparison.totals().freed, Size::zero());
        assert!(comparison.places().is_empty());
    }

    #[test]
    fn a_file_written_again_at_the_same_size_is_not_a_change() {
        let then = tree(&[("same.txt", Some(10))]);
        let mut now = then.clone();
        let id = now.child(ROOT, "same.txt").expect("the file");
        now.mark(id, |node| node.modified = Some(std::time::SystemTime::now()));
        assert!(compare(&then, &now).tree().is_empty());
    }

    #[test]
    fn names_are_matched_exactly_so_a_change_of_case_is_a_rename() {
        let then = tree(&[("Readme.md", Some(10))]);
        let now = tree(&[("README.md", Some(10))]);
        let comparison = compare(&then, &now);
        assert_eq!(names(&comparison, ROOT), ["README.md", "Readme.md"]);
        assert_eq!(comparison.totals().new, 1);
        assert_eq!(comparison.totals().gone, 1);
    }

    #[test]
    fn a_folder_where_a_file_stood_is_one_entry_replaced() {
        let then = tree(&[("build", Some(70)), ("keep.txt", Some(1))]);
        let now = tree(&[("build", None), ("build/out.bin", Some(400)), ("keep.txt", Some(1))]);
        let comparison = compare(&then, &now);
        assert_eq!(names(&comparison, ROOT), ["build"], "one entry, not a file and a folder of one name");
        let build = find(&comparison, "build").expect("build");
        assert_eq!(comparison.mark(build), Mark::Replaced);
        assert_eq!(comparison.mark(find(&comparison, "build/out.bin").expect("out")), Mark::New);
        let totals = comparison.totals();
        assert_eq!((totals.grown, totals.freed), (Size::flat(400), Size::flat(70)));
        assert_eq!(comparison.moved(ROOT), Size::flat(470));
    }

    #[test]
    fn a_file_where_a_folder_stood_counts_what_the_folder_held_as_gone() {
        let then = tree(&[("data", None), ("data/a", Some(10)), ("data/b", Some(20))]);
        let now = tree(&[("data", Some(5))]);
        let comparison = compare(&then, &now);
        let data = find(&comparison, "data").expect("data");
        assert_eq!(comparison.mark(data), Mark::Replaced);
        let totals = comparison.totals();
        assert_eq!((totals.grown, totals.freed), (Size::flat(5), Size::flat(30)));
        assert_eq!((totals.new, totals.gone), (1, 3), "the file came; the folder and its two went");
    }

    #[test]
    fn the_places_add_up_to_the_totals_and_name_a_folder_that_went_once() {
        let (then, now) = days();
        let comparison = compare(&then, &now);
        let places = comparison.places();
        let path = |place: &Place| comparison.tree().path_of(place.id).to_string_lossy().replace('\\', "/");

        let cache = places.iter().find(|place| place.kind == PlaceKind::Gone).expect("the cache went");
        assert_eq!(path(cache), "C:/scan/cache");
        assert_eq!((cache.freed, cache.entries), (Size::flat(1_050), 3), "one place for the folder and both files");

        let downloads = places.iter().find(|place| place.kind == PlaceKind::New).expect("downloads came");
        assert_eq!(path(downloads), "C:/scan/downloads");

        let loose = places.iter().find(|place| place.kind == PlaceKind::Files).expect("files in node_modules");
        assert_eq!(path(loose), "C:/scan/app/node_modules");
        assert_eq!((loose.grown, loose.freed, loose.entries), (Size::flat(750), Size::flat(200), 3));

        let totals = comparison.totals();
        assert_eq!(places.iter().map(|place| place.freed).sum::<Size>(), totals.freed);
        assert_eq!(places.iter().map(|place| place.grown).sum::<Size>(), totals.grown);
    }

    #[test]
    fn growth_orders_the_rows_and_the_unchanged_are_not_rows_at_all() {
        let (then, now) = days();
        let comparison = compare(&then, &now);
        let sort = Sort {
            key: SortKey::Growth,
            direction: crate::order::Direction::Descending,
        };
        let (page, total) = comparison.children_page(ROOT, sort, A, Span::default());
        let order: Vec<_> = page.iter().map(|&id| comparison.tree().node(id).name.as_str()).collect();
        assert_eq!(total, 3);
        assert_eq!(order, ["downloads", "app", "cache"], "the most grown first, the most freed last");
    }

    #[test]
    fn an_entry_counts_what_changed_under_it() {
        let (then, now) = days();
        let comparison = compare(&then, &now);
        assert_eq!(
            comparison.tree().node(find(&comparison, "app").expect("app")).entries,
            5,
            "app, node_modules and its three"
        );
        assert_eq!(comparison.tree().root().entries, comparison.tree().len() as u64);
    }
}
