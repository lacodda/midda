//! Snapshots on disk, and comparisons made from them.
//!
//! A snapshot is only worth keeping if comparing it later says exactly what
//! comparing the tree it was taken from would have said — whatever happened
//! to the saved index meanwhile. These tests take a real folder, keep its
//! picture the two ways midda keeps one, change the folder, save over the
//! index, and hold the comparison read back from disk to the comparison of
//! the trees themselves.
//!
//! Synthetic folders, like every fixture in the line.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use midda_core::fresh::store;
use midda_core::snapshot::{self, Kind};
use midda_core::{Comparison, Index, Mark, PlaceKind, Progress, ROOT, Scanner, Size, SizeBasis, Tree, WalkScanner, compare};

fn write(path: &Path, bytes: usize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create the parent");
    }
    fs::write(path, vec![b'a'; bytes]).expect("write a fixture file");
}

fn scan(root: &Path) -> Tree {
    WalkScanner::new().scan(root, &Progress::default()).expect("the scan succeeds")
}

/// A project before a day's work: dependencies, a build, a cache.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir.path();
    write(&root.join("app/src/main.rs"), 1_000);
    write(&root.join("app/node_modules/react/index.js"), 30_000);
    write(&root.join("app/node_modules/left-pad/index.js"), 2_000);
    write(&root.join("app/target/debug/app.exe"), 90_000);
    write(&root.join("cache/blob.bin"), 40_000);
    write(&root.join("notes.txt"), 5);
    dir
}

/// The day's work: a dependency grew, one went, a build was cleaned, a
/// download arrived.
fn work(root: &Path) {
    write(&root.join("app/node_modules/react/index.js"), 45_000);
    fs::remove_dir_all(root.join("app/node_modules/left-pad")).expect("remove a dependency");
    fs::remove_dir_all(root.join("app/target")).expect("clean the build");
    write(&root.join("downloads/setup.exe"), 120_000);
}

fn path_in(comparison: &Comparison, path: &str) -> midda_core::NodeId {
    path.split('/')
        .try_fold(ROOT, |at, name| comparison.tree().child(at, name))
        .unwrap_or_else(|| panic!("{path} is not in the comparison"))
}

/// Two comparisons say the same thing: the same entries, marks and sizes
/// at both moments.
fn assert_same_comparison(expected: &Comparison, actual: &Comparison) {
    let (a, b) = (expected.tree(), actual.tree());
    let paths = |tree: &Tree| tree.nodes().map(|(id, _)| tree.path_of(id)).collect::<Vec<_>>();
    assert_eq!(paths(a), paths(b), "the same entries");
    for ((x, left), (y, right)) in a.nodes().zip(b.nodes()) {
        let at = a.path_of(x);
        assert_eq!(left.size, right.size, "now at {}", at.display());
        assert_eq!(expected.before(x), actual.before(y), "then at {}", at.display());
        assert_eq!(expected.mark(x), actual.mark(y), "mark at {}", at.display());
        assert_eq!(expected.moved(x), actual.moved(y), "moved at {}", at.display());
    }
    assert_eq!(expected.totals(), actual.totals());
}

#[test]
fn the_picture_of_last_time_outlives_the_index_saved_over_it() {
    let dir = fixture();
    let store_dir = tempfile::tempdir().expect("a store");
    let root = dir.path();

    // A session ends: the index is saved.
    let first = Index::new(scan(root), SystemTime::now());
    let saved = store::save(&first, store_dir.path(), SystemTime::now()).expect("save the index");

    // The next opens it, and keeps it as last time before anything changes.
    let folder = snapshot::folder_for(&store_dir.path().join("snapshots"), root);
    snapshot::keep_last(&folder, &saved).expect("keep last time");

    // The folder changes, and the session saves its index over the old one.
    work(root);
    let second = Index::new(scan(root), SystemTime::now());
    store::save(&second, store_dir.path(), SystemTime::now()).expect("save over it");

    let last = snapshot::load(&folder, "last.midx").expect("last time reads back");
    let from_disk = compare(last.index.tree(), second.tree());
    let from_memory = compare(first.tree(), second.tree());
    assert!(!from_memory.tree().is_empty(), "the day's work is a change");
    assert_same_comparison(&from_memory, &from_disk);
}

#[test]
fn a_comparison_of_a_real_folder_says_what_grew_and_what_came_back() {
    let dir = fixture();
    let root = dir.path();
    let then = scan(root);
    work(root);
    let now = scan(root);
    let comparison = compare(&then, &now);

    let target = path_in(&comparison, "app/target");
    assert_eq!(comparison.mark(target), Mark::Gone);
    assert!(comparison.before(target).logical >= 90_000);
    assert_eq!(comparison.tree().node(target).size, Size::zero());
    assert_eq!(comparison.mark(path_in(&comparison, "downloads/setup.exe")), Mark::New);
    assert_eq!(comparison.mark(path_in(&comparison, "app/node_modules/react/index.js")), Mark::Changed);
    assert!(comparison.tree().child(ROOT, "notes.txt").is_none(), "what did not change is not listed");
    assert!(
        comparison.tree().child(path_in(&comparison, "app"), "src").is_none(),
        "nor is a folder where nothing changed"
    );

    // The places: the build and the dependency went whole, the download came
    // whole, and the dependency that grew is its own folder's file.
    let places = comparison.places();
    let kinds = |kind| {
        let mut found: Vec<String> = places
            .iter()
            .filter(|place| place.kind == kind)
            .map(|place| comparison.tree().node(place.id).name.clone())
            .collect();
        found.sort();
        found
    };
    assert_eq!(kinds(PlaceKind::Gone), ["left-pad", "target"]);
    assert_eq!(kinds(PlaceKind::New), ["downloads"]);
    assert_eq!(kinds(PlaceKind::Files), ["react"]);

    // On both sizes, the folder's own totals agree with the sum of what moved.
    let totals = comparison.totals();
    for basis in [SizeBasis::Allocated, SizeBasis::Logical] {
        let net = i128::from(basis.of(totals.now)) - i128::from(basis.of(totals.before));
        assert_eq!(net, i128::from(basis.of(totals.grown)) - i128::from(basis.of(totals.freed)), "{basis:?}");
        assert_eq!(i128::from(comparison.growth(ROOT, basis)), net);
    }
    assert_eq!(totals.before, then.root().size);
    assert_eq!(totals.now, now.root().size);
}

#[test]
fn taken_snapshots_are_listed_newest_first_and_can_be_deleted() {
    let dir = fixture();
    let store_dir = tempfile::tempdir().expect("a store");
    let root = dir.path();
    let folder = snapshot::folder_for(store_dir.path(), root);

    let index = Index::new(scan(root), SystemTime::now());
    let monday = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
    let tuesday = monday + Duration::from_secs(86_400);
    let early = snapshot::keep(&store::encode(&index, monday), &folder, monday).expect("take one");
    let late = snapshot::keep(&store::encode(&index, tuesday), &folder, tuesday).expect("take another");
    let saved = store::save(&index, store_dir.path(), monday - Duration::from_secs(3_600)).expect("save the index");
    snapshot::keep_last(&folder, &saved).expect("keep last time");

    let listed = snapshot::list(&folder, root);
    let order: Vec<_> = listed.iter().map(|snapshot| (snapshot.kind, snapshot.at)).collect();
    assert_eq!(
        order,
        [(Kind::Taken, tuesday), (Kind::Taken, monday), (Kind::Last, monday - Duration::from_secs(3_600))],
        "each dated by the moment it is a picture of"
    );
    assert!(listed.iter().all(|snapshot| snapshot.unusable.is_none() && snapshot.bytes > 0));
    assert_eq!(listed[0].name, late.name);

    snapshot::remove(&folder, &early.name).expect("delete one");
    assert_eq!(snapshot::list(&folder, root).len(), 2);
    assert!(snapshot::remove(&folder, "../escape.midx").is_err(), "only a snapshot's own name is deleted");
}

#[test]
fn a_snapshot_of_another_version_is_listed_with_the_reason_and_not_read() {
    let dir = tempfile::tempdir().expect("a folder");
    let folder = dir.path().join("snapshots");
    fs::create_dir_all(&folder).expect("make the folder");
    let mut bytes = b"MIDX".to_vec();
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    fs::write(folder.join("taken-1790000000000.midx"), bytes).expect("write a file from the future");

    let listed = snapshot::list(&folder, dir.path());
    assert_eq!(listed.len(), 1);
    assert!(listed[0].unusable.as_deref().is_some_and(|why| why.contains("another version")), "{listed:?}");
    assert!(snapshot::load(&folder, &listed[0].name).is_err());
}
