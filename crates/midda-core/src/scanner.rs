//! The one interface both scanners answer to, and what a scan reports while it
//! runs.
//!
//! midda has two ways to read a volume — a filesystem walk that works without
//! elevation, and an MFT read that needs administrator rights and finishes in
//! seconds (ADR 0001). Only the walk exists today; the trait exists anyway,
//! because the shape of the seam is the decision, and discovering it later
//! means discovering it against code that already assumed one implementation.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::error::Result;
use crate::tree::Tree;

/// How far a scan has got.
///
/// A scan of a system volume runs for minutes on the walk path, and a window
/// with nothing moving in it is a window that looks hung. The counters are
/// atomics rather than messages through a channel: the scan runs on many
/// threads, the reader only wants the latest value, and a channel would make
/// every worker queue behind a consumer that is repainting at 60 Hz.
#[derive(Debug, Default)]
pub struct Progress {
    entries: AtomicU64,
    bytes: AtomicU64,
    cancelled: AtomicBool,
}

impl Progress {
    /// A fresh counter.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Counts entries and bytes seen. Called by scanners, per directory rather
    /// than per file so the contention stays off the hot path.
    pub fn advance(&self, entries: u64, bytes: u64) {
        // `Relaxed` is right: these are counters read for display, and no other
        // memory is published through them.
        self.entries.fetch_add(entries, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// How many entries have been seen.
    #[must_use]
    pub fn entries(&self) -> u64 {
        self.entries.load(Ordering::Relaxed)
    }

    /// How many bytes, on disk, have been counted.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Asks the scan to stop.
    ///
    /// A scan is not killed: it unwinds at the next directory boundary and
    /// returns what it has. A half-scan is not shown as a total, but abandoning
    /// the threads mid-read would leave handles open on a volume the user is
    /// about to act on.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether a stop has been asked for.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// A way of reading a volume into a [`Tree`].
///
/// Implementations must agree: the same directory scanned two ways produces the
/// same tree. That is a rule of the project, held by a test rather than by
/// hope — see `tests/scanners_agree.rs`.
pub trait Scanner {
    /// What this scanner is called, for the freshness line in the toolbar.
    fn name(&self) -> &'static str;

    /// Whether this scanner can run here and now.
    ///
    /// The MFT scanner answers `false` without elevation; the walk always
    /// answers `true`. The window uses this to decide whether to offer
    /// "accelerate", rather than offering it and failing.
    fn available(&self) -> bool;

    /// Scans `root`, reporting into `progress`.
    ///
    /// Returns a tree even when parts of it could not be read: what was refused
    /// is in [`Tree::skipped`], because a scan of a system volume always refuses
    /// something and a total that silently omitted it would be wrong in the
    /// direction that matters.
    ///
    /// # Errors
    ///
    /// When `root` itself cannot be opened, or the scan was cancelled before it
    /// finished.
    fn scan(&self, root: &Path, progress: &Progress) -> Result<Tree>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_counts_what_it_is_given() {
        let progress = Progress::new();
        progress.advance(3, 12_288);
        progress.advance(1, 4096);
        assert_eq!(progress.entries(), 4);
        assert_eq!(progress.bytes(), 16_384);
    }

    #[test]
    fn a_fresh_counter_is_not_cancelled() {
        let progress = Progress::new();
        assert!(!progress.cancelled());
        progress.cancel();
        assert!(progress.cancelled());
    }

    #[test]
    fn counters_add_up_across_threads() {
        // The scan runs on a pool; the point of the atomics is that a count
        // taken from many threads is still the right count.
        let progress = Progress::new();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let progress = Arc::clone(&progress);
                scope.spawn(move || {
                    for _ in 0..1000 {
                        progress.advance(1, 4096);
                    }
                });
            }
        });
        assert_eq!(progress.entries(), 8000);
        assert_eq!(progress.bytes(), 8000 * 4096);
    }
}
