//! A tree kept up to date is the tree a fresh scan would read.
//!
//! Each test builds a folder, scans it, changes it on disk, tells the index
//! which paths changed — the way a watcher or the change journal would — and
//! then holds the index, laid out again, to a fresh scan of the same folder:
//! the same arena, node for node, every total and every shared byte. The
//! comparison is the scanners' own (`common::assert_same`), so an update that
//! got anything wrong — a total left behind, a name charged the wrong bytes, a
//! folder's write time — fails here rather than on someone's screen.
//!
//! Synthetic folders, like every fixture in the line. They run on Windows and
//! on the Linux runner alike.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use midda_core::{Index, Progress, ROOT, Scanner, Size, Traits, Tree, WalkScanner};

mod common;

use common::assert_same;

fn write(path: &Path, bytes: usize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create the parent");
    }
    fs::write(path, vec![b'a'; bytes]).expect("write a fixture file");
}

fn scan(root: &Path) -> Tree {
    WalkScanner::new().scan(root, &Progress::default()).expect("the scan succeeds")
}

/// A folder holding a little of everything.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir.path();
    write(&root.join("project/src/main.rs"), 1000);
    write(&root.join("project/target/debug/app.bin"), 50_000);
    write(&root.join("project/target/debug/deps/a.rlib"), 7000);
    write(&root.join("project/target/debug/deps/b.rlib"), 9000);
    write(&root.join("docs/readme.md"), 300);
    write(&root.join("keep.txt"), 5);
    fs::create_dir(root.join("empty")).expect("create an empty folder");
    dir
}

/// Scans `root`, makes `changes`, tells the index the paths they return, and
/// holds the result to a fresh scan.
fn after(root: &Path, changes: impl FnOnce(&Path) -> Vec<PathBuf>) -> Index {
    let mut index = Index::new(scan(root), SystemTime::now());
    let touched = changes(root);
    let observed = index.observe(touched);
    index.apply(observed);
    index.compact();
    assert_same(&scan(root), index.tree());
    index
}

fn find(tree: &Tree, path: &str) -> midda_core::NodeId {
    let wanted = tree.root_path().join(path);
    tree.nodes()
        .find(|(id, _)| tree.path_of(*id) == wanted)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("{path} is not in the tree"))
}

#[test]
fn a_new_file_is_listed_where_a_scan_would_list_it() {
    let dir = fixture();
    after(dir.path(), |root| {
        write(&root.join("project/src/lib.rs"), 2500);
        vec![root.join("project/src/lib.rs")]
    });
}

#[test]
fn a_file_that_grew_is_measured_again() {
    let dir = fixture();
    after(dir.path(), |root| {
        write(&root.join("project/target/debug/app.bin"), 900_000);
        vec![root.join("project/target/debug/app.bin")]
    });
}

#[test]
fn a_deleted_file_goes() {
    let dir = fixture();
    after(dir.path(), |root| {
        fs::remove_file(root.join("docs/readme.md")).expect("delete");
        vec![root.join("docs/readme.md")]
    });
}

#[test]
fn a_deleted_folder_goes_with_everything_in_it() {
    let dir = fixture();
    let index = after(dir.path(), |root| {
        fs::remove_dir_all(root.join("project/target")).expect("delete");
        vec![root.join("project/target")]
    });
    assert_eq!(index.tree().root().size.logical, 1305, "main.rs, readme.md and keep.txt are what is left");
}

#[test]
fn a_new_folder_is_read_whole_from_the_one_path_reported_inside_it() {
    // A watcher reports the files in a new folder, and the folder itself may
    // come later or not at all. The deepest folder the tree holds is where
    // the change starts, and everything under it is read.
    let dir = fixture();
    after(dir.path(), |root| {
        write(&root.join("node_modules/pkg/lib/index.js"), 4000);
        write(&root.join("node_modules/pkg/package.json"), 200);
        write(&root.join("node_modules/other/index.js"), 100);
        vec![root.join("node_modules/pkg/lib/index.js")]
    });
}

#[test]
fn a_renamed_folder_moves_with_what_is_in_it() {
    let dir = fixture();
    after(dir.path(), |root| {
        fs::rename(root.join("project/target"), root.join("project/build")).expect("rename");
        vec![root.join("project/target"), root.join("project/build")]
    });
}

#[test]
fn a_folder_replaced_by_another_of_the_same_name_is_read_again() {
    // Same name, another folder: its contents are not the old contents, and
    // only the folder's identity says so.
    let dir = fixture();
    after(dir.path(), |root| {
        fs::rename(root.join("docs"), root.join("docs-old")).expect("rename");
        write(&root.join("docs/guide/intro.md"), 12_000);
        vec![root.join("docs"), root.join("docs-old")]
    });
}

#[test]
fn a_file_replaced_by_a_folder_of_the_same_name_is_read_as_a_folder() {
    let dir = fixture();
    after(dir.path(), |root| {
        fs::remove_file(root.join("keep.txt")).expect("delete");
        write(&root.join("keep.txt/inner.bin"), 3000);
        vec![root.join("keep.txt")]
    });
}

#[test]
fn a_folder_replaced_by_a_file_of_the_same_name_is_read_as_a_file() {
    let dir = fixture();
    after(dir.path(), |root| {
        fs::remove_dir_all(root.join("docs")).expect("delete");
        write(&root.join("docs"), 777);
        vec![root.join("docs")]
    });
}

#[test]
fn many_changes_reported_together_land_together() {
    let dir = fixture();
    after(dir.path(), |root| {
        write(&root.join("project/src/lib.rs"), 2500);
        fs::remove_file(root.join("project/target/debug/deps/a.rlib")).expect("delete");
        write(&root.join("project/target/debug/deps/b.rlib"), 90_000);
        fs::remove_dir(root.join("empty")).expect("delete");
        write(&root.join("fresh/one.txt"), 10);
        vec![
            root.join("project/src/lib.rs"),
            root.join("project/target/debug/deps/a.rlib"),
            root.join("project/target/debug/deps/b.rlib"),
            root.join("empty"),
            root.join("fresh/one.txt"),
            root.join("fresh"),
            // Reported twice, as a journal does for one save.
            root.join("project/src/lib.rs"),
        ]
    });
}

#[test]
fn a_path_reported_that_did_not_change_changes_nothing() {
    let dir = fixture();
    let mut index = Index::new(scan(dir.path()), SystemTime::now());
    let before = index.tree().root().size;
    let applied = index.apply(index.observe([dir.path().join("docs/readme.md")]));
    assert!(!applied.any(), "{applied:?}");
    assert_eq!(index.tree().root().size, before);
}

#[test]
fn a_path_outside_the_scanned_folder_is_passed_over() {
    let dir = fixture();
    let elsewhere = tempfile::tempdir().expect("another directory");
    write(&elsewhere.path().join("other.bin"), 5000);
    let mut index = Index::new(scan(&dir.path().join("project")), SystemTime::now());
    let applied = index.apply(index.observe([elsewhere.path().join("other.bin"), dir.path().join("keep.txt")]));
    assert!(!applied.any(), "{applied:?}");
}

#[test]
fn what_was_not_touched_keeps_its_id() {
    // The window holds ids: the folder on screen, the row selected. An update
    // that renumbered the tree would pull both out from under it.
    let dir = fixture();
    let mut index = Index::new(scan(dir.path()), SystemTime::now());
    let readme = find(index.tree(), "docs/readme.md");
    write(&dir.path().join("project/src/lib.rs"), 2500);
    fs::remove_dir_all(dir.path().join("project/target")).expect("delete");
    index.apply(index.observe([dir.path().join("project/src/lib.rs"), dir.path().join("project/target")]));

    assert!(index.tree().contains(readme));
    assert_eq!(index.tree().path_of(readme), dir.path().join("docs/readme.md"));
    let gone = index.tree().nodes().any(|(id, _)| index.tree().path_of(id).ends_with("target"));
    assert!(!gone, "a deleted folder is not listed");
}

#[test]
fn a_folder_total_follows_a_change_deep_below_it() {
    let dir = fixture();
    let mut index = Index::new(scan(dir.path()), SystemTime::now());
    let project = find(index.tree(), "project");
    let before = index.tree().node(project).size.logical;
    write(&dir.path().join("project/target/debug/app.bin"), 150_000);
    index.apply(index.observe([dir.path().join("project/target/debug/app.bin")]));
    assert_eq!(index.tree().node(project).size.logical, before + 100_000);
    assert_eq!(
        index.tree().node(ROOT).size.logical,
        index
            .tree()
            .nodes()
            .filter(|(_, node)| !node.is_directory())
            .map(|(_, node)| node.size.logical)
            .sum::<u64>()
    );
}

/// Makes `link` a second name for `target`, when the filesystem allows it.
fn hard_link(target: &Path, link: &Path) -> bool {
    fs::hard_link(target, link).is_ok()
}

#[test]
fn a_second_name_made_later_is_counted_once() {
    let dir = fixture();
    write(&dir.path().join("store/blob.bin"), 60_000);
    if !hard_link(&dir.path().join("store/blob.bin"), &dir.path().join("store/again.bin")) {
        eprintln!("SKIPPED: this filesystem makes no hard links");
        return;
    }
    let index = after(dir.path(), |root| {
        assert!(hard_link(&root.join("store/blob.bin"), &root.join("project/blob.bin")));
        vec![root.join("project/blob.bin")]
    });
    assert_eq!(index.tree().shared().shared_names, 2, "three names, two of them extra");
}

#[test]
fn the_bytes_move_to_the_next_name_when_the_name_holding_them_goes() {
    let dir = fixture();
    write(&dir.path().join("a/blob.bin"), 60_000);
    fs::create_dir_all(dir.path().join("b")).expect("create");
    fs::create_dir_all(dir.path().join("c")).expect("create");
    let made =
        hard_link(&dir.path().join("a/blob.bin"), &dir.path().join("b/blob.bin")) && hard_link(&dir.path().join("a/blob.bin"), &dir.path().join("c/blob.bin"));
    if !made {
        eprintln!("SKIPPED: this filesystem makes no hard links");
        return;
    }
    let index = after(dir.path(), |root| {
        fs::remove_file(root.join("a/blob.bin")).expect("delete the name holding the bytes");
        vec![root.join("a/blob.bin")]
    });
    let owner = index.tree().node(find(index.tree(), "b/blob.bin"));
    assert!(owner.traits.has(Traits::COUNTED_HERE), "the next name in scan order keeps the bytes");
    assert_eq!(owner.links, Some(2), "and knows the file has two names now, though nothing reported it");
}

#[test]
fn a_file_down_to_one_name_is_an_ordinary_file_again() {
    let dir = fixture();
    write(&dir.path().join("pair/one.bin"), 30_000);
    if !hard_link(&dir.path().join("pair/one.bin"), &dir.path().join("pair/two.bin")) {
        eprintln!("SKIPPED: this filesystem makes no hard links");
        return;
    }
    let index = after(dir.path(), |root| {
        fs::remove_file(root.join("pair/two.bin")).expect("delete the second name");
        vec![root.join("pair/two.bin")]
    });
    let one = index.tree().node(find(index.tree(), "pair/one.bin"));
    assert_eq!(one.traits, Traits::none());
    assert_eq!(one.links, Some(1));
    assert!(one.size.logical == 30_000 && one.size != Size::zero());
}

#[test]
fn a_write_through_one_name_is_seen_under_all_of_them() {
    let dir = fixture();
    write(&dir.path().join("pair/one.bin"), 30_000);
    if !hard_link(&dir.path().join("pair/one.bin"), &dir.path().join("pair/two.bin")) {
        eprintln!("SKIPPED: this filesystem makes no hard links");
        return;
    }
    after(dir.path(), |root| {
        // Grown through the name that does not hold the bytes: the one that
        // does is not reported, and has to be found.
        write(&root.join("pair/two.bin"), 300_000);
        vec![root.join("pair/two.bin")]
    });
}

#[test]
#[cfg(windows)]
fn a_change_reported_in_another_case_is_found_under_the_case_on_disk() {
    let dir = fixture();
    after(dir.path(), |root| {
        write(&root.join("project/src/main.rs"), 4321);
        vec![root.join("PROJECT").join("SRC").join("MAIN.RS")]
    });
}

#[test]
#[cfg(windows)]
fn a_rename_that_only_changes_case_is_one_entry_not_two() {
    let dir = fixture();
    after(dir.path(), |root| {
        fs::rename(root.join("keep.txt"), root.join("Keep.TXT")).expect("rename");
        vec![root.join("keep.txt"), root.join("Keep.TXT")]
    });
}
