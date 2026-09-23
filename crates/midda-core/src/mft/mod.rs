//! The fast scanner: the volume's Master File Table, read as one stream.
//!
//! This is the half of ADR 0001 that needs elevation. NTFS keeps a record for
//! every file on the volume — its names and the directories they sit in, its
//! sizes, its timestamps — in one table, and reading that table front to back
//! is a few sequential reads where the walk is a syscall per directory and a
//! handle per file. A system volume that takes the walk minutes takes this
//! seconds.
//!
//! The scanner is two halves with a line between them that the tests rely on:
//!
//! - [`volume`] (Windows only) opens the volume and streams the table into
//!   [`Records`] — one small summary per file, not the table itself. The table
//!   of a busy system volume is gigabytes; the summaries are what the tree is
//!   built from, and holding the raw table to build them would double the
//!   memory a scan needs for nothing. See ADR 0006.
//! - [`assemble`] turns records into a [`Tree`] — pure arithmetic over names
//!   and parents, with no volume in sight. It builds and is tested on every
//!   platform, including the Linux runner in CI.
//!
//! # Agreement with the walk
//!
//! Two scanners are allowed to exist only because they give the same answer
//! (a rule of the project, held by `tests/scanners_agree.rs`). Every choice
//! below that looks arbitrary is the walk's choice, copied: what a symbolic
//! link costs, that a directory's own entry costs nothing, that a named stream
//! is not counted, where the file identity comes from, which traits a
//! placeholder carries. Where the MFT sees *more* than the walk — `System
//! Volume Information`, another user's profile — it says so by having no
//! entry in [`Tree::skipped`] where the walk has one; that is the one
//! disagreement the rule permits, because it is the walk falling short.

mod assemble;
#[cfg(windows)]
mod volume;

use std::path::Path;

pub use assemble::assemble;

use crate::error::{Error, Result};
use crate::scanner::{Progress, Scanner};
use crate::size::Size;
use crate::tree::Tree;

/// The first record number that can belong to a user's file.
///
/// Records 0 to 15 are the volume's own metadata — `$MFT`, `$LogFile`,
/// `$Bitmap` and the rest — hung under the root directory with names that
/// start with `$`. The walk never sees them: the volume does not list them.
/// Record 5 is the exception, being the root directory itself.
pub const FIRST_USER_RECORD: u64 = 16;

/// The root directory's record number on every NTFS volume.
pub const ROOT_RECORD: u64 = 5;

/// The low 48 bits of a file reference are the record number; the high 16 are
/// the sequence number that says which use of that record is meant.
const RECORD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// The record number a file reference points at.
#[must_use]
pub const fn record_of(reference: u64) -> u64 {
    reference & RECORD_MASK
}

/// The sequence number a file reference expects its record to carry.
#[must_use]
pub const fn sequence_of(reference: u64) -> u16 {
    // The shift leaves 16 bits; the cast cannot lose any.
    #[allow(clippy::cast_possible_truncation, reason = "a 64-bit value shifted right by 48 fits in 16 bits")]
    let sequence = (reference >> 48) as u16;
    sequence
}

/// One name a file goes by: what it is called, and in which directory.
///
/// A file with two hard links has two of these, each in its own directory, over
/// one set of bytes. The walk sees them as two entries; so does the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    /// The directory this name is in, as a full file reference — sequence
    /// number included, so a name pointing at a directory that has since been
    /// deleted and its record reused is recognised as an orphan rather than
    /// hung under a stranger.
    pub parent: u64,
    pub name: String,
}

/// What one file's records said, gathered.
///
/// A file can spread over several records — a base record and extensions, when
/// it has too many attributes to fit in one — and the parts arrive in record
/// order, not together. This is where they meet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    /// Whether the base record has been seen and is in use. An extension that
    /// arrived for a base that never did describes nothing.
    pub in_use: bool,
    /// Which use of this record number this is.
    pub sequence: u16,
    pub directory: bool,
    /// Every name that is a hard link. The DOS short names NTFS generates
    /// alongside a long one are not here: they are aliases of a name, not
    /// names of their own, and the walk never lists them.
    pub names: Vec<Name>,
    /// The unnamed data stream's size, both ways. Named streams are not
    /// counted, because the walk's `AllocationSize` does not count them either.
    pub size: Size,
    /// The `FILE_ATTRIBUTE_*` bits from `$STANDARD_INFORMATION`.
    pub attributes: u32,
    /// When the file was last written, as a Windows `FILETIME`: hundreds of
    /// nanoseconds since 1601.
    pub modified: Option<u64>,
    /// The reparse tag, when the file is a reparse point.
    pub reparse_tag: Option<u32>,
    /// The size of the `WofCompressedData` stream, when the file has one.
    ///
    /// Windows' own file compression — `compact /exe`, and the whole of a
    /// CompactOS system volume — keeps a file's bytes in this named stream and
    /// leaves the unnamed one empty and sparse. See [`Record::occupied`].
    pub backing: Option<Size>,
}

/// `IO_REPARSE_TAG_WOF`: a file whose contents the Windows Overlay Filter
/// keeps compressed in a stream of its own.
const WOF: u32 = 0x8000_0017;

/// The name-surrogate bit of a reparse tag: set on symbolic links and
/// junctions, which name another place, and not on the reparse points that
/// only change how a file's own bytes are stored — a cloud placeholder, a
/// deduplicated file. `std` draws its `is_symlink` line on this bit, so the walk
/// stops at exactly these entries, and so must this scanner.
const NAME_SURROGATE: u32 = 0x2000_0000;

impl Record {
    /// Whether this is a link to somewhere else, which a scan counts as an
    /// entry and never follows.
    #[must_use]
    pub const fn is_link(&self) -> bool {
        match self.reparse_tag {
            Some(tag) => tag & NAME_SURROGATE != 0,
            None => false,
        }
    }

    /// Whether the Windows Overlay Filter keeps this file's bytes.
    #[must_use]
    pub fn is_overlaid(&self) -> bool {
        self.reparse_tag == Some(WOF)
    }

    /// The file's size as every process but this reader is shown it.
    ///
    /// For nearly every file that is the unnamed stream. For a WOF-compressed
    /// one the overlay filter answers instead: it reads as the unnamed stream's
    /// length and occupies what its compressed stream occupies — measured
    /// 2026-09-23 through `FileStandardInfo`, an 800 000-byte file reporting
    /// 53 248 allocated. Reading the unnamed stream's allocation here would
    /// count it as zero, and on a CompactOS volume that is gigabytes: the first
    /// whole-volume comparison against the walk came out 17 GB short.
    #[must_use]
    pub fn occupied(&self) -> Size {
        match (self.is_overlaid(), self.backing) {
            (true, Some(backing)) => Size {
                logical: self.size.logical,
                allocated: backing.allocated,
            },
            _ => self.size,
        }
    }

    /// Why the two sizes differ, as every process but this reader is shown it.
    ///
    /// The overlay filter hides the reparse and sparse bits of a file it keeps,
    /// just as the cloud filter does for a placeholder, so the walk never sees
    /// them. Sparseness is the filter's mechanism, not a fact about the file,
    /// and reporting it here would be one scanner explaining what the other
    /// cannot see.
    #[must_use]
    pub fn traits(&self) -> crate::traits::Traits {
        let traits = crate::platform::traits_from_attributes(self.attributes);
        if self.is_overlaid() {
            traits.without(crate::traits::Traits::SPARSE)
        } else {
            traits
        }
    }
}

/// Every file's record, by record number.
#[derive(Debug, Default)]
pub struct Records {
    by_number: Vec<Option<Record>>,
}

impl Records {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The record at `number`, created empty if nothing has arrived for it yet.
    ///
    /// # Panics
    ///
    /// If `number` does not fit in memory as an index, which no volume's record
    /// count comes near.
    pub fn entry(&mut self, number: u64) -> &mut Record {
        let index = usize::try_from(number).expect("a record number fits in usize");
        if self.by_number.len() <= index {
            self.by_number.resize_with(index + 1, || None);
        }
        self.by_number[index].get_or_insert_with(Record::default)
    }

    /// The record at `number`, when one is in use there.
    #[must_use]
    pub fn get(&self, number: u64) -> Option<&Record> {
        let index = usize::try_from(number).ok()?;
        self.by_number.get(index)?.as_ref().filter(|record| record.in_use)
    }

    /// Every record in use, with its number.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &Record)> {
        self.by_number
            .iter()
            .enumerate()
            .filter_map(|(number, record)| Some((number as u64, record.as_ref().filter(|record| record.in_use)?)))
    }
}

/// Reads a tree from the volume's Master File Table.
#[derive(Debug, Default, Clone, Copy)]
pub struct MftScanner;

impl MftScanner {
    /// A new scanner.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Whether the fast scanner can read `root`, and if not, what stands between.
///
/// The window offers "accelerate" on the answer. Offering it where elevation
/// would not help — a FAT stick, a network share — would be a UAC prompt that
/// buys nothing, which is exactly the prompt-by-ambush ADR 0001 rules out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Acceleration {
    /// This process may read the MFT under `root`, and scans do.
    Active,
    /// `root` is on NTFS; an elevated process could read it in seconds.
    NeedsElevation,
    /// Not an NTFS volume, or not Windows: the walk is the only way.
    Unsupported,
}

/// Where `root` stands with the fast scanner.
#[must_use]
pub fn acceleration(root: &Path) -> Acceleration {
    #[cfg(windows)]
    {
        if volume::locate(root).is_none() {
            Acceleration::Unsupported
        } else if crate::platform::is_elevated() {
            Acceleration::Active
        } else {
            Acceleration::NeedsElevation
        }
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Acceleration::Unsupported
    }
}

impl Scanner for MftScanner {
    fn name(&self) -> &'static str {
        "mft"
    }

    fn available(&self, root: &Path) -> bool {
        acceleration(root) == Acceleration::Active
    }

    fn scan(&self, root: &Path, progress: &Progress) -> Result<Tree> {
        let metadata = std::fs::metadata(root).map_err(|source| Error::Unreadable {
            path: root.to_path_buf(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(Error::NotADirectory(root.to_path_buf()));
        }

        #[cfg(windows)]
        {
            let located = volume::locate(root).ok_or_else(|| Error::Unavailable(format!("{} is not on an NTFS volume", root.display())))?;
            let records = volume::read(&located, progress)?;
            let mut tree = assemble(
                &records,
                located.root_record,
                root,
                located.serial,
                crate::platform::cluster_bytes_of(root),
                progress,
            )?;
            tree.record_scanner(self.name());
            Ok(tree)
        }
        #[cfg(not(windows))]
        {
            let _ = progress;
            Err(Error::Unavailable("the MFT exists only on Windows".to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_splits_into_its_record_and_its_sequence() {
        let reference = (7_u64 << 48) | 0x1234;
        assert_eq!(record_of(reference), 0x1234);
        assert_eq!(sequence_of(reference), 7);
    }

    #[test]
    fn only_a_name_surrogate_is_a_link() {
        let symlink = Record {
            reparse_tag: Some(0xA000_000C),
            ..Record::default()
        };
        let junction = Record {
            reparse_tag: Some(0xA000_0003),
            ..Record::default()
        };
        // A OneDrive placeholder is a reparse point too, and it is a file with
        // bytes of its own, not a way to somewhere else.
        let placeholder = Record {
            reparse_tag: Some(0x9000_601A),
            ..Record::default()
        };
        assert!(symlink.is_link());
        assert!(junction.is_link());
        assert!(!placeholder.is_link());
        assert!(!Record::default().is_link());
    }

    #[test]
    fn an_overlaid_file_occupies_its_compressed_stream() {
        let overlaid = Record {
            reparse_tag: Some(WOF),
            attributes: 0x0000_0620,
            size: Size {
                logical: 800_000,
                allocated: 0,
            },
            backing: Some(Size {
                logical: 51_000,
                allocated: 53_248,
            }),
            ..Record::default()
        };
        assert_eq!(
            overlaid.occupied(),
            Size {
                logical: 800_000,
                allocated: 53_248
            }
        );
        assert_eq!(
            overlaid.traits(),
            crate::traits::Traits::none(),
            "the filter hides the sparse bit from the walk"
        );

        // The same stream on a file the filter does not keep changes nothing.
        let plain = Record {
            reparse_tag: None,
            ..overlaid.clone()
        };
        assert_eq!(
            plain.occupied(),
            Size {
                logical: 800_000,
                allocated: 0
            }
        );
        assert!(plain.traits().has(crate::traits::Traits::SPARSE));
    }

    #[test]
    fn a_record_not_in_use_is_not_there() {
        let mut records = Records::new();
        records.entry(40).directory = true;
        assert!(records.get(40).is_none(), "an extension whose base never arrived describes nothing");
        records.entry(40).in_use = true;
        assert!(records.get(40).is_some());
        assert_eq!(records.iter().count(), 1);
    }

    #[test]
    #[cfg(not(windows))]
    fn there_is_nothing_to_accelerate_off_windows() {
        assert_eq!(acceleration(Path::new(".")), Acceleration::Unsupported);
        assert!(!MftScanner::new().available(Path::new(".")));
    }
}
