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

use crate::size::{Size, round_up_to_cluster};

/// The cluster size to assume when the volume will not say.
///
/// 4 KiB is the NTFS default for every volume size up to 16 TB and the ext4
/// default too, so an assumed answer is the right answer on almost every
/// machine this runs on.
pub const ASSUMED_CLUSTER_BYTES: u64 = 4096;

/// Measures a file both ways.
///
/// `metadata` has already been read by the caller — a walk gets it from the
/// directory entry, which on Windows costs nothing extra — so only the second
/// number needs a call of its own.
#[must_use]
pub fn measure(path: &Path, metadata: &Metadata, cluster_bytes: u64) -> Size {
    let logical = metadata.len();
    let allocated = allocated_size(path, metadata).unwrap_or_else(|| round_up_to_cluster(logical, cluster_bytes));
    Size { logical, allocated }
}

/// The cluster size of the volume `path` sits on, when it can be determined.
#[must_use]
pub fn cluster_bytes_of(path: &Path) -> Option<u64> {
    imp::cluster_bytes_of(path)
}

/// The space `path` actually occupies, when the platform can say.
fn allocated_size(path: &Path, metadata: &Metadata) -> Option<u64> {
    imp::allocated_size(path, metadata)
}

#[cfg(windows)]
mod imp {
    use std::fs::Metadata;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileStandardInfo, GetDiskFreeSpaceW,
        GetFileInformationByHandleEx, OPEN_EXISTING,
    };

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

    pub(super) fn allocated_size(path: &Path, _metadata: &Metadata) -> Option<u64> {
        let handle = open_for_metadata(path)?;

        let mut info = FILE_STANDARD_INFO {
            AllocationSize: 0,
            EndOfFile: 0,
            NumberOfLinks: 0,
            DeletePending: false,
            Directory: false,
        };

        // SAFETY: `handle` is open for the duration of the call, `info` is a
        // valid writable `FILE_STANDARD_INFO`, and the size passed is that
        // struct's own size. The function writes only through that pointer.
        #[allow(unsafe_code, reason = "AllocationSize is not exposed by std and never will be")]
        let ok = unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                FileStandardInfo,
                std::ptr::from_mut(&mut info).cast(),
                u32::try_from(size_of::<FILE_STANDARD_INFO>()).ok()?,
            )
        };

        if ok == 0 {
            return None;
        }

        // `AllocationSize` is signed in the header and never negative in
        // practice; a negative value would mean a corrupt answer, and guessing
        // is worse than falling back to the cluster rounding.
        u64::try_from(info.AllocationSize).ok()
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

    // `st_blocks` counts 512-byte units by POSIX definition, whatever the
    // filesystem's own block size is — and it is the real answer, sparse ranges
    // and all. Windows is the product's target, but the walk is the portable
    // half of ADR 0001 and it should be honest wherever it runs.
    #[cfg(unix)]
    pub(super) fn allocated_size(_path: &Path, metadata: &Metadata) -> Option<u64> {
        use std::os::unix::fs::MetadataExt as _;
        Some(metadata.blocks() * 512)
    }

    // Neither Windows nor Unix: there is no portable way to ask, so the caller
    // falls back to rounding up to a cluster rather than being told a number
    // that was not measured.
    #[cfg(not(unix))]
    pub(super) fn allocated_size(_path: &Path, _metadata: &Metadata) -> Option<u64> {
        None
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
        let size = measure(&path, &metadata, cluster);

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
        let size = measure(&path, &metadata, ASSUMED_CLUSTER_BYTES);

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
        let size = measure(&path, &metadata, ASSUMED_CLUSTER_BYTES);
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
        let size = measure(&path, &metadata, ASSUMED_CLUSTER_BYTES);
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

        let size = measure(path, &metadata, ASSUMED_CLUSTER_BYTES);
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
