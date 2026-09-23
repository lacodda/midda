//! Counting shared bytes once.
//!
//! A hard link is not a pointer to a file; it is a *name* for one, and a file
//! with three names is three directory entries over one set of bytes. Deleting
//! one name frees nothing. A scanner that measures each name and adds them up
//! reports three times the truth, and it does so invisibly: every individual
//! number it printed was correct.
//!
//! Measured on this volume, 2026-09-20: one 50 000-byte file under three names,
//! each reporting 53 248 bytes allocated. A naive total says 159 744 for
//! 53 248 bytes of disk.
//!
//! # Which name keeps the bytes
//!
//! One of them has to. The alternative — charging the bytes to nobody — keeps
//! every folder's number defensible in isolation and breaks the one promise
//! that matters: that the folders under a volume add up to the volume. So the
//! bytes stay on one name, and the other names are marked as what they are.
//!
//! The owner is the **first name in node order**, which is not an arbitrary
//! choice dressed up as a rule. The walk reads the tree in levels, breadth
//! first, so a lower node id means a shallower name, and within one level it
//! means earlier in [`crate::tree::listing_order`]. That makes the owner
//! statable in words — *the shallowest name, and among equals the first by
//! name* — and identical across two scans of the same disk, by either scanner.
//!
//! "First seen during the walk" was the obvious cheap answer and it is wrong:
//! the walk runs on a work-stealing pool, so which name a thread reaches first
//! is a race. Two scans of one disk would disagree about which folder is large,
//! which is worse than being unable to say at all.
//!
//! # Where this runs
//!
//! After the walk, before the roll-up. Not inside the directory read, which is
//! where it would seem to belong: recognising a duplicate needs a set shared by
//! every thread, and a lock around that set is a lock on the whole scan — the
//! one thing ADR 0001's parallel walk exists to avoid.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::platform::FileIdentity;
use crate::size::Size;
use crate::traits::Traits;
use crate::tree::{NodeId, Tree};

/// What a pass of deduplication changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deduplicated {
    /// How many entries turned out to be additional names for bytes counted
    /// elsewhere.
    pub shared_names: u64,
    /// How many bytes were counted more than once and are not any more.
    ///
    /// This is the size of the error being corrected, and it is worth reporting
    /// rather than silently fixing: on a volume with a package manager's store
    /// it is tens of gigabytes, and a reader comparing midda against another
    /// tool deserves to know which of them is explaining itself.
    pub reclaimed: Size,
}

impl Deduplicated {
    /// Whether anything was shared at all. False on nearly every volume.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.shared_names > 0
    }
}

/// Charges each set of shared bytes to one name, and marks the others.
///
/// Call after every node is pushed and before [`Tree::roll_up`]: this rewrites
/// leaf sizes, and a roll-up that had already run would carry the doubled
/// numbers in every ancestor total.
///
/// Files whose identity the platform would not report are left alone. An unknown
/// identity is not evidence of sharing, and treating two unknowns as equal would
/// zero out a real file's bytes — an error in the direction this product must
/// never err in.
pub fn count_shared_once(tree: &mut Tree) -> Deduplicated {
    // The capacity is deliberately not the node count: on a real volume almost
    // nothing is linked, and reserving four million slots to hold a few hundred
    // would cost more than the pass it serves.
    let mut owner_of: HashMap<FileIdentity, NodeId> = HashMap::new();
    let mut outcome = Deduplicated::default();

    // Ascending node order is what makes the owner the shallowest name: the
    // walk pushes a level before the level below it, so the first id holding an
    // identity is the shallowest name of those bytes.
    let candidates: Vec<(NodeId, FileIdentity)> = tree
        .nodes()
        .filter_map(|(id, node)| {
            // One name is not sharing, and it is the overwhelming majority of
            // every volume: checking it here keeps the map small.
            let links = node.links?;
            if links <= 1 {
                return None;
            }
            node.identity.map(|identity| (id, identity))
        })
        .collect();

    for (id, identity) in candidates {
        use std::collections::hash_map::Entry;

        // One lookup decides both things: whether these bytes have been claimed,
        // and by whom. The first id to arrive keeps them; every later one is
        // another name for them.
        match owner_of.entry(identity) {
            Entry::Vacant(slot) => {
                slot.insert(id);
                tree.mark(id, |node| {
                    node.traits.insert(Traits::LINKED.with(Traits::COUNTED_HERE));
                });
            }
            Entry::Occupied(_) => {
                let released = tree.node(id).size;
                tree.mark(id, |node| {
                    // The bytes are already counted under the owner. This name
                    // occupies nothing, which is the literal truth: deleting it
                    // returns nothing.
                    node.size = Size::zero();
                    node.traits.insert(Traits::LINKED);
                });
                outcome.shared_names += 1;
                outcome.reclaimed.add(released);
            }
        }
    }

    outcome
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tree::{Kind, Node, ROOT};

    /// A file with `links` names over the bytes identified by `file`.
    fn linked(name: &str, parent: NodeId, bytes: u64, links: u32, file: u128) -> Node {
        Node {
            name: name.into(),
            parent,
            kind: Kind::File,
            size: Size::flat(bytes),
            modified: None,
            subtree_modified: None,
            entries: 1,
            children: Vec::new(),
            traits: Traits::none(),
            links: Some(links),
            identity: Some(FileIdentity { volume: 1, file }),
        }
    }

    fn plain(name: &str, parent: NodeId, bytes: u64) -> Node {
        Node {
            links: Some(1),
            ..linked(name, parent, bytes, 1, 0)
        }
    }

    fn directory(name: &str, parent: NodeId) -> Node {
        Node {
            kind: Kind::Directory,
            size: Size::zero(),
            links: None,
            identity: None,
            ..linked(name, parent, 0, 1, 0)
        }
    }

    #[test]
    fn two_names_for_one_file_cost_what_one_file_costs() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let first = tree.push(ROOT, linked("data.bin", ROOT, 53_248, 2, 7));
        let second = tree.push(ROOT, linked("also-data.bin", ROOT, 53_248, 2, 7));

        let outcome = count_shared_once(&mut tree);
        tree.roll_up();

        assert_eq!(outcome.shared_names, 1);
        assert_eq!(outcome.reclaimed, Size::flat(53_248));
        assert_eq!(tree.node(first).size, Size::flat(53_248), "the first name keeps the bytes");
        assert_eq!(tree.node(second).size, Size::zero(), "the second name occupies nothing");
        assert_eq!(tree.root().size, Size::flat(53_248), "the volume gets back what one file costs, not two");
    }

    #[test]
    fn a_shared_name_says_so_and_the_owner_says_which_it_is() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let first = tree.push(ROOT, linked("data.bin", ROOT, 4096, 2, 7));
        let second = tree.push(ROOT, linked("also-data.bin", ROOT, 4096, 2, 7));

        count_shared_once(&mut tree);

        assert!(tree.node(first).traits.has(Traits::LINKED));
        assert!(tree.node(first).traits.has(Traits::COUNTED_HERE));
        assert!(!tree.node(first).traits.is_shared_name(), "the row holding the bytes needs no footnote");

        assert!(tree.node(second).traits.is_shared_name(), "the row showing zero is the one to explain");
    }

    #[test]
    fn the_first_name_in_node_order_keeps_the_bytes_even_when_it_is_the_deeper_one() {
        // Built so the deeper name is pushed FIRST. Ownership follows node order,
        // which in a real walk means the shallowest name — but the rule is node
        // order, and a test that only ever saw a shallow-first arena could not
        // tell the two statements apart.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let deep = tree.push(ROOT, directory("deep", ROOT));
        let buried = tree.push(deep, linked("buried.bin", deep, 8192, 2, 11));
        let shallow = tree.push(ROOT, linked("shallow.bin", ROOT, 8192, 2, 11));

        count_shared_once(&mut tree);

        assert!(buried < shallow, "the fixture only means something if the buried name has the lower id");
        assert_eq!(tree.node(buried).size, Size::flat(8192));
        assert_eq!(tree.node(shallow).size, Size::zero());
    }

    #[test]
    fn three_names_leave_the_bytes_on_exactly_one() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let names: Vec<NodeId> = (0..3).map(|n| tree.push(ROOT, linked(&format!("name-{n}.bin"), ROOT, 53_248, 3, 7))).collect();

        let outcome = count_shared_once(&mut tree);
        tree.roll_up();

        assert_eq!(outcome.shared_names, 2, "three names, two of them extra");
        assert_eq!(tree.node(names[0]).size, Size::flat(53_248));
        assert_eq!(tree.node(names[1]).size, Size::zero());
        assert_eq!(tree.node(names[2]).size, Size::zero());
        assert_eq!(tree.root().size, Size::flat(53_248));
    }

    #[test]
    fn two_different_files_that_both_have_links_are_not_each_other() {
        // The mistake this catches: keying on "is linked" rather than on the
        // identity. Both files here are linked twice; neither is the other.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let a1 = tree.push(ROOT, linked("a.bin", ROOT, 1000, 2, 7));
        let b1 = tree.push(ROOT, linked("b.bin", ROOT, 2000, 2, 9));
        let a2 = tree.push(ROOT, linked("a-again.bin", ROOT, 1000, 2, 7));
        let b2 = tree.push(ROOT, linked("b-again.bin", ROOT, 2000, 2, 9));

        count_shared_once(&mut tree);
        tree.roll_up();

        assert_eq!(tree.node(a1).size, Size::flat(1000));
        assert_eq!(tree.node(b1).size, Size::flat(2000));
        assert_eq!(tree.node(a2).size, Size::zero());
        assert_eq!(tree.node(b2).size, Size::zero());
        assert_eq!(tree.root().size, Size::flat(3000));
    }

    #[test]
    fn the_same_file_id_on_another_volume_is_another_file() {
        // A scan crossing a mount point meets file ids from two volumes, and they
        // collide. Dropping the volume from the key would zero out a real file's
        // bytes, which is the one direction this product must not err in.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let here = tree.push(
            ROOT,
            Node {
                identity: Some(FileIdentity { volume: 1, file: 7 }),
                ..linked("here.bin", ROOT, 4096, 2, 7)
            },
        );
        let elsewhere = tree.push(
            ROOT,
            Node {
                identity: Some(FileIdentity { volume: 2, file: 7 }),
                ..linked("elsewhere.bin", ROOT, 4096, 2, 7)
            },
        );

        let outcome = count_shared_once(&mut tree);
        tree.roll_up();

        assert_eq!(outcome.shared_names, 0, "same file id, different volume, different bytes");
        assert_eq!(tree.node(here).size, Size::flat(4096));
        assert_eq!(tree.node(elsewhere).size, Size::flat(4096));
        assert_eq!(tree.root().size, Size::flat(8192));
    }

    #[test]
    fn an_ordinary_file_is_left_exactly_as_it_was() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let one = tree.push(ROOT, plain("one.bin", ROOT, 4096));
        let two = tree.push(ROOT, plain("two.bin", ROOT, 8192));

        let outcome = count_shared_once(&mut tree);
        tree.roll_up();

        assert!(!outcome.any());
        assert_eq!(tree.node(one).size, Size::flat(4096));
        assert_eq!(tree.node(two).size, Size::flat(8192));
        assert!(!tree.node(one).traits.any(), "a file with one name has nothing to explain");
        assert_eq!(tree.root().size, Size::flat(12_288));
    }

    #[test]
    fn two_files_that_merely_share_a_size_are_not_shared_bytes() {
        // Identical sizes, identical names even, no link count above one: this is
        // most of a real disk, and touching any of it would be catastrophic.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let a = tree.push(ROOT, directory("a", ROOT));
        let b = tree.push(ROOT, directory("b", ROOT));
        tree.push(a, plain("same.bin", a, 50_000));
        tree.push(b, plain("same.bin", b, 50_000));

        let outcome = count_shared_once(&mut tree);
        tree.roll_up();

        assert!(!outcome.any());
        assert_eq!(tree.root().size, Size::flat(100_000), "two copies of the same bytes are still two copies");
    }

    #[test]
    fn a_file_whose_identity_is_unknown_keeps_its_bytes() {
        // The platform refused to say. Two refusals are not a match: guessing that
        // they are would delete a real file's bytes from the total.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let first = tree.push(
            ROOT,
            Node {
                identity: None,
                ..linked("mystery-a.bin", ROOT, 4096, 2, 7)
            },
        );
        let second = tree.push(
            ROOT,
            Node {
                identity: None,
                ..linked("mystery-b.bin", ROOT, 4096, 2, 7)
            },
        );

        let outcome = count_shared_once(&mut tree);
        tree.roll_up();

        assert_eq!(outcome.shared_names, 0);
        assert_eq!(tree.node(first).size, Size::flat(4096));
        assert_eq!(tree.node(second).size, Size::flat(4096));
    }

    #[test]
    fn a_link_count_the_platform_would_not_report_is_not_treated_as_sharing() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let first = tree.push(
            ROOT,
            Node {
                links: None,
                ..linked("a.bin", ROOT, 4096, 2, 7)
            },
        );
        let second = tree.push(
            ROOT,
            Node {
                links: None,
                ..linked("b.bin", ROOT, 4096, 2, 7)
            },
        );

        count_shared_once(&mut tree);
        tree.roll_up();

        assert_eq!(tree.root().size, Size::flat(8192), "unknown is not evidence");
        assert_eq!(tree.node(first).size, Size::flat(4096));
        assert_eq!(tree.node(second).size, Size::flat(4096));
    }

    #[test]
    fn shared_bytes_leave_the_folder_totals_adding_up_to_the_volume() {
        // The property the whole choice of "one name owns the bytes" exists to
        // keep. Charging them to nobody would break this line.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let store = tree.push(ROOT, directory("store", ROOT));
        let project = tree.push(ROOT, directory("project", ROOT));
        tree.push(store, linked("package.bin", store, 100_000, 2, 7));
        tree.push(project, linked("package.bin", project, 100_000, 2, 7));
        tree.push(project, plain("own.bin", project, 4096));

        count_shared_once(&mut tree);
        tree.roll_up();

        let parts = tree.node(store).size.allocated + tree.node(project).size.allocated;
        assert_eq!(parts, tree.root().size.allocated, "the parts add up to the whole");
        assert_eq!(tree.root().size.allocated, 104_096, "100000 shared bytes counted once, plus 4096 of its own");
    }

    #[test]
    fn a_folder_that_reads_as_empty_can_say_why() {
        // The row that alarmed on screen during the live run of 2026-09-20: a
        // folder whose only child is a second name reports 0 B, and without
        // this it reports it with nothing to explain itself — a folder the
        // reader can see is full, drawn as empty.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let store = tree.push(ROOT, directory("store", ROOT));
        let project = tree.push(ROOT, directory("project", ROOT));
        tree.push(store, linked("package.bin", store, 100_000, 2, 7));
        tree.push(project, linked("package.bin", project, 100_000, 2, 7));

        count_shared_once(&mut tree);
        tree.roll_up();

        assert_eq!(
            tree.node(project).size,
            Size::zero(),
            "the fixture only means anything if this folder reads as empty"
        );
        assert!(
            tree.node(project).traits.has(Traits::HOLDS_SHARED),
            "a folder holding a second name has to be able to say so"
        );
        assert!(tree.root().traits.has(Traits::HOLDS_SHARED), "and the fact reaches the root");
    }

    #[test]
    fn the_folder_holding_the_bytes_says_so_too() {
        // Both folders are involved, and both are worth marking: one reads as
        // empty and needs the explanation, the other reads as large and is the
        // place the reader has to go to actually free the space.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let store = tree.push(ROOT, directory("store", ROOT));
        tree.push(store, linked("package.bin", store, 100_000, 2, 7));
        tree.push(ROOT, linked("package.bin", ROOT, 100_000, 2, 7));

        count_shared_once(&mut tree);
        tree.roll_up();

        assert!(tree.node(store).traits.has(Traits::HOLDS_SHARED));
    }

    #[test]
    fn a_folder_with_nothing_shared_under_it_stays_unmarked() {
        // The guard that keeps the note off every folder on the disk.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let plain_folder = tree.push(ROOT, directory("plain", ROOT));
        tree.push(plain_folder, plain("one.bin", plain_folder, 4096));

        count_shared_once(&mut tree);
        tree.roll_up();

        assert!(!tree.node(plain_folder).traits.has(Traits::HOLDS_SHARED));
        assert!(!tree.root().traits.has(Traits::HOLDS_SHARED));
    }

    #[test]
    fn only_the_shared_fact_travels_upward() {
        // A file being compressed or sparse says nothing about the folder above
        // it; a second name under it does. Rolling the others up would put a
        // wrong sentence on a right number.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let folder = tree.push(ROOT, directory("folder", ROOT));
        tree.push(
            folder,
            Node {
                traits: Traits::COMPRESSED.with(Traits::SPARSE).with(Traits::PLACEHOLDER),
                ..plain("odd.bin", folder, 4096)
            },
        );

        count_shared_once(&mut tree);
        tree.roll_up();

        let inherited = tree.node(folder).traits;
        assert!(!inherited.has(Traits::COMPRESSED));
        assert!(!inherited.has(Traits::SPARSE));
        assert!(!inherited.has(Traits::PLACEHOLDER));
        assert!(!inherited.has(Traits::HOLDS_SHARED));
    }

    #[test]
    fn deduplicating_after_the_roll_up_does_not_repair_the_totals() {
        // The ordering contract as a test rather than as a comment: a roll-up that
        // ran first leaves every ancestor holding the doubled sum, and the leaves
        // then look right while the totals do not.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        tree.push(ROOT, linked("a.bin", ROOT, 50_000, 2, 7));
        tree.push(ROOT, linked("b.bin", ROOT, 50_000, 2, 7));

        tree.roll_up();
        assert_eq!(tree.root().size.allocated, 100_000, "this is what the wrong order leaves behind");

        count_shared_once(&mut tree);
        assert_eq!(tree.root().size.allocated, 100_000, "and deduplicating afterwards does not undo it");
    }
}
