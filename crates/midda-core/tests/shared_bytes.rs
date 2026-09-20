//! The deduplication against real hard links on a real filesystem.
//!
//! The unit tests in `links` build an arena by hand and prove the arithmetic.
//! That is necessary and it is not sufficient: it proves nothing about whether
//! the *scanner* ever sets `links` and `identity`, or whether the numbers the
//! platform reports look like the numbers the fixtures assume. A pass that read
//! the identity of nothing would leave every one of those unit tests green.
//!
//! So this makes hard links on the filesystem the test is running on and scans
//! them. It runs on Windows and on Unix alike — the CI runners are Linux, and
//! the walk is the portable half of ADR 0001 — because the whole point is that
//! the scan and the platform agree somewhere other than in a mock.

use std::fs;
use std::path::Path;

use midda_core::{Progress, ROOT, Scanner, Size, Traits, Tree, WalkScanner};

/// Makes `link` a second name for `target`. `None` when the filesystem or the
/// permissions will not allow it.
fn hard_link(target: &Path, link: &Path) -> Option<()> {
    fs::hard_link(target, link).ok()
}

fn scan(root: &Path) -> Tree {
    let progress = Progress::new();
    WalkScanner::new().scan(root, &progress).expect("the scan succeeds")
}

fn node_named<'a>(tree: &'a Tree, name: &str) -> &'a midda_core::Node {
    tree.nodes()
        .find(|(_, node)| node.name == name)
        .map(|(_, node)| node)
        .expect("the fixture has this entry")
}

/// The one name a set of shared bytes was counted under.
///
/// Found rather than assumed. The owner is the first name in node order, and
/// node order follows the volume's own listing — which is not creation order on
/// NTFS and not creation order on ext4 either, by a different rule. A test that
/// named its favourite would be asserting the filesystem's index: it passed on
/// Windows and failed in CI on Linux, which is how this helper came to exist.
fn owner(tree: &Tree) -> &midda_core::Node {
    let owners: Vec<&midda_core::Node> = tree
        .nodes()
        .filter(|(_, node)| node.traits.has(Traits::COUNTED_HERE))
        .map(|(_, node)| node)
        .collect();
    assert_eq!(owners.len(), 1, "exactly one name holds the bytes");
    owners[0]
}

/// Every name that is a second one for bytes counted elsewhere.
fn shared_names(tree: &Tree) -> Vec<&midda_core::Node> {
    tree.nodes().filter(|(_, node)| node.traits.is_shared_name()).map(|(_, node)| node).collect()
}

/// The measurement this version exists to fix, end to end.
#[test]
fn a_file_under_three_names_is_counted_once() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let target = dir.path().join("data.bin");
    fs::write(&target, vec![b'x'; 50_000]).expect("write the payload");

    if hard_link(&target, &dir.path().join("second.bin")).is_none() {
        // A filesystem without hard links, or a machine that will not make one.
        // Skipping is the honest outcome; asserting would be a red test about
        // the runner rather than about midda.
        eprintln!("skipped: this filesystem will not make a hard link");
        return;
    }
    hard_link(&target, &dir.path().join("third.bin")).expect("a second link, on a filesystem that made the first");

    let tree = scan(dir.path());

    // What one copy of the payload actually costs here, measured rather than
    // assumed: the cluster size is the volume's business, and so is which of
    // the three names ends up holding it.
    let one_copy = owner(&tree).size.allocated;
    assert!(one_copy >= 50_000, "a 50000-byte file came back as {one_copy} bytes on disk");

    assert_eq!(
        tree.root().size.allocated,
        one_copy,
        "three names over one payload have to cost what one payload costs"
    );
    assert_eq!(tree.root().size.logical, 50_000, "and the same goes for what they read as");

    let shared = tree.shared();
    assert_eq!(shared.shared_names, 2, "three names, two of them second ones");
    assert_eq!(shared.reclaimed.allocated, one_copy * 2, "two payloads' worth of double counting undone");

    // All three names are there, and two of them occupy nothing.
    for name in ["data.bin", "second.bin", "third.bin"] {
        let node = node_named(&tree, name);
        assert!(node.traits.has(Traits::LINKED), "{name} is one of three names for one file");
    }
    assert_eq!(shared_names(&tree).len(), 2);
}

/// The half of the fix a reader can see: which row explains itself.
#[test]
fn the_second_names_are_the_ones_marked_as_shared() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let target = dir.path().join("data.bin");
    fs::write(&target, vec![b'x'; 20_000]).expect("write the payload");

    if hard_link(&target, &dir.path().join("second.bin")).is_none() {
        eprintln!("skipped: this filesystem will not make a hard link");
        return;
    }

    let tree = scan(dir.path());

    // Which of the two names holds the bytes is the volume's listing order to
    // decide; that one of them does, and that the other says so, is midda's.
    let holder = owner(&tree);
    assert!(holder.traits.has(Traits::LINKED), "the file really does have two names");
    assert!(!holder.traits.is_shared_name(), "the row holding the bytes needs no footnote");
    assert!(holder.size.allocated > 0, "the owner keeps the bytes");

    let others = shared_names(&tree);
    assert_eq!(others.len(), 1, "two names, one of them a second one");
    assert_eq!(others[0].size, Size::zero(), "and it occupies nothing, because deleting it returns nothing");
    assert_ne!(others[0].name, holder.name, "the two are different entries");

    // Both names really are the two the fixture made, whichever way round.
    let mut names = vec![holder.name.as_str(), others[0].name.as_str()];
    names.sort_unstable();
    assert_eq!(names, ["data.bin", "second.bin"]);
}

/// The mistake with the worst consequences: deciding two ordinary files are one.
#[test]
fn two_separate_files_of_identical_content_are_both_counted() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::write(dir.path().join("one.bin"), vec![b'x'; 30_000]).expect("write");
    fs::write(dir.path().join("two.bin"), vec![b'x'; 30_000]).expect("write");

    let tree = scan(dir.path());

    let one = node_named(&tree, "one.bin").size.allocated;
    let two = node_named(&tree, "two.bin").size.allocated;
    assert_eq!(
        tree.root().size.allocated,
        one + two,
        "identical bytes in two files are two files, and deleting one frees its own"
    );
    assert!(!tree.shared().any(), "nothing here is shared");
    assert_eq!(tree.root().size.logical, 60_000);
}

/// The scan has to set the fields at all, or every unit test above is theatre.
#[test]
fn the_scanner_reports_the_link_count_and_the_identity_of_an_ordinary_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    fs::write(dir.path().join("plain.bin"), vec![b'x'; 1000]).expect("write");

    let tree = scan(dir.path());
    let plain = node_named(&tree, "plain.bin");

    assert_eq!(plain.links, Some(1), "an ordinary file has exactly one name, and the platform says so");
    assert!(plain.identity.is_some(), "and it has an identity, which is what a second name would match on");
    assert!(!plain.traits.any(), "and nothing about it needs explaining");
}

/// Two scans of one disk have to agree about which name holds the bytes.
///
/// This is the property the ownership rule was chosen for: "first seen by the
/// walk" would pass every other test here and fail this one intermittently,
/// because the walk runs on a work-stealing pool.
#[test]
fn which_name_holds_the_bytes_does_not_change_between_scans() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    // Enough names, spread across enough directories, that a scheduler-dependent
    // answer would have room to differ: one payload reachable from five folders.
    let target = dir.path().join("payload.bin");
    fs::write(&target, vec![b'x'; 40_000]).expect("write the payload");
    for n in 0..5 {
        let folder = dir.path().join(format!("folder-{n}"));
        fs::create_dir(&folder).expect("create a folder");
        if hard_link(&target, &folder.join("name.bin")).is_none() {
            eprintln!("skipped: this filesystem will not make a hard link");
            return;
        }
    }

    // The path of whichever name holds the bytes — not a particular one, which
    // is the whole point: the claim is that the answer does not move, not that
    // it is any given folder.
    let owner_in = |tree: &Tree| -> String {
        tree.nodes()
            .find(|(_, node)| node.traits.has(Traits::COUNTED_HERE))
            .map(|(id, _)| tree.path_of(id).to_string_lossy().into_owned())
            .expect("some name holds the bytes")
    };

    let first = scan(dir.path());
    let second = scan(dir.path());

    assert_eq!(
        owner_in(&first),
        owner_in(&second),
        "two scans of one disk disagreed about which name is the large one"
    );
    assert_eq!(first.root().size.allocated, second.root().size.allocated);
    assert_eq!(first.shared().shared_names, 5, "six names, five of them second ones");
}

/// A shared payload must not make the parts stop adding up to the whole.
#[test]
fn folder_totals_still_add_up_to_the_root_when_bytes_are_shared() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = dir.path().join("store");
    let project = dir.path().join("project");
    fs::create_dir(&store).expect("create store");
    fs::create_dir(&project).expect("create project");

    let target = store.join("package.bin");
    fs::write(&target, vec![b'x'; 60_000]).expect("write the payload");
    if hard_link(&target, &project.join("package.bin")).is_none() {
        eprintln!("skipped: this filesystem will not make a hard link");
        return;
    }
    fs::write(project.join("own.bin"), vec![b'x'; 5000]).expect("write");

    let tree = scan(dir.path());

    let parts: u64 = tree.root().children.iter().map(|&child| tree.node(child).size.allocated).sum();
    assert_eq!(parts, tree.root().size.allocated, "the folders under the root have to add up to the root");

    // Exactly one of the two folders holds the payload, and the other holds only
    // what is its own. WHICH one is the volume's listing order to decide, not
    // the fixture's creation order — measured here, 2026-09-20: NTFS returned
    // `project` before `store` although `store` was created first. So the
    // assertion is about the shape of the answer, not about which folder wins:
    // asserting a favourite would be asserting the filesystem's index.
    let store_bytes = node_named(&tree, "store").size.allocated;
    let project_bytes = node_named(&tree, "project").size.allocated;
    let own_bytes = node_named(&tree, "own.bin").size.allocated;

    let (holder, other) = if store_bytes > project_bytes {
        (store_bytes, project_bytes)
    } else {
        (project_bytes, store_bytes)
    };

    assert!(holder >= 60_000, "one of the two folders has to hold the payload; the larger held {holder}");
    assert!(
        other <= own_bytes,
        "the other holds only what is its own, and held {other} against {own_bytes} of its own files"
    );
    assert_eq!(tree.root().size.logical, 65_000, "the payload once, plus the file that is nobody else's");
}

/// The counter the window watches is allowed to disagree with the final total,
/// and it is worth knowing by how much and in which direction.
#[test]
fn the_progress_counter_counts_before_the_deduplication_runs() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let target = dir.path().join("data.bin");
    fs::write(&target, vec![b'x'; 50_000]).expect("write the payload");
    if hard_link(&target, &dir.path().join("second.bin")).is_none() {
        eprintln!("skipped: this filesystem will not make a hard link");
        return;
    }

    let progress = Progress::new();
    let tree = WalkScanner::new().scan(dir.path(), &progress).expect("the scan succeeds");

    assert!(
        progress.bytes() > tree.root().size.allocated,
        "the running counter saw both names; the total knows better"
    );
    assert_eq!(
        progress.bytes() - tree.root().size.allocated,
        tree.shared().reclaimed.allocated,
        "and the gap between them is exactly what was double-counted"
    );
    assert_eq!(progress.entries(), tree.root().entries - ROOT as u64 - 1);
}
