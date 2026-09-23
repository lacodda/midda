//! The one interface both scanners answer to, and what a scan reports while it
//! runs.
//!
//! midda has two ways to read a volume — a filesystem walk that works without
//! elevation, and an MFT read that needs administrator rights and finishes in
//! seconds (ADR 0001). Both answer to this trait, and a test holds them to the
//! same answer.

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
    /// MFT records read so far. Zero on the walk, which has no records.
    records: AtomicU64,
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

    /// Counts records read from the MFT.
    ///
    /// A separate counter rather than entries: the table holds every file on
    /// the volume, and a scan of one folder reads all of them to find the few
    /// under it. Counting records as entries would show the window a number the
    /// finished tree then contradicts by a factor of a thousand.
    pub fn read_records(&self, records: u64) {
        self.records.fetch_add(records, Ordering::Relaxed);
    }

    /// How many MFT records have been read.
    #[must_use]
    pub fn records(&self) -> u64 {
        self.records.load(Ordering::Relaxed)
    }

    /// Clears the counters, keeping a stop that has been asked for.
    ///
    /// Called when the fast scanner gives way to the walk: the walk then counts
    /// from nothing, and a counter still holding the MFT's partial read would
    /// finish above the tree it describes.
    pub fn restart(&self) {
        self.entries.store(0, Ordering::Relaxed);
        self.bytes.store(0, Ordering::Relaxed);
        self.records.store(0, Ordering::Relaxed);
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
/// same tree, node for node. That is a rule of the project, held by a test
/// rather than by hope — see `tests/scanners_agree.rs`.
pub trait Scanner {
    /// What this scanner is called, for the freshness line in the toolbar.
    fn name(&self) -> &'static str;

    /// Whether this scanner can read `root` here and now.
    ///
    /// Asked of a root, not of the machine: the MFT scanner needs elevation
    /// *and* an NTFS volume under the path, and one process can hold both
    /// answers at once — `C:` on NTFS, a USB stick on exFAT. The walk always
    /// answers `true`.
    fn available(&self, root: &Path) -> bool;

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
    fn a_restart_clears_the_counts_but_not_the_stop() {
        let progress = Progress::new();
        progress.advance(5, 100);
        progress.read_records(1000);
        progress.cancel();
        progress.restart();
        assert_eq!((progress.entries(), progress.bytes(), progress.records()), (0, 0, 0));
        assert!(progress.cancelled(), "a stop asked for during the fast read still stops the walk");
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
