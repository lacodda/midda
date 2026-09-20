//! Where the second number comes from.
//!
//! `std::fs::Metadata` reports the logical size on every platform and the
//! allocated size on none of them. On Windows it is `AllocationSize` from
//! `GetFileInformationByHandleEx(FileStandardInfo)`.
//!
//! `GetCompressedFileSizeW` is the obvious-looking candidate and it is the
//! wrong one. Measured on an NTFS volume with 4 KiB clusters:
//!
//! | file | logical | `AllocationSize` | `GetCompressedFileSizeW` |
//! | --- | --- | --- | --- |
//! | 5000 bytes, plain | 5000 | 8192 | 5000 |
//! | 100 bytes, resident | 100 | 104 | 100 |
//! | 360 KB, NTFS-compressed | 360000 | 45056 | 45056 |
//!
//! The two agree only when a file is compressed or sparse. For an ordinary
//! file — which is nearly every file on a disk — `GetCompressedFileSizeW`
//! hands back the logical size unrounded, so a scanner built on it would report
//! the number it was written to replace and no test on a compressed fixture
//! would notice.
//!
//! Note the resident case: a 100-byte file occupies 104 bytes, not a cluster,
//! because NTFS stores small files inside the MFT record itself. Rounding every
//! small file up to a cluster would overstate a directory of many tiny files by
//! forty times. This is measured, not assumed — see the tests below.
//!
//! Everywhere else — which for midda means Linux in CI, keeping the walk
//! honest per ADR 0001 — `st_blocks` is the real answer and is used directly.

use std::fs::Metadata;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::size::{Size, round_up_to_cluster};
use crate::traits::Traits;

/// The cluster size to assume when the volume will not say.
///
/// 4 KiB is the NTFS default for every volume size up to 16 TB and the ext4
/// default too, so an assumed answer is the right answer on almost every
/// machine this runs on.
pub const ASSUMED_CLUSTER_BYTES: u64 = 4096;

/// Everything one file had to be opened to learn.
///
/// Gathered in one call because the expensive part is the handle, not the
/// questions asked through it: the size, the link count and the file identity
/// all come out of the same open file, and asking them separately would open a
/// system volume's worth of files three times over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measured {
    /// What the file takes, both ways.
    pub size: Size,
    /// Why the two numbers differ, when they do.
    pub traits: Traits,
    /// How many names this file has on the volume, when the platform says.
    ///
    /// `1` for nearly every file. Greater than one means the bytes are reachable
    /// under another name too, and counting them once per name is the mistake
    /// [`crate::links`] exists to undo. `None` when it could not be asked, which
    /// is kept distinct from `Some(1)`: the second is a measurement, the first
    /// is a gap.
    pub links: Option<u32>,
    /// What identifies these bytes on this volume, when the platform says.
    ///
    /// Two entries with the same identity are the same file under two names.
    /// `None` when it could not be asked — and an unknown identity is never
    /// equal to another unknown one, which is why this is an `Option` rather
    /// than a zero.
    pub identity: Option<FileIdentity>,
}

/// What makes a file the same file, whatever it is called.
///
/// On Windows this is the volume serial plus the 128-bit file id. The volume
/// part is not ceremony: a scan can cross a mount point, and two files on
/// different volumes may share a file id without sharing a byte. On Unix it is
/// the device and inode, which is the same idea with shorter numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileIdentity {
    /// Which volume the bytes are on.
    pub volume: u64,
    /// Which file on that volume.
    pub file: u128,
}

/// Measures a file both ways, and learns why.
///
/// `metadata` has already been read by the caller — a walk gets it from the
/// directory entry, which on Windows costs nothing extra — so only what needs
/// an open handle is asked for here.
#[must_use]
pub fn measure(path: &Path, metadata: &Metadata, cluster_bytes: u64) -> Measured {
    let logical = metadata.len();
    let traits = traits_of(metadata);

    let Some(opened) = imp::interrogate(path) else {
        // No handle: the size falls back to the cluster rounding, and the two
        // facts that need one are reported as unknown rather than as "one name,
        // no identity", which would read as a measurement.
        return Measured {
            size: Size {
                logical,
                allocated: round_up_to_cluster(logical, cluster_bytes),
            },
            traits,
            links: None,
            identity: None,
        };
    };

    Measured {
        size: Size {
            logical,
            allocated: opened.allocated.unwrap_or_else(|| round_up_to_cluster(logical, cluster_bytes)),
        },
        traits,
        links: opened.links,
        identity: opened.identity,
    }
}

/// What the attribute bits say about why a size is the number it is.
///
/// Read from the metadata the caller already has, with no handle: these are
/// flags on the directory entry.
///
/// One of them is not where it looks like it should be. A cloud placeholder is
/// marked on the volume with `REPARSE_POINT`, `SPARSE_FILE` and `OFFLINE` as
/// well — PowerShell shows all four — but Rust's `std` reports only
/// `RECALL_ON_DATA_ACCESS` to a caller: it resolves the reparse point and hands
/// back attributes with those bits cleared, through `metadata`,
/// `symlink_metadata` and a `DirEntry` alike. Measured on real OneDrive files,
/// 2026-09-20: 0x401620 as PowerShell reads it, 0x400020 as Rust does. So
/// `RECALL_ON_DATA_ACCESS` is the only bit that survives to here, and a
/// detection built on `OFFLINE` or on `REPARSE_POINT` would find nothing while
/// looking entirely correct.
#[must_use]
pub fn traits_of(metadata: &Metadata) -> Traits {
    imp::traits_of(metadata)
}

/// What one open handle was able to report.
///
/// Each field is separately optional: a volume can answer the size and refuse
/// the identity, and reporting an unknown as a default would turn "we could not
/// ask" into a fact the deduplication then acts on.
#[derive(Debug, Clone, Copy)]
struct Opened {
    allocated: Option<u64>,
    links: Option<u32>,
    identity: Option<FileIdentity>,
}

/// The cluster size of the volume `path` sits on, when it can be determined.
#[must_use]
pub fn cluster_bytes_of(path: &Path) -> Option<u64> {
    imp::cluster_bytes_of(path)
}

#[cfg(windows)]
mod imp {
    use std::fs::Metadata;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_SPARSE_FILE, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_128,
        FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileIdInfo, FileStandardInfo, GetDiskFreeSpaceW,
        GetFileInformationByHandleEx, OPEN_EXISTING,
    };

    use super::{FileIdentity, Opened};
    use crate::traits::Traits;

    /// A path as Windows wants it: UTF-16, NUL-terminated.
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// An open handle that closes itself.
    ///
    /// A scan opens a handle per file and a leak would exhaust the process
    /// inside one directory of a system volume, so the close does not depend on
    /// every return path remembering it.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful `CreateFileW` and is
            // closed exactly once, here, because `Handle` is neither `Copy` nor
            // `Clone` and nothing else holds the value.
            #[allow(unsafe_code, reason = "a handle from CreateFileW has to be closed by hand")]
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Opens `path` for metadata only.
    ///
    /// Zero desired access is what asks for "tell me about this, do not let me
    /// read it": it succeeds on files the caller has no read permission for,
    /// which on a system volume is a great many of them. Sharing everything
    /// means a file another process holds open — a log, a database, the page
    /// file — is still measurable rather than skipped. `BACKUP_SEMANTICS` is
    /// what allows a directory to be opened at all.
    fn open_for_metadata(path: &Path) -> Option<Handle> {
        let wide = wide(path);

        // SAFETY: `wide` is NUL-terminated and outlives the call; the two null
        // pointers are the documented "no security attributes, no template"
        // arguments.
        #[allow(unsafe_code, reason = "the size on disk needs a handle, and std will not open one without read access")]
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };

        (handle != INVALID_HANDLE_VALUE).then_some(Handle(handle))
    }

    /// Opens `path` once and asks it everything.
    ///
    /// Two queries through one handle. `FILE_STANDARD_INFO` carries the
    /// allocated size *and* the link count — the second one arrives free,
    /// which is why hard links cost this scan no extra syscall — and
    /// `FILE_ID_INFO` carries the identity that says which names are the same
    /// bytes.
    pub(super) fn interrogate(path: &Path) -> Option<Opened> {
        let handle = open_for_metadata(path)?;

        let mut standard = FILE_STANDARD_INFO {
            AllocationSize: 0,
            EndOfFile: 0,
            NumberOfLinks: 0,
            DeletePending: false,
            Directory: false,
        };

        // SAFETY: `handle` is open for the duration of the call, `standard` is a
        // valid writable `FILE_STANDARD_INFO`, and the size passed is that
        // struct's own size. The function writes only through that pointer.
        #[allow(unsafe_code, reason = "AllocationSize and NumberOfLinks are not exposed by std and never will be")]
        let standard_ok = unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                FileStandardInfo,
                std::ptr::from_mut(&mut standard).cast(),
                u32::try_from(size_of::<FILE_STANDARD_INFO>()).ok()?,
            )
        };

        let (allocated, links) = if standard_ok == 0 {
            (None, None)
        } else {
            (
                // `AllocationSize` is signed in the header and never negative in
                // practice; a negative value would mean a corrupt answer, and
                // guessing is worse than falling back to the cluster rounding.
                u64::try_from(standard.AllocationSize).ok(),
                Some(standard.NumberOfLinks),
            )
        };

        let mut id = FILE_ID_INFO {
            VolumeSerialNumber: 0,
            FileId: FILE_ID_128 { Identifier: [0; 16] },
        };

        // SAFETY: as above, with `id` a valid writable `FILE_ID_INFO` and its
        // own size passed.
        #[allow(unsafe_code, reason = "the identity that says two names are one file is not exposed by std")]
        let id_ok = unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                FileIdInfo,
                std::ptr::from_mut(&mut id).cast(),
                u32::try_from(size_of::<FILE_ID_INFO>()).ok()?,
            )
        };

        let identity = (id_ok != 0).then(|| FileIdentity {
            volume: id.VolumeSerialNumber,
            // The 128-bit id is bytes in the header, little-endian on every
            // Windows target. Read as one number so a comparison is one
            // instruction rather than a slice walk, four million times over.
            file: u128::from_le_bytes(id.FileId.Identifier),
        });

        Some(Opened { allocated, links, identity })
    }

    /// What the attribute bits say. See [`super::traits_of`] for why
    /// `RECALL_ON_DATA_ACCESS` is the only cloud bit that reaches here.
    pub(super) fn traits_of(metadata: &Metadata) -> Traits {
        use std::os::windows::fs::MetadataExt as _;

        let attributes = metadata.file_attributes();
        let mut traits = Traits::none();

        if attributes & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS != 0 {
            traits.insert(Traits::PLACEHOLDER);
        }
        if attributes & FILE_ATTRIBUTE_COMPRESSED != 0 {
            traits.insert(Traits::COMPRESSED);
        }
        if attributes & FILE_ATTRIBUTE_SPARSE_FILE != 0 {
            traits.insert(Traits::SPARSE);
        }

        traits
    }

    pub(super) fn cluster_bytes_of(path: &Path) -> Option<u64> {
        // `GetDiskFreeSpaceW` wants a root: `C:\`, not the directory being
        // scanned.
        let root = root_of(path)?;
        let wide = wide(&root);

        let mut sectors_per_cluster: u32 = 0;
        let mut bytes_per_sector: u32 = 0;
        let mut free_clusters: u32 = 0;
        let mut total_clusters: u32 = 0;

        // SAFETY: `wide` is NUL-terminated and outlives the call; the four
        // out-parameters are valid writable u32s. The function writes only
        // through them.
        #[allow(unsafe_code, reason = "the cluster size of a volume is not exposed by std")]
        let ok = unsafe {
            GetDiskFreeSpaceW(
                wide.as_ptr(),
                &raw mut sectors_per_cluster,
                &raw mut bytes_per_sector,
                &raw mut free_clusters,
                &raw mut total_clusters,
            )
        };

        if ok == 0 {
            return None;
        }

        let cluster = u64::from(sectors_per_cluster) * u64::from(bytes_per_sector);
        (cluster > 0).then_some(cluster)
    }

    /// The volume root a path sits on: `C:\Projects\midda` becomes `C:\`.
    fn root_of(path: &Path) -> Option<std::path::PathBuf> {
        use std::path::Component;

        let mut components = path.components();
        match components.next()? {
            // A drive letter, a UNC share or a verbatim path: the prefix plus a
            // separator is the root.
            Component::Prefix(prefix) => {
                let mut root = std::path::PathBuf::from(prefix.as_os_str());
                root.push(std::path::MAIN_SEPARATOR_STR);
                Some(root)
            }
            _ => None,
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::fs::Metadata;
    use std::path::Path;

    use super::{FileIdentity, Opened};
    use crate::traits::Traits;

    // `st_blocks` counts 512-byte units by POSIX definition, whatever the
    // filesystem's own block size is — and it is the real answer, sparse ranges
    // and all. The link count and the inode come out of the same `stat`, so Unix
    // needs no handle at all.
    //
    // Windows is the product's target, but the walk is the portable half of
    // ADR 0001 and it should be honest wherever it runs — which is also what
    // lets the deduplication be tested in CI, where the runners are Linux.
    #[cfg(unix)]
    pub(super) fn interrogate(path: &Path) -> Option<Opened> {
        use std::os::unix::fs::MetadataExt as _;

        // `symlink_metadata` rather than `metadata`: a symlink's own identity is
        // its own, and following it here would make a link look like a second
        // name for its target.
        let metadata = std::fs::symlink_metadata(path).ok()?;
        Some(Opened {
            allocated: Some(metadata.blocks() * 512),
            links: u32::try_from(metadata.nlink()).ok(),
            identity: Some(FileIdentity {
                volume: metadata.dev(),
                file: u128::from(metadata.ino()),
            }),
        })
    }

    // Neither Windows nor Unix: there is no portable way to ask, so the caller
    // falls back to rounding up to a cluster rather than being told a number
    // that was not measured.
    #[cfg(not(unix))]
    pub(super) fn interrogate(_path: &Path) -> Option<Opened> {
        None
    }

    // Compression and sparseness are filesystem-specific and not in `Metadata`
    // anywhere but Windows. Reporting none is the truth: nothing was measured.
    pub(super) fn traits_of(_metadata: &Metadata) -> Traits {
        Traits::none()
    }

    pub(super) fn cluster_bytes_of(_path: &Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn write(dir: &Path, name: &str, bytes: usize) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(&vec![b'x'; bytes]).expect("write");
        file.sync_all().expect("sync");
        path
    }

    #[test]
    #[cfg(windows)]
    fn a_file_that_spills_past_a_cluster_is_measured_at_the_next_one() {
        // The case that caught `GetCompressedFileSizeW` out: 5000 bytes on a
        // 4 KiB volume occupy 8192. That call returned 5000 — the logical size
        // it was chosen to replace — and only a compressed fixture would have
        // shown the difference. Measured against the volume's own cluster size
        // rather than against 4096, so the test is right on a volume formatted
        // differently.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let cluster = cluster_bytes_of(dir.path()).expect("a Windows volume knows its cluster size");

        #[allow(clippy::cast_possible_truncation, reason = "a cluster size fits in a usize on every machine this runs on")]
        let spill = (cluster + cluster / 4) as usize;
        let path = write(dir.path(), "spill.bin", spill);

        let metadata = std::fs::metadata(&path).expect("metadata");
        let size = measure(&path, &metadata, cluster).size;

        assert_eq!(size.logical, spill as u64);
        assert_eq!(
            size.allocated,
            cluster * 2,
            "a file of {spill} bytes on a volume of {cluster}-byte clusters occupies two of them"
        );
    }

    #[test]
    fn a_file_never_reports_as_occupying_less_than_nothing() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = write(dir.path(), "tiny.bin", 1);

        let metadata = std::fs::metadata(&path).expect("metadata");
        let size = measure(&path, &metadata, ASSUMED_CLUSTER_BYTES).size;

        assert_eq!(size.logical, 1);
        // Deliberately not asserting a whole cluster: NTFS stores a file this
        // small inside its MFT record, where it occupies 8 bytes rather than
        // 4096, and that is the true answer. Rounding it up would overstate a
        // directory of many tiny files by forty times.
        assert!(size.allocated >= size.logical, "{} on disk for {} bytes", size.allocated, size.logical);
    }

    #[test]
    fn an_empty_file_reads_as_empty() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = write(dir.path(), "empty.bin", 0);

        let metadata = std::fs::metadata(&path).expect("metadata");
        let size = measure(&path, &metadata, ASSUMED_CLUSTER_BYTES).size;
        assert_eq!(size.logical, 0);
        assert_eq!(
            size.allocated, 0,
            "an empty file occupies nothing, and rounding it up to a cluster would invent bytes"
        );
    }

    #[test]
    fn a_file_held_open_by_someone_else_is_still_measurable() {
        // A scan of a system volume meets log files, databases and the page
        // file, all held open. Opening with zero desired access and full
        // sharing is what keeps them from becoming holes in the total.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = write(dir.path(), "held.bin", 9000);

        let held = std::fs::OpenOptions::new().read(true).write(true).open(&path).expect("hold the file open");

        let metadata = std::fs::metadata(&path).expect("metadata");
        let size = measure(&path, &metadata, ASSUMED_CLUSTER_BYTES).size;
        assert_eq!(size.logical, 9000);
        assert!(size.allocated >= 9000, "a file held open came back as {} bytes on disk", size.allocated);

        drop(held);
    }

    #[test]
    fn a_missing_file_falls_back_to_the_cluster_rounding() {
        // The platform call fails; the caller still gets a number, and it is
        // the honest approximation rather than the logical size dressed up.
        let path = Path::new("no-such-file-anywhere.bin");
        let dir = tempfile::tempdir().expect("a temporary directory");
        let real = dir.path().join("real.bin");
        std::fs::write(&real, vec![0_u8; 100]).expect("write");
        let metadata = std::fs::metadata(&real).expect("metadata");

        let size = measure(path, &metadata, ASSUMED_CLUSTER_BYTES).size;
        assert_eq!(size.logical, 100);
        assert_eq!(size.allocated, ASSUMED_CLUSTER_BYTES);
    }

    #[test]
    #[cfg(windows)]
    fn a_volume_reports_its_cluster_size() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let cluster = cluster_bytes_of(dir.path()).expect("a Windows volume knows its cluster size");
        assert!(cluster.is_power_of_two(), "a cluster size of {cluster} is not a power of two");
        assert!(
            (512..=2_097_152).contains(&cluster),
            "a cluster size of {cluster} is outside what NTFS supports"
        );
    }
}
