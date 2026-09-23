//! The engine behind midda: it reads a volume and hands back the tree of what
//! occupies it.
//!
//! Nothing in here knows about a window. The desktop shell calls [`scan`] and
//! reads a [`Tree`]; a future CLI and the MCP door of v0.19 call the same
//! thing. One core, several doors.
//!
//! # Two numbers, not one
//!
//! Every entry carries both the bytes it reads as and the bytes it occupies,
//! from the first version rather than added later. Folder totals and everything
//! shown are the occupied ones, because someone who opened a disk analyzer came
//! to free space. See [`size`].
//!
//! # And why they differ
//!
//! When the two numbers are far apart there is a reason, and the reason travels
//! with the entry: a cloud placeholder, NTFS compression, a sparse file, or a
//! second name for bytes counted elsewhere. See [`traits`].
//!
//! Shared bytes are the one of those four that is a property of the *scan*
//! rather than of the file: a hard-linked file measured under each of its names
//! is counted as many times as it has them, so [`links`] charges it to one name
//! and marks the rest. Without that pass a package manager's store doubles the
//! volume, invisibly, with every individual number correct.
//!
//! # One folder, two answers
//!
//! The same children come back as an ordered page for a table ([`order`]) and
//! as rectangles for a picture ([`treemap`]). Both orders are decided here, so
//! the list and the map agree about which thing is the big one — and the CLI
//! and the MCP door of v0.19 get the same two answers without a second
//! implementation of either.
//!
//! # Two scanners, one trait
//!
//! [`Scanner`] is the seam between the filesystem walk that works everywhere
//! ([`walk`]) and the read of the NTFS Master File Table that needs elevation
//! and finishes in seconds ([`mft`]). [`scan`] picks the fast one when it can
//! and says which one it used; `tests/scanners_agree.rs` holds the two to the
//! same tree. See `docs/adr/0001-hybrid-scanning.md` and
//! `docs/adr/0006-accelerate-by-relaunching-elevated.md`.
//!
//! ```no_run
//! use midda_core::{Progress, SizeBasis, scan};
//!
//! let progress = Progress::new();
//! let tree = scan(std::path::Path::new("."), &progress)?;
//!
//! for id in tree.children_by_size(midda_core::ROOT, SizeBasis::Allocated) {
//!     let node = tree.node(id);
//!     println!("{:>12}  {}", node.size.allocated, node.name);
//! }
//! # Ok::<(), midda_core::Error>(())
//! ```

pub mod error;
pub mod links;
pub mod mft;
pub mod order;
pub mod platform;
pub mod scanner;
pub mod size;
pub mod traits;
pub mod tree;
pub mod treemap;
pub mod walk;

pub use error::{Error, Result};
pub use links::Deduplicated;
pub use mft::{Acceleration, MftScanner, acceleration};
pub use order::{Direction, Sort, SortKey, Span};
pub use platform::FileIdentity;
pub use scanner::{Progress, Scanner};
pub use size::{Size, SizeBasis};
pub use traits::Traits;
pub use tree::{Kind, Node, NodeId, ROOT, Skipped, Tree, listing_order};
pub use treemap::{Category, Layout, Rect, Tile};
pub use walk::WalkScanner;

use std::path::Path;

/// Scans `root` with the fastest scanner that can read it.
///
/// The MFT reader when the process is elevated and `root` is on an NTFS
/// volume, the walk otherwise. Callers are given this rather than a concrete
/// scanner: the choice is the core's to make, and the window should not grow a
/// copy of it. The tree says which one read it ([`Tree::scanned_by`]).
///
/// When the MFT reader was available and failed anyway — a volume it could not
/// open, a table it could not stream — the walk reads the same root and the
/// tree carries the reason ([`Tree::fallback`]). A slower scan is an answer; a
/// scan that failed because a faster way to the same answer did is not.
///
/// # Errors
///
/// When `root` cannot be read, is not a directory, or the scan was cancelled.
pub fn scan(root: &Path, progress: &Progress) -> Result<Tree> {
    scan_with(&MftScanner::new(), root, progress)
}

/// [`scan`], with the fast scanner given rather than chosen — the seam the
/// fallback is tested through.
fn scan_with(fast: &impl Scanner, root: &Path, progress: &Progress) -> Result<Tree> {
    let mut fallback = None;
    if fast.available(root) {
        match fast.scan(root, progress) {
            Ok(tree) => return Ok(tree),
            // The root itself is the problem, or the reader asked to stop:
            // the walk would only say the same thing slower.
            Err(error @ (Error::Cancelled | Error::Unreadable { .. } | Error::NotADirectory(_))) => return Err(error),
            Err(error) => {
                fallback = Some(format!("the {} scanner could not read this volume: {error}", fast.name()));
                progress.restart();
            }
        }
    }

    let mut tree = WalkScanner::new().scan(root, progress)?;
    if let Some(reason) = fallback {
        tree.record_fallback(reason);
    }
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_door_scans() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(dir.path().join("one.bin"), vec![b'x'; 128]).expect("write");

        let progress = Progress::new();
        let tree = scan(dir.path(), &progress).expect("the scan succeeds");
        assert_eq!(tree.root().size.logical, 128);
    }

    /// A fast scanner that is always available and always fails the same way.
    struct Failing(fn() -> Error);

    impl Scanner for Failing {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn available(&self, _root: &Path) -> bool {
            true
        }

        fn scan(&self, _root: &Path, progress: &Progress) -> Result<Tree> {
            // Counted before failing, so a fallback that forgot to restart the
            // counters would finish above its tree.
            progress.advance(1_000, 1 << 30);
            Err((self.0)())
        }
    }

    #[test]
    fn a_fast_scanner_that_fails_gives_way_to_the_walk_and_says_why() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(dir.path().join("one.bin"), vec![b'x'; 128]).expect("write");

        let progress = Progress::new();
        let tree = scan_with(&Failing(|| Error::Unavailable("the volume is busy".into())), dir.path(), &progress).expect("the walk answers");
        assert_eq!(tree.scanned_by(), "walk");
        let reason = tree.fallback().expect("the fallback is recorded");
        assert!(reason.contains("failing") && reason.contains("the volume is busy"), "{reason}");
        assert_eq!(progress.entries(), tree.root().entries - 1, "the counters restarted with the walk");
    }

    #[test]
    fn a_cancelled_fast_scan_is_not_retried_slowly() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let error = scan_with(&Failing(|| Error::Cancelled), dir.path(), &Progress::new()).expect_err("a stop is a stop");
        assert!(matches!(error, Error::Cancelled));
    }

    #[test]
    fn an_unavailable_fast_scanner_is_not_a_fallback() {
        // Not being elevated is the ordinary case, not a failure to explain.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let tree = scan(dir.path(), &Progress::new()).expect("the scan succeeds");
        if !MftScanner::new().available(dir.path()) {
            assert_eq!(tree.scanned_by(), "walk");
            assert_eq!(tree.fallback(), None);
        }
    }
}
