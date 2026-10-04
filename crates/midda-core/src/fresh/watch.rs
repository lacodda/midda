//! A watcher on the scanned folder: what changes under it while midda is
//! open, with no rights beyond reading it.
//!
//! The first step of freshness (ADR 0009). `ReadDirectoryChangesW` on the
//! folder, subtree included, on a thread of its own; what it reports is kept
//! as a set of paths until [`Watcher::drain`] takes them, and the index looks
//! at each one again ([`super::Index::observe`]).
//!
//! The watcher can lose track. When changes come faster than the thread
//! empties the buffer, Windows drops them and says only that it did. That is
//! reported, not papered over: a tree that missed a change is a tree to read
//! again, and the window says so.

use std::collections::HashSet;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, ERROR_IO_INCOMPLETE, ERROR_NOTIFY_ENUM_DIR, ERROR_OPERATION_ABORTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    ReadDirectoryChangesW,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects};

use crate::error::{Error, Result};

/// How much the watcher can hold between two reads: a megabyte is some ten
/// thousand changes. A network share accepts no more than 64 KiB, and gets
/// that.
const BUFFER_BYTES: usize = 1 << 20;
const NETWORK_BUFFER_BYTES: usize = 64 << 10;

/// What is watched: names appearing, going and changing, and sizes, write
/// times and attribute bits — compression and sparseness are attribute bits.
const WATCHED: u32 =
    FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_ATTRIBUTES | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE;

/// A watch on one folder, until it is dropped.
#[derive(Debug)]
pub struct Watcher {
    stop: Event,
    thread: Option<JoinHandle<()>>,
    pending: Arc<Mutex<Pending>>,
}

/// What the watcher has seen since it was last drained.
#[derive(Debug, Default)]
struct Pending {
    paths: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
    lost: bool,
    ended: Option<String>,
}

/// What [`Watcher::drain`] hands over.
#[derive(Debug, Default)]
pub struct Drained {
    /// The paths reported, each once, in the order first reported.
    pub paths: Vec<PathBuf>,
    /// Whether changes were dropped because they came faster than they were
    /// read. The tree may now be wrong anywhere under the folder.
    pub lost: bool,
    /// Why the watch stopped, when it did: the folder was deleted, or the
    /// volume went away.
    pub ended: Option<String>,
}

impl Watcher {
    /// Starts watching `root` and everything under it.
    ///
    /// Returns once the watch is in place, not merely asked for: a change made
    /// after this returns is reported. Windows records changes only from the
    /// first read on, and a thread that had not reached it yet would let the
    /// first changes through unseen — on a slow machine, measurably so.
    ///
    /// # Errors
    ///
    /// When the folder cannot be opened for watching.
    pub fn start(root: &Path) -> Result<Self> {
        let directory = open(root)?;
        let stop = Event::new()?;
        let wake = Event::new()?;
        let pending = Arc::new(Mutex::new(Pending::default()));

        let (armed, ready) = std::sync::mpsc::channel();
        let thread = {
            let signal = Stop(stop.0);
            let pending = Arc::clone(&pending);
            let root = root.to_path_buf();
            std::thread::Builder::new()
                .name("midda-watch".into())
                .spawn(move || watch(&directory, &wake, &signal, &root, &pending, Some(armed)))
                .map_err(|error| Error::Unavailable(format!("could not start watching: {error}")))?
        };
        match ready.recv() {
            Ok(Ok(())) => {}
            Ok(Err(why)) => return Err(Error::Unavailable(format!("could not watch {}: {why}", root.display()))),
            Err(_) => return Err(Error::Unavailable("the watcher stopped before it started".into())),
        }

        Ok(Self {
            stop,
            thread: Some(thread),
            pending,
        })
    }

    /// Takes what has been seen since the last call.
    #[must_use]
    pub fn drain(&self) -> Drained {
        let Ok(mut pending) = self.pending.lock() else {
            return Drained {
                ended: Some("the watcher stopped".into()),
                ..Drained::default()
            };
        };
        let taken = std::mem::take(&mut *pending);
        // An ended watch stays ended: the next drain says so too.
        pending.ended.clone_from(&taken.ended);
        Drained {
            paths: taken.paths,
            lost: taken.lost,
            ended: taken.ended,
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: `stop` is an event this watcher created and still owns.
        #[allow(unsafe_code, reason = "signalling an event is a Win32 call")]
        unsafe {
            SetEvent(self.stop.0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// An event handle that closes itself.
#[derive(Debug)]
struct Event(HANDLE);

// SAFETY: an event handle may be signalled and waited on from any thread.
#[allow(unsafe_code, reason = "a Win32 event handle is not tied to the thread that made it")]
unsafe impl Send for Event {}

impl Event {
    fn new() -> Result<Self> {
        // SAFETY: null attributes and name are the documented defaults; a
        // manual-reset event, initially unsignalled.
        #[allow(unsafe_code, reason = "events are a Win32 call")]
        let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if handle.is_null() {
            return Err(Error::Unavailable(format!("could not create an event: {}", std::io::Error::last_os_error())));
        }
        Ok(Self(handle))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateEventW` and is closed once.
        #[allow(unsafe_code, reason = "a Win32 handle is closed by hand")]
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// The stop event as the watching thread sees it: borrowed, not owned — the
/// [`Watcher`] closes it after the thread has been joined.
struct Stop(HANDLE);

// SAFETY: as for `Event`; the handle outlives the thread, which is joined
// before the owner closes it.
#[allow(unsafe_code, reason = "a Win32 event handle is not tied to the thread that made it")]
unsafe impl Send for Stop {}

/// A directory handle opened for watching, closed when dropped.
struct Directory(HANDLE);

// SAFETY: a file handle may be used from another thread; this one is used
// only by the watching thread once it is moved there.
#[allow(unsafe_code, reason = "a Win32 file handle is not tied to the thread that opened it")]
unsafe impl Send for Directory {}

impl Drop for Directory {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateFileW` and is closed once.
        #[allow(unsafe_code, reason = "a Win32 handle is closed by hand")]
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn open(root: &Path) -> Result<Directory> {
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call; the null
    // pointers are the documented "no security attributes, no template".
    // Sharing everything means the watch never stops anyone else from
    // renaming or deleting what is under it.
    #[allow(unsafe_code, reason = "a directory handle for watching needs flags std does not offer")]
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(Error::Unavailable(format!(
            "could not watch {}: {}",
            root.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(Directory(handle))
}

/// The watching thread: read changes until told to stop or the watch ends.
fn watch(
    directory: &Directory,
    wake: &Event,
    stop: &Stop,
    root: &Path,
    pending: &Mutex<Pending>,
    mut armed: Option<std::sync::mpsc::Sender<std::result::Result<(), String>>>,
) {
    let mut size = BUFFER_BYTES;
    // `u64`s, so the buffer is aligned for the records in it.
    let mut buffer = vec![0_u64; size / 8];
    loop {
        // SAFETY: an all-zero `OVERLAPPED` is the documented starting value.
        #[allow(unsafe_code, reason = "a plain C struct the call fills in")]
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = wake.0;
        // SAFETY: `wake` is a live event.
        #[allow(unsafe_code, reason = "resetting an event is a Win32 call")]
        unsafe {
            ResetEvent(wake.0);
        }
        // SAFETY: `directory` is open for listing with overlapped I/O;
        // `buffer` and `overlapped` stay alive and unmoved until the read
        // completes or is cancelled and waited out below.
        #[allow(unsafe_code, reason = "watching a directory is a Win32 call")]
        let started = unsafe {
            ReadDirectoryChangesW(
                directory.0,
                buffer.as_mut_ptr().cast(),
                u32::try_from(size).unwrap_or(u32::MAX),
                1,
                WATCHED,
                std::ptr::null_mut(),
                &raw mut overlapped,
                None,
            )
        };
        if started == 0 {
            // SAFETY: reads this thread's last error.
            #[allow(unsafe_code, reason = "the reason is only in the thread's last error")]
            let error = unsafe { GetLastError() };
            if error == ERROR_INVALID_PARAMETER && size > NETWORK_BUFFER_BYTES {
                // A network share: the same watch with the buffer it allows.
                size = NETWORK_BUFFER_BYTES;
                buffer.truncate(size / 8);
                continue;
            }
            let why = std::io::Error::from_raw_os_error(i32::try_from(error).unwrap_or(i32::MAX)).to_string();
            if let Some(armed) = armed.take() {
                let _ = armed.send(Err(why));
                return;
            }
            end(pending, &why);
            return;
        }
        // The first read is in place: from here on, every change is kept.
        if let Some(armed) = armed.take() {
            let _ = armed.send(Ok(()));
        }

        let events = [wake.0, stop.0];
        // SAFETY: both handles are live events.
        #[allow(unsafe_code, reason = "waiting on two events is a Win32 call")]
        let woke = unsafe { WaitForMultipleObjects(2, events.as_ptr(), 0, INFINITE) };

        let mut bytes = 0_u32;
        if woke != WAIT_OBJECT_0 {
            // Told to stop, or the wait itself failed: cancel the read and
            // wait it out, so the buffer is not written after it is freed.
            // SAFETY: the read was started on this handle with this
            // `OVERLAPPED`, which is still alive.
            #[allow(unsafe_code, reason = "cancelling a read is a Win32 call")]
            unsafe {
                CancelIoEx(directory.0, &raw const overlapped);
                GetOverlappedResult(directory.0, &raw const overlapped, &raw mut bytes, 1);
            }
            return;
        }

        // SAFETY: the read completed; this collects its result.
        #[allow(unsafe_code, reason = "collecting an overlapped result is a Win32 call")]
        let done = unsafe { GetOverlappedResult(directory.0, &raw const overlapped, &raw mut bytes, 0) };
        if done == 0 {
            // SAFETY: reads this thread's last error.
            #[allow(unsafe_code, reason = "the reason is only in the thread's last error")]
            let error = unsafe { GetLastError() };
            match error {
                ERROR_NOTIFY_ENUM_DIR => lose(pending),
                ERROR_IO_INCOMPLETE => {}
                ERROR_OPERATION_ABORTED => return,
                _ => {
                    end(
                        pending,
                        &std::io::Error::from_raw_os_error(i32::try_from(error).unwrap_or(i32::MAX)).to_string(),
                    );
                    return;
                }
            }
            continue;
        }
        if bytes == 0 {
            // The buffer overflowed: Windows dropped what did not fit and
            // says only that it did.
            lose(pending);
            continue;
        }

        let filled = usize::try_from(bytes).unwrap_or(0).min(size);
        // SAFETY: `buffer` holds `size` initialised bytes as `u64`s, and
        // `filled` is no more than that; a byte view of them is valid.
        #[allow(unsafe_code, reason = "the records are bytes in an aligned buffer")]
        let records = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), filled) };
        let names = names_in(records);
        if let Ok(mut pending) = pending.lock() {
            for name in names {
                let path = root.join(name);
                if pending.seen.insert(path.clone()) {
                    pending.paths.push(path);
                }
            }
        }
    }
}

fn lose(pending: &Mutex<Pending>) {
    if let Ok(mut pending) = pending.lock() {
        pending.lost = true;
    }
}

fn end(pending: &Mutex<Pending>, why: &str) {
    if let Ok(mut pending) = pending.lock() {
        pending.ended = Some(why.to_owned());
    }
}

/// The relative paths in a buffer of `FILE_NOTIFY_INFORMATION` records.
///
/// Each record: the offset of the next (zero for the last), the action, the
/// name's length in bytes, the name in UTF-16. The action is not read: every
/// kind of change comes down to "look here again".
fn names_in(records: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut at = 0_usize;
    loop {
        let field = |offset: usize| {
            records
                .get(at + offset..at + offset + 4)
                .and_then(|bytes| bytes.try_into().ok())
                .map(u32::from_le_bytes)
        };
        let (Some(next), Some(length)) = (field(0), field(8)) else {
            break;
        };
        let start = at + 12;
        let Some(name) = usize::try_from(length).ok().and_then(|length| records.get(start..start + length)) else {
            break;
        };
        let units: Vec<u16> = name.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        names.push(String::from_utf16_lossy(&units));
        if next == 0 {
            break;
        }
        at += usize::try_from(next).unwrap_or(usize::MAX);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(next: u32, name: &str) -> Vec<u8> {
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut record = Vec::new();
        record.extend_from_slice(&next.to_le_bytes());
        record.extend_from_slice(&1_u32.to_le_bytes());
        record.extend_from_slice(&u32::try_from(units.len()).expect("short").to_le_bytes());
        record.extend_from_slice(&units);
        record
    }

    #[test]
    fn a_buffer_of_records_reads_as_the_names_in_it() {
        let mut first = record(0, r"dir\file.txt");
        first.resize(first.len().next_multiple_of(4), 0);
        let next = u32::try_from(first.len()).expect("short");
        first[0..4].copy_from_slice(&next.to_le_bytes());
        first.extend(record(0, "other"));
        assert_eq!(names_in(&first), [r"dir\file.txt", "other"]);
    }

    #[test]
    fn a_record_that_runs_off_the_buffer_ends_the_reading() {
        let mut record = record(0, "name");
        record[8..12].copy_from_slice(&500_u32.to_le_bytes());
        assert!(names_in(&record).is_empty());
        assert!(names_in(&[]).is_empty());
    }
}
