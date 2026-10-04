//! The change journal, read on a real volume: what it reports is enough to
//! bring a tree up to date, and a position it no longer holds is refused.
//!
//! The journal is read through the volume device, which needs an elevated
//! process — the same as the MFT reader. Where it cannot be opened the tests
//! say so on stderr and pass, unless `MIDDA_REQUIRE_MFT` is set, as it is on
//! the elevated CI runner.

#![cfg(windows)]

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use midda_core::fresh::journal::{Gap, Journal};
use midda_core::fresh::since;
use midda_core::{Index, JournalPosition, Progress, Scanner, Tree, WalkScanner};

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

/// The journal of `root`'s volume, or `None` when this process may not read
/// it and that is allowed to pass.
fn journal(root: &Path) -> Option<Journal> {
    match Journal::open(root) {
        Ok(journal) => Some(journal),
        Err(error) => {
            assert!(
                std::env::var_os("MIDDA_REQUIRE_MFT").is_none_or(|value| value.is_empty()),
                "MIDDA_REQUIRE_MFT is set, but the change journal cannot be read: {error}"
            );
            eprintln!("SKIPPED: the change journal cannot be read here ({error}). Run elevated, or in CI.");
            None
        }
    }
}

fn folder() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

#[test]
fn what_the_journal_reports_is_enough_to_keep_the_tree_true() {
    let dir = folder();
    let root = dir.path();
    write(&root.join("project/src/main.rs"), 1000);
    write(&root.join("project/target/app.bin"), 50_000);
    write(&root.join("docs/readme.md"), 300);
    let Some(mut journal) = journal(root) else {
        return;
    };

    let mut index = Index::new(scan(root), SystemTime::now());
    let from = journal.position();

    write(&root.join("project/src/lib.rs"), 2500);
    fs::remove_file(root.join("docs/readme.md")).expect("delete");
    fs::rename(root.join("project/target"), root.join("project/build")).expect("rename");
    write(&root.join("node_modules/pkg/lib/index.js"), 4000);
    write(&root.join("project/build/app.bin"), 90_000);

    let records = journal.read_from(from).expect("the journal reads");
    assert!(journal.position().next > from.next, "the journal moved on");
    let paths = journal.paths(&records);
    assert!(paths.iter().any(|path| path.ends_with("lib.rs")), "the new file is reported: {paths:?}");

    index.apply(index.observe(paths));
    index.compact();
    assert_same(&scan(root), index.tree());
}

#[test]
fn a_position_the_journal_no_longer_holds_is_refused() {
    let dir = folder();
    let Some(mut journal) = journal(dir.path()) else {
        return;
    };
    let now = journal.position();
    assert_eq!(
        journal.read_from(JournalPosition {
            journal: now.journal.wrapping_add(1),
            next: now.next,
        }),
        Err(Gap::Recreated.into())
    );
    assert_eq!(
        journal.reaches(JournalPosition {
            journal: now.journal,
            next: -1
        }),
        Err(Gap::Overwritten)
    );
    assert_eq!(journal.reaches(now), Ok(()));
}

#[test]
fn the_files_written_since_a_moment_are_found_from_the_journal_alone() {
    let dir = folder();
    let root = dir.path();
    write(&root.join("old/kept.bin"), 1000);
    let Some(mut journal) = journal(root) else {
        return;
    };
    let from = journal.position();
    let moment = SystemTime::now() - Duration::from_secs(1);
    write(&root.join("downloads/big.iso"), 400_000);
    write(&root.join("old/kept.bin"), 70_000);

    let records = journal.read_from(from).expect("the journal reads");
    let index = Index::new(scan(root), SystemTime::now());
    let changes = since::written_since(&index, &records, moment, journal.place());
    assert_eq!((changes.created, changes.written), (1, 1), "the download is new and the old file was written");
    assert_eq!(changes.tree.root().size.logical, 470_000, "both at what they hold now");
}
