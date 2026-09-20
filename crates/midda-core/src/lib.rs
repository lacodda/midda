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
//! and the MFT read that arrives in v0.5.0 and needs elevation. Only the walk
//! exists today; the trait exists anyway, because discovering the shape of the
//! seam later means discovering it against code that assumed one
//! implementation. See `docs/adr/0001-hybrid-scanning.md`.
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
pub use order::{Direction, Sort, SortKey, Span};
pub use platform::FileIdentity;
pub use scanner::{Progress, Scanner};
pub use size::{Size, SizeBasis};
pub use traits::Traits;
pub use tree::{Kind, Node, NodeId, ROOT, Skipped, Tree};
pub use treemap::{Category, Layout, Rect, Tile};
pub use walk::WalkScanner;

use std::path::Path;

/// Scans `root` with the best scanner available here.
///
/// Today that is always the walk. From v0.5.0 this picks the MFT reader when
/// the process is elevated and the path sits on an NTFS volume, which is why
/// callers are given this rather than a concrete scanner: the choice is the
/// core's to make, and the window should not grow a copy of it.
///
/// # Errors
///
/// When `root` cannot be read, is not a directory, or the scan was cancelled.
pub fn scan(root: &Path, progress: &Progress) -> Result<Tree> {
    best_scanner().scan(root, progress)
}

/// The scanner this machine can use right now.
#[must_use]
pub fn best_scanner() -> impl Scanner {
    // One candidate so far. When the MFT reader lands the order becomes
    // fastest-first with `available()` deciding, and this is the one place that
    // changes.
    WalkScanner::new()
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

    #[test]
    fn the_best_scanner_here_is_one_that_can_run() {
        assert!(best_scanner().available());
    }
}
