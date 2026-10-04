//! The volume's change journal: every change on the volume, in order, from
//! any point it still holds — while midda was closed included.
//!
//! The second step of freshness (ADR 0009), and it needs what the MFT reader
//! needs: an elevated process and an NTFS volume. The journal is a ring of a
//! few dozen megabytes. A position saved with the index is good for as long as
//! the ring has not come round past it; when it has — or when the journal was
//! deleted and made again, which restarts its numbering — the journal says so
//! ([`Gap`]) and the folder is read in full instead.

use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_JOURNAL_DELETE_IN_PROGRESS, ERROR_JOURNAL_ENTRY_DELETED, ERROR_JOURNAL_NOT_ACTIVE, GENERIC_READ, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FileIdType, FileNameInfo, GetFileInformationByHandleEx, OPEN_EXISTING, OpenFileById,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V1, USN_JOURNAL_DATA_V0};

use super::JournalPosition;
use super::usn::{self, Record};

/// How much one read of the journal brings back: a few thousand records.
const READ_BYTES: usize = 256 << 10;

/// The journal of the volume a folder is on, opened.
#[derive(Debug)]
pub struct Journal {
    volume: Handle,
    /// Where the volume is mounted, which the volume-relative paths the
    /// journal resolves to are joined onto.
    mount: PathBuf,
    id: u64,
    /// The oldest record still in the ring, when the journal was opened.
    first: i64,
    /// Where the journal ended when it was opened, or the next record after
    /// the last one read.
    next: i64,
}

/// Why the journal cannot say what changed since a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Gap {
    /// The ring came round past it: the changes since were written over.
    #[error("the change journal no longer reaches back that far")]
    Overwritten,
    /// The journal was deleted and made again; its numbering started over.
    #[error("the change journal was made again since")]
    Recreated,
    /// The volume keeps no journal now.
    #[error("the volume keeps no change journal")]
    Inactive,
}

/// What reading the journal can fail on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    /// The journal cannot answer for the span asked about.
    #[error(transparent)]
    Gap(#[from] Gap),
    /// The volume or the journal could not be read.
    #[error("{0}")]
    Unreadable(String),
}

impl Journal {
    /// Opens the journal of the volume `root` is on.
    ///
    /// # Errors
    ///
    /// When `root` is not on NTFS, the process may not open the volume — it is
    /// not elevated — or the volume keeps no journal.
    pub fn open(root: &Path) -> Result<Self, Failure> {
        let located = crate::mft::volume::locate(root).ok_or_else(|| Failure::Unreadable(format!("{} is not on an NTFS volume", root.display())))?;
        let wide: Vec<u16> = located.device.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; the null
        // pointers are the documented "no security attributes, no template".
        #[allow(unsafe_code, reason = "the change journal is read through the volume device")]
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(Failure::Unreadable(format!("could not open the volume: {}", std::io::Error::last_os_error())));
        }
        let volume = Handle(handle);

        // SAFETY: an all-zero journal description is a valid value of a
        // plain struct of integers.
        #[allow(unsafe_code, reason = "a plain C struct the call fills in")]
        let mut data: USN_JOURNAL_DATA_V0 = unsafe { std::mem::zeroed() };
        let mut returned = 0_u32;
        // SAFETY: `volume` is open; the output is a `USN_JOURNAL_DATA_V0` of
        // the size passed; no input, no overlapped I/O.
        #[allow(unsafe_code, reason = "querying the journal is a device control")]
        let ok = unsafe {
            DeviceIoControl(
                volume.0,
                FSCTL_QUERY_USN_JOURNAL,
                std::ptr::null(),
                0,
                std::ptr::from_mut(&mut data).cast(),
                u32::try_from(size_of::<USN_JOURNAL_DATA_V0>()).unwrap_or(0),
                &raw mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(failure("could not query the change journal"));
        }
        Ok(Self {
            volume,
            mount: located.mount,
            id: data.UsnJournalID,
            first: data.FirstUsn,
            next: data.NextUsn,
        })
    }

    /// Where the journal is up to: the next record to be read.
    #[must_use]
    pub const fn position(&self) -> JournalPosition {
        JournalPosition {
            journal: self.id,
            next: self.next,
        }
    }

    /// Whether the journal still holds every change since `at`.
    ///
    /// # Errors
    ///
    /// Which [`Gap`] stands between `at` and now.
    pub const fn reaches(&self, at: JournalPosition) -> Result<(), Gap> {
        if at.journal != self.id {
            Err(Gap::Recreated)
        } else if at.next < self.first {
            Err(Gap::Overwritten)
        } else {
            Ok(())
        }
    }

    /// Reads every record from `from` to the end of the journal, and moves
    /// [`Journal::position`] past them.
    ///
    /// # Errors
    ///
    /// A [`Gap`] when the journal no longer holds `from`, or was made again;
    /// otherwise what the volume refused.
    pub fn read_from(&mut self, from: JournalPosition) -> Result<Vec<Record>, Failure> {
        self.reaches(from)?;
        let mut records = Vec::new();
        let mut start = from.next;
        // `u64`s, so the buffer is aligned for the records in it.
        let mut buffer = vec![0_u64; READ_BYTES / 8];
        loop {
            let request = READ_USN_JOURNAL_DATA_V1 {
                StartUsn: start,
                ReasonMask: usn::WATCHED,
                ReturnOnlyOnClose: 0,
                Timeout: 0,
                BytesToWaitFor: 0,
                UsnJournalID: self.id,
                MinMajorVersion: 2,
                MaxMajorVersion: 3,
            };
            let mut returned = 0_u32;
            // SAFETY: `volume` is open; the input is a `READ_USN_JOURNAL_DATA_V1`
            // of the size passed and the output is `buffer`, writable for
            // its length in bytes; no overlapped I/O.
            #[allow(unsafe_code, reason = "reading the journal is a device control")]
            let ok = unsafe {
                DeviceIoControl(
                    self.volume.0,
                    FSCTL_READ_USN_JOURNAL,
                    std::ptr::from_ref(&request).cast(),
                    u32::try_from(size_of::<READ_USN_JOURNAL_DATA_V1>()).unwrap_or(0),
                    buffer.as_mut_ptr().cast(),
                    u32::try_from(READ_BYTES).unwrap_or(u32::MAX),
                    &raw mut returned,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(failure("could not read the change journal"));
            }
            let filled = usize::try_from(returned).unwrap_or(0).min(READ_BYTES);
            // SAFETY: `buffer` holds `READ_BYTES` initialised bytes as `u64`s;
            // `filled` is no more than that.
            #[allow(unsafe_code, reason = "the records are bytes in an aligned buffer")]
            let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), filled) };
            let (next, mut read) = usn::parse(bytes);
            let next = next.unwrap_or(start);
            records.append(&mut read);
            // Nothing past the leading USN means the end was reached; a read
            // that did not move is treated the same, rather than asked again
            // for ever.
            if filled <= 8 || next <= start {
                self.next = next.max(start);
                return Ok(records);
            }
            start = next;
        }
    }

    /// Where each record's file is — was, for a deleted one — as a path.
    ///
    /// The folder a record names is looked up by its id as it is now, and the
    /// record's name joined on: so a folder moved since gives its new place,
    /// and a file renamed away gives the name it no longer has, which is the
    /// path to look at for its going. A record whose folder is gone too gives
    /// the file's own path if the file survived elsewhere, and nothing
    /// otherwise — a folder that went is reported by its own record.
    #[must_use]
    pub fn paths(&self, records: &[Record]) -> Vec<PathBuf> {
        let mut folders: HashMap<u64, Option<PathBuf>> = HashMap::new();
        let mut paths = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for record in records {
            let folder = folders.entry(record.parent).or_insert_with(|| self.path_of(record.parent)).clone();
            let path = match folder {
                Some(folder) => Some(folder.join(&record.name)),
                None => self.path_of(record.file),
            };
            if let Some(path) = path
                && seen.insert(path.clone())
            {
                paths.push(path);
            }
        }
        paths
    }

    /// The path of the file or folder with this reference, now.
    fn path_of(&self, reference: u64) -> Option<PathBuf> {
        let descriptor = FILE_ID_DESCRIPTOR {
            dwSize: u32::try_from(size_of::<FILE_ID_DESCRIPTOR>()).ok()?,
            Type: FileIdType,
            Anonymous: FILE_ID_DESCRIPTOR_0 {
                FileId: i64::from_le_bytes(reference.to_le_bytes()),
            },
        };
        // SAFETY: `descriptor` is a valid `FILE_ID_DESCRIPTOR` that outlives
        // the call; the volume handle is open; zero access asks for no right
        // to the contents, and `OPEN_REPARSE_POINT` opens a link itself.
        #[allow(unsafe_code, reason = "opening a file by its id is a Win32 call")]
        let handle = unsafe {
            OpenFileById(
                self.volume.0,
                &raw const descriptor,
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let handle = Handle(handle);
        // `FILE_NAME_INFO`: a length in bytes, then the name. `u32`s keep the
        // buffer aligned for the length.
        let mut buffer = vec![0_u32; 2 + 32_768 / 2];
        // SAFETY: `handle` is open and `buffer` is writable for the size
        // passed.
        #[allow(unsafe_code, reason = "a file's path from its handle is a Win32 call")]
        let ok = unsafe { GetFileInformationByHandleEx(handle.0, FileNameInfo, buffer.as_mut_ptr().cast(), u32::try_from(buffer.len() * 4).ok()?) };
        if ok == 0 {
            return None;
        }
        let length = usize::try_from(buffer[0]).ok()? / 2;
        let bytes: Vec<u8> = buffer[1..].iter().flat_map(|word| word.to_le_bytes()).collect();
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).take(length).collect();
        let relative = String::from_utf16_lossy(&units);
        // A path from the volume's root: `\` alone for the root itself.
        Some(
            relative
                .split('\\')
                .filter(|part| !part.is_empty())
                .fold(self.mount.clone(), |path, part| path.join(part)),
        )
    }
}

/// The last error as a [`Failure`], with the journal's own codes as gaps.
fn failure(doing: &str) -> Failure {
    // SAFETY: reads this thread's last error.
    #[allow(unsafe_code, reason = "the reason is only in the thread's last error")]
    let error = unsafe { GetLastError() };
    match error {
        ERROR_JOURNAL_ENTRY_DELETED => Failure::Gap(Gap::Overwritten),
        ERROR_JOURNAL_NOT_ACTIVE | ERROR_JOURNAL_DELETE_IN_PROGRESS => Failure::Gap(Gap::Inactive),
        _ => Failure::Unreadable(format!(
            "{doing}: {}",
            std::io::Error::from_raw_os_error(i32::try_from(error).unwrap_or(i32::MAX))
        )),
    }
}

/// A handle that closes itself.
#[derive(Debug)]
struct Handle(HANDLE);

// SAFETY: a volume or file handle may be used from any thread; a `Journal` is
// used by one thread at a time, through `&mut self` or `&self` alike.
#[allow(unsafe_code, reason = "a Win32 handle is not tied to the thread that opened it")]
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful open and is closed once.
        #[allow(unsafe_code, reason = "a Win32 handle is closed by hand")]
        unsafe {
            CloseHandle(self.0);
        }
    }
}
