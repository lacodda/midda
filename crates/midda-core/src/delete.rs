//! Deleting what the rules found: for good, or into the Recycle Bin.
//!
//! # Two ways
//!
//! What comes back as it was ([`Risk::Safe`]) is deleted for good. A
//! `node_modules` with its lockfile beside it comes back with an install, and
//! in the Recycle Bin it would only move: the volume gets nothing back until
//! the bin is emptied, and the space is what the reader came for. What may not
//! come back as it was ([`Risk::Caution`]) goes to the Recycle Bin, where
//! Explorer restores it. The reader may send a safe one to the bin as well;
//! deleting a careful one for good is a choice of its own, which the window
//! asks for a second time ([`Way::of`]).
//!
//! # For good
//!
//! midda's own walk, not `remove_dir_all`, for three reasons. A link is
//! removed as a link and never followed: the junctions pnpm puts in a
//! `node_modules` point into its store, which every other project uses. A
//! read-only file loses the attribute and goes, as Explorer deletes it. And
//! what the volume refuses - a file another program holds, one this account
//! may not change - stays, is counted and said, and the rest still goes: a
//! build folder with one locked file in it gives back everything else. It
//! stops between two files when asked.
//!
//! # The Recycle Bin
//!
//! The shell's (`IFileOperation`), never a move of our own into
//! `$Recycle.Bin`: what the shell put there, Explorer lists and restores, and
//! the bin's size is the shell's to keep. The shell, though, deletes for good
//! what the bin will not take - an item too large for it, a volume whose bin
//! is off - and says so only in a dialog, which a silent operation answers
//! yes to. midda's progress sink refuses every item the shell is about to
//! delete rather than recycle, so "to the Recycle Bin" never means "for good":
//! such an item stays where it is, and the window says why. See
//! `docs/adr/0018-deleting-what-the-rules-found.md`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::rules::Risk;
use crate::scanner::Progress;

/// How an entry goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Way {
    /// Deleted for good: the space comes back now.
    ForGood,
    /// Moved to the Recycle Bin: it can be restored, and the space comes back
    /// when the bin is emptied.
    RecycleBin,
}

impl Way {
    /// How a finding goes unless the reader says otherwise: deleted, when it
    /// comes back as it was - the bin would only move it - and into the bin
    /// when it may not (the owner's decision of 11.09).
    #[must_use]
    pub const fn of(risk: Risk) -> Self {
        match risk {
            Risk::Safe => Self::ForGood,
            Risk::Caution => Self::RecycleBin,
        }
    }

    /// The same, for an entry that may or may not go to the bin. What cannot
    /// (what is in the bin already, what is on a volume without one) goes for
    /// good or not at all, and a careful one going for good is asked for a
    /// second time by the window.
    #[must_use]
    pub const fn of_entry(risk: Risk, recyclable: bool) -> Self {
        if recyclable { Self::of(risk) } else { Self::ForGood }
    }
}

/// Why something stayed where it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Left {
    /// Another program holds it.
    InUse,
    /// This account may not delete it.
    Denied,
    /// The Recycle Bin would not take it, and it was not deleted instead.
    NotRecycled,
    /// The volume refused for another reason.
    Failed,
}

impl Left {
    /// What the window says of what stayed for this reason.
    #[must_use]
    pub const fn says(self) -> &'static str {
        match self {
            Self::InUse => "held open by another program",
            Self::Denied => "not this account's to delete",
            Self::NotRecycled => "not taken by the Recycle Bin, and so not deleted",
            Self::Failed => "refused by the volume",
        }
    }
}

/// What stayed for one reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub why: Left,
    /// How many entries, and what the files among them read as.
    pub entries: u64,
    pub bytes: u64,
    /// The first of them, and what the volume said: what a reader asks about.
    pub first: Option<String>,
}

/// What deleting one entry did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    /// Files deleted, and what they read as.
    pub files: u64,
    pub bytes: u64,
    /// Folders deleted, and links removed without following them.
    pub folders: u64,
    pub links: u64,
    /// Entries moved to the Recycle Bin whole, rather than deleted.
    pub recycled: u64,
    /// What stayed, by why - the most first.
    pub left: Vec<Kept>,
    /// The reader stopped it before the end.
    pub stopped: bool,
}

impl Removed {
    fn leave(&mut self, why: Left, bytes: u64, first: impl FnOnce() -> String) {
        if let Some(kept) = self.left.iter_mut().find(|kept| kept.why == why) {
            kept.entries += 1;
            kept.bytes += bytes;
        } else {
            self.left.push(Kept {
                why,
                entries: 1,
                bytes,
                first: Some(first()),
            });
        }
    }

    /// Whether everything went.
    #[must_use]
    pub fn whole(&self) -> bool {
        self.left.is_empty() && !self.stopped
    }

    /// Whether anything went at all.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.files + self.folders + self.links + self.recycled > 0
    }

    /// Adds what deleting another entry did.
    fn join(&mut self, other: Self) {
        self.files += other.files;
        self.bytes += other.bytes;
        self.folders += other.folders;
        self.links += other.links;
        self.recycled += other.recycled;
        self.stopped |= other.stopped;
        for kept in other.left {
            match self.left.iter_mut().find(|held| held.why == kept.why) {
                Some(held) => {
                    held.entries += kept.entries;
                    held.bytes += kept.bytes;
                    held.first = held.first.take().or(kept.first);
                }
                None => self.left.push(kept),
            }
        }
    }
}

/// Deletes each of `entries` - a path and what it reads as - the one way:
/// for good ([`remove`]) or into the Recycle Bin ([`recycle`]), one entry at
/// a time, stopping between two when `progress` is cancelled. An entry sent
/// to the bin counts what it reads as into `progress` once it is there.
#[must_use]
pub fn take(entries: &[(std::path::PathBuf, u64)], way: Way, progress: &Progress) -> Removed {
    let mut done = Removed::default();
    for (path, bytes) in entries {
        if progress.cancelled() {
            done.stopped = true;
            break;
        }
        match way {
            Way::ForGood => done.join(remove(path, progress)),
            Way::RecycleBin => match recycle(path) {
                Ok(()) => {
                    done.recycled += 1;
                    progress.advance(1, *bytes);
                }
                Err((why, said)) => done.leave(why, *bytes, || said),
            },
        }
        if done.stopped {
            break;
        }
    }
    done.left.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.why.cmp(&b.why)));
    done
}

/// Why the entry at `path` cannot go to the Recycle Bin, when it cannot: it
/// is in the bin already - emptying the bin is deleting for good - or its
/// volume has no bin. What only the shell knows - the bin's size, a bin
/// turned off - it says when asked, and [`recycle`] keeps the entry then.
#[must_use]
pub fn bin_refused(path: &Path) -> Option<String> {
    if in_the_bin(path) {
        return Some("it is in the Recycle Bin already, and emptying the bin deletes for good".to_owned());
    }
    imp::bin_refused(path)
}

/// Whether `path` is inside a volume's Recycle Bin.
fn in_the_bin(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str().to_str().is_some_and(|name| name.eq_ignore_ascii_case("$Recycle.Bin")))
}

/// Deletes the file or folder at `path` for good: every file in it, every
/// folder once it is empty, every link as a link. What the volume refuses
/// stays and is counted; an entry that is gone already is no error. Stops
/// between two files when `progress` is cancelled; `progress` counts the
/// files gone through and what they read as.
#[must_use]
pub fn remove(path: &Path, progress: &Progress) -> Removed {
    let mut done = Removed::default();
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return done,
        Err(error) => {
            done.leave(refusal(&error), 0, || said(path, &error));
            return done;
        }
    };
    if progress.cancelled() {
        done.stopped = true;
        return done;
    }
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        entry(path, &metadata, progress, &mut done);
        return done;
    }
    // Depth first, a folder removed once what is in it has been: the second
    // visit is the one that removes it.
    let mut stack = vec![(path.to_path_buf(), false)];
    while let Some((folder, emptied)) = stack.pop() {
        if emptied {
            match remove_folder(&folder) {
                Ok(()) => done.folders += 1,
                // Something in it stayed, and is counted where it stayed.
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => done.leave(refusal(&error), 0, || said(&folder, &error)),
            }
            continue;
        }
        if progress.cancelled() {
            done.stopped = true;
            break;
        }
        stack.push((folder.clone(), true));
        let listing = match std::fs::read_dir(&folder) {
            Ok(listing) => listing,
            Err(error) => {
                done.leave(refusal(&error), 0, || said(&folder, &error));
                continue;
            }
        };
        for child in listing.flatten() {
            if progress.cancelled() {
                done.stopped = true;
                break;
            }
            // `DirEntry::metadata` does not follow a link: a junction is a
            // link here, not the folder it points to.
            let Ok(metadata) = child.metadata() else {
                continue;
            };
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                stack.push((child.path(), false));
            } else {
                entry(&child.path(), &metadata, progress, &mut done);
            }
        }
        if done.stopped {
            break;
        }
    }
    done.left.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.why.cmp(&b.why)));
    done
}

/// Deletes one file, or one link as a link.
fn entry(path: &Path, metadata: &std::fs::Metadata, progress: &Progress, done: &mut Removed) {
    let link = metadata.file_type().is_symlink();
    let bytes = if link { 0 } else { metadata.len() };
    let removed = if link && imp::link_to_folder(metadata) {
        remove_folder(path)
    } else {
        remove_file(path)
    };
    progress.advance(1, bytes);
    match removed {
        Ok(()) if link => done.links += 1,
        Ok(()) => {
            done.files += 1;
            done.bytes += bytes;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => done.leave(refusal(&error), bytes, || said(path, &error)),
    }
}

/// Removes a file. Rust deletes a read-only one by itself on Windows
/// (measured 2026-11-12); the retry stands behind it on any volume that
/// refuses it.
fn remove_file(path: &Path) -> std::io::Result<()> {
    retry_writable(path, || std::fs::remove_file(path))
}

/// Removes an empty folder, or a link to one. A read-only folder is refused
/// (measured 2026-11-12), and loses the attribute, as Explorer deletes it.
fn remove_folder(path: &Path) -> std::io::Result<()> {
    retry_writable(path, || std::fs::remove_dir(path))
}

/// Runs `remove`, and once more with the read-only attribute taken off when
/// the volume refused it for that.
fn retry_writable(path: &Path, remove: impl Fn() -> std::io::Result<()>) -> std::io::Result<()> {
    let refused = match remove() {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => error,
        Err(error) => return Err(error),
    };
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Err(refused);
    };
    let mut permissions = metadata.permissions();
    if !permissions.readonly() {
        return Err(refused);
    }
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "the entry is being deleted, and a read-only one is deleted as Explorer deletes it"
    )]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).map_err(|_| refused)?;
    remove()
}

/// Why the volume refused, in the terms the window counts by.
fn refusal(error: &std::io::Error) -> Left {
    match error.raw_os_error() {
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
        Some(32 | 33) if cfg!(windows) => Left::InUse,
        _ if error.kind() == std::io::ErrorKind::PermissionDenied => Left::Denied,
        _ => Left::Failed,
    }
}

/// The path and what the volume said of it.
fn said(path: &Path, error: &std::io::Error) -> String {
    format!("{}: {error}", path.display())
}

/// Moves the file or folder at `path` to the Recycle Bin, through the shell.
///
/// An item the bin would not take is left where it is, never deleted
/// instead: the shell says it would delete it for good, and midda says no.
///
/// # Errors
///
/// Why it stayed, and what was said: the bin would not take it, another
/// program holds it, this account may not move it, the shell refused.
pub fn recycle(path: &Path) -> Result<(), (Left, String)> {
    if let Some(refusal) = bin_refused(path) {
        return Err((Left::NotRecycled, refusal));
    }
    imp::recycle(path)
}

/// Whether the shell, asking to delete an item with `flags`, would delete it
/// for good rather than recycle it: it says it would recycle only with
/// `TSF_DELETE_RECYCLE_IF_POSSIBLE`, and an item the bin will not take comes
/// without it. The one decision the progress sink makes.
#[cfg_attr(not(windows), allow(dead_code, reason = "the shell is Windows'"))]
const fn would_delete_for_good(flags: u32) -> bool {
    /// `TSF_DELETE_RECYCLE_IF_POSSIBLE`.
    const RECYCLE_IF_POSSIBLE: u32 = 0x80;
    flags & RECYCLE_IF_POSSIBLE == 0
}

#[cfg(windows)]
#[allow(unsafe_code, reason = "the Recycle Bin is the shell's, reached through COM")]
mod imp {
    use std::cell::Cell;
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::fs::MetadataExt as _;
    use std::path::Path;
    use std::rc::Rc;

    use windows::Win32::Foundation::{E_ABORT, ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::{
        FOF_ALLOWUNDO, FOF_NO_UI, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IFileOperationProgressSink,
        IFileOperationProgressSink_Impl, IShellItem, SHCreateItemFromParsingName,
    };
    use windows_core::{HRESULT, HSTRING, PCWSTR, Ref, implement};
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};

    use super::Left;

    /// `FILE_ATTRIBUTE_DIRECTORY`.
    const DIRECTORY: u32 = 0x10;
    /// `DRIVE_FIXED`: the one kind of volume Windows keeps a Recycle Bin on.
    const DRIVE_FIXED: u32 = 3;

    pub(super) fn link_to_folder(metadata: &std::fs::Metadata) -> bool {
        metadata.file_attributes() & DIRECTORY != 0
    }

    pub(super) fn bin_refused(path: &Path) -> Option<String> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut volume = vec![0_u16; 261];
        // SAFETY: `wide` ends in a zero; `volume` is as long as the length
        // passed.
        let found = unsafe { GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), u32::try_from(volume.len()).unwrap_or(261)) };
        if found == 0 {
            return Some("the volume it is on could not be asked whether it has a Recycle Bin".to_owned());
        }
        // SAFETY: `volume` was written with a zero at its end.
        let kind = unsafe { GetDriveTypeW(volume.as_ptr()) };
        (kind != DRIVE_FIXED).then(|| {
            match kind {
                2 => "a removable drive has no Recycle Bin",
                4 => "a network share has no Recycle Bin",
                _ => "this volume has no Recycle Bin",
            }
            .to_owned()
        })
    }

    /// What the shell said of the one item it was given.
    #[derive(Debug, Clone, Copy, Default)]
    struct Heard {
        /// It was about to delete it for good, and was told no.
        refused: bool,
        /// It deleted it, into the bin or not.
        deleted: Option<HRESULT>,
        recycled: bool,
    }

    /// The progress sink: says no to deleting for good, and listens.
    #[implement(IFileOperationProgressSink)]
    struct Guard {
        heard: Rc<Cell<Heard>>,
    }

    impl IFileOperationProgressSink_Impl for Guard_Impl {
        fn StartOperations(&self) -> windows_core::Result<()> {
            Ok(())
        }
        fn FinishOperations(&self, _: HRESULT) -> windows_core::Result<()> {
            Ok(())
        }
        fn PreRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows_core::Result<()> {
            Ok(())
        }
        fn PostRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<IShellItem>) -> windows_core::Result<()> {
            Ok(())
        }
        fn PreMoveItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR) -> windows_core::Result<()> {
            Ok(())
        }
        fn PostMoveItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<IShellItem>) -> windows_core::Result<()> {
            Ok(())
        }
        fn PreCopyItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR) -> windows_core::Result<()> {
            Ok(())
        }
        fn PostCopyItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<IShellItem>) -> windows_core::Result<()> {
            Ok(())
        }
        fn PreDeleteItem(&self, flags: u32, _: Ref<IShellItem>) -> windows_core::Result<()> {
            if super::would_delete_for_good(flags) {
                let mut heard = self.heard.get();
                heard.refused = true;
                self.heard.set(heard);
                // A failure here cancels this item and every one after it.
                return Err(E_ABORT.into());
            }
            Ok(())
        }
        fn PostDeleteItem(&self, _: u32, _: Ref<IShellItem>, result: HRESULT, in_the_bin: Ref<IShellItem>) -> windows_core::Result<()> {
            let mut heard = self.heard.get();
            heard.deleted = Some(result);
            heard.recycled = result.is_ok() && !in_the_bin.is_null();
            self.heard.set(heard);
            Ok(())
        }
        fn PreNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows_core::Result<()> {
            Ok(())
        }
        fn PostNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR, _: &PCWSTR, _: u32, _: HRESULT, _: Ref<IShellItem>) -> windows_core::Result<()> {
            Ok(())
        }
        fn UpdateProgress(&self, _: u32, _: u32) -> windows_core::Result<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> windows_core::Result<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> windows_core::Result<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> windows_core::Result<()> {
            Ok(())
        }
    }

    pub(super) fn recycle(path: &Path) -> Result<(), (Left, String)> {
        let path = std::path::absolute(path).map_err(|error| (Left::Failed, format!("{}: {error}", path.display())))?;
        // The apartment is entered for this one operation and left after it:
        // a thread that is in another one already can still make the call.
        // SAFETY: no reserved pointer; balanced below when it succeeded.
        let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        let outcome = operate(&path);
        if entered {
            // SAFETY: balances the CoInitializeEx above, on the same thread.
            unsafe { CoUninitialize() };
        }
        outcome
    }

    fn operate(path: &Path) -> Result<(), (Left, String)> {
        let shown = path.display().to_string();
        let failed = |error: windows_core::Error| (why_of(error.code()), format!("{shown}: {}", error.message()));
        // SAFETY: a COM call on a thread in an apartment, with no outer
        // object.
        let operation: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }.map_err(failed)?;
        // No dialog of any kind, the shell's undo stack, the bin, and a stop
        // at the first error rather than a question.
        // SAFETY: the operation is live; the flags are the shell's own.
        unsafe { operation.SetOperationFlags(FOF_NO_UI | FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE) }.map_err(failed)?;
        // SAFETY: the path is a string the call copies; no bind context.
        let item: IShellItem = unsafe { SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None) }.map_err(failed)?;
        let heard = Rc::new(Cell::new(Heard::default()));
        let guard: IFileOperationProgressSink = Guard { heard: Rc::clone(&heard) }.into();
        // SAFETY: both items are live for the call, and the operation keeps
        // its own reference to the sink.
        unsafe { operation.DeleteItem(&item, &guard) }.map_err(failed)?;
        // SAFETY: as above.
        let performed = unsafe { operation.PerformOperations() };
        let heard = heard.get();
        if heard.refused {
            return Err((
                Left::NotRecycled,
                format!("{shown}: the Recycle Bin would not take it - too large for the bin, or the bin is off on this volume"),
            ));
        }
        performed.map_err(failed)?;
        match heard.deleted {
            Some(result) if result.is_err() => Err(failed(result.into())),
            Some(_) if heard.recycled => Ok(()),
            Some(_) => Err((Left::Failed, format!("{shown}: the shell deleted it, and did not say it is in the Recycle Bin"))),
            // SAFETY: as above.
            None if unsafe { operation.GetAnyOperationsAborted() }.is_ok_and(|aborted| aborted.as_bool()) => {
                Err((Left::Failed, format!("{shown}: the shell stopped before it")))
            }
            None => Err((Left::Failed, format!("{shown}: the shell did nothing with it"))),
        }
    }

    /// What a failure the shell gave is, in the terms the window counts by.
    fn why_of(code: HRESULT) -> Left {
        let win32 = |error: windows::Win32::Foundation::WIN32_ERROR| HRESULT::from_win32(error.0);
        if code == win32(ERROR_SHARING_VIOLATION) || code == win32(ERROR_LOCK_VIOLATION) {
            Left::InUse
        } else if code == win32(ERROR_ACCESS_DENIED) {
            Left::Denied
        } else {
            Left::Failed
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    use super::Left;

    pub(super) fn link_to_folder(_metadata: &std::fs::Metadata) -> bool {
        // A link is a file here, removed as one.
        false
    }

    pub(super) fn bin_refused(_path: &Path) -> Option<String> {
        Some("the Recycle Bin is Windows'".to_owned())
    }

    pub(super) fn recycle(_path: &Path) -> Result<(), (Left, String)> {
        Err((Left::NotRecycled, "the Recycle Bin is Windows'".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn write(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("folders");
        }
        std::fs::write(path, vec![b'x'; bytes]).expect("write");
    }

    #[test]
    fn what_comes_back_is_deleted_and_what_may_not_goes_to_the_bin() {
        assert_eq!(Way::of(Risk::Safe), Way::ForGood);
        assert_eq!(Way::of(Risk::Caution), Way::RecycleBin);
        assert_eq!(Way::of_entry(Risk::Caution, false), Way::ForGood, "emptying the bin is for good");
        assert_eq!(Way::of_entry(Risk::Caution, true), Way::RecycleBin);
    }

    #[test]
    fn the_shell_is_told_no_whenever_it_would_not_recycle() {
        assert!(!would_delete_for_good(0x80), "recycle if possible");
        assert!(!would_delete_for_good(0x80 | 0x4), "with other flags beside it");
        assert!(would_delete_for_good(0), "no word of the bin: it would delete for good");
        assert!(would_delete_for_good(0x4 | 0x40), "other flags alone");
    }

    #[test]
    fn what_is_in_the_bin_is_never_sent_to_it() {
        let inside = PathBuf::from("C:/$Recycle.Bin/S-1-5-21-1/$RABC123.zip");
        assert!(bin_refused(&inside).is_some_and(|why| why.contains("Recycle Bin already")));
        assert!(in_the_bin(Path::new("D:/$RECYCLE.BIN/x")), "case does not hide the bin");
        assert!(!in_the_bin(Path::new("C:/Users/me/Recycle.Bin.notes")));
    }

    #[test]
    fn a_folder_goes_whole_with_its_read_only_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("target");
        write(&folder.join("debug/app.exe"), 3000);
        write(&folder.join("debug/deps/a.rlib"), 2000);
        write(&folder.join(".rustc_info.json"), 100);
        let read_only = folder.join("debug/locked.txt");
        write(&read_only, 50);
        // A read-only file, and on Windows a read-only folder, which it
        // refuses to remove as it is. Elsewhere a folder's write bit is what
        // lets anything in it be deleted at all - the account's to give.
        let mut read_only = vec![read_only];
        if cfg!(windows) {
            read_only.push(folder.join("debug/deps"));
        }
        for path in read_only {
            let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&path, permissions).expect("read-only");
        }

        let progress = Progress::new();
        let done = remove(&folder, &progress);
        assert!(done.whole(), "{done:?}");
        assert_eq!((done.files, done.bytes, done.folders), (4, 5150, 3));
        assert!(!folder.exists());
        assert!(dir.path().exists(), "only the entry goes, not the folder it is in");
        assert_eq!(progress.entries(), 4);
    }

    #[test]
    fn a_link_inside_is_removed_and_never_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = dir.path().join("store");
        write(&store.join("package/index.js"), 400);
        // Joined name by name: cmd takes a `/` in a path for a switch.
        let modules = dir.path().join("app").join("node_modules");
        std::fs::create_dir_all(&modules).expect("modules");
        write(&modules.join(".modules.yaml"), 10);
        let link = modules.join("package");
        #[cfg(windows)]
        {
            // A junction, as pnpm makes them: no privilege needed.
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(store.join("package"))
                .output()
                .expect("cmd runs");
            assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(store.join("package"), &link).expect("symlink");

        let done = remove(&modules, &Progress::new());
        assert!(done.whole(), "{done:?}");
        assert_eq!((done.files, done.links), (1, 1));
        assert!(!modules.exists());
        assert!(store.join("package/index.js").exists(), "what the link pointed to is untouched");
    }

    #[test]
    #[cfg(windows)]
    fn a_file_held_open_stays_and_the_rest_goes() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("build");
        write(&folder.join("a.o"), 100);
        write(&folder.join("held.log"), 70);
        write(&folder.join("deep/b.o"), 200);
        // Opened with no sharing at all: deleting it is a sharing violation.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(folder.join("held.log"))
            .expect("held");

        let done = remove(&folder, &Progress::new());
        drop(held);
        assert_eq!((done.files, done.bytes), (2, 300), "{done:?}");
        assert_eq!(done.left.len(), 1);
        assert_eq!((done.left[0].why, done.left[0].entries, done.left[0].bytes), (Left::InUse, 1, 70));
        assert!(done.left[0].first.as_deref().is_some_and(|first| first.contains("held.log")));
        assert!(folder.join("held.log").exists());
        assert!(!folder.join("deep").exists(), "the folder that emptied went");
        assert!(folder.exists(), "the one that did not, stayed");
    }

    #[test]
    fn several_entries_go_one_way_and_what_the_bin_refuses_stays() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("Temp/old1.tmp"), 10);
        write(&dir.path().join("Temp/old2/x.tmp"), 20);
        write(&dir.path().join("Temp/new.tmp"), 30);
        let entries = [(dir.path().join("Temp/old1.tmp"), 10), (dir.path().join("Temp/old2"), 20)];
        let progress = Progress::new();
        let done = take(&entries, Way::ForGood, &progress);
        assert!(done.whole());
        assert_eq!((done.files, done.bytes, done.folders), (2, 30, 1));
        assert!(dir.path().join("Temp/new.tmp").exists(), "what was not named stays");
        assert!(dir.path().join("Temp").exists(), "and the folder emptied stays");

        // Emptying the bin into the bin is no way at all: it stays.
        let binned = dir.path().join("$Recycle.Bin").join("$RXYZ.txt");
        write(&binned, 40);
        let done = take(&[(binned.clone(), 40)], Way::RecycleBin, &progress);
        assert_eq!(done.recycled, 0);
        assert_eq!((done.left[0].why, done.left[0].entries, done.left[0].bytes), (Left::NotRecycled, 1, 40));
        assert!(binned.exists());
    }

    #[test]
    fn a_stopped_deletion_says_it_stopped_and_one_already_gone_is_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(&dir.path().join("x/a.txt"), 10);
        let progress = Progress::new();
        progress.cancel();
        let done = remove(&dir.path().join("x"), &progress);
        assert!(done.stopped);
        assert_eq!(done.files, 0);
        assert!(dir.path().join("x/a.txt").exists());

        let gone = remove(&dir.path().join("never"), &Progress::new());
        assert_eq!(gone, Removed::default());
    }

    /// Where the shell put `original` in the current account's Recycle Bin:
    /// the `$I` file that records it, and the `$R` one that is it. Read by
    /// the format the bin writes: a version (2), the size, the time it was
    /// deleted, the length of the path and the path.
    #[cfg(windows)]
    fn in_the_recycle_bin(original: &Path) -> Option<(PathBuf, PathBuf)> {
        let volume = original.components().next()?.as_os_str().to_string_lossy().into_owned();
        let bin = PathBuf::from(format!("{volume}\\$Recycle.Bin"));
        for account in std::fs::read_dir(bin).ok()?.flatten() {
            let Ok(listing) = std::fs::read_dir(account.path()) else {
                continue;
            };
            for record in listing.flatten() {
                let name = record.file_name().to_string_lossy().into_owned();
                let Some(rest) = name.strip_prefix("$I") else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(record.path()) else {
                    continue;
                };
                if bytes.len() < 28 || bytes[..8] != 2_u64.to_le_bytes() {
                    continue;
                }
                let units: Vec<u16> = bytes[28..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&pair| u16::from_le_bytes(pair))
                    .take_while(|&unit| unit != 0)
                    .collect();
                // Case is Windows' to ignore, and the shell's to write.
                if String::from_utf16_lossy(&units).eq_ignore_ascii_case(&original.to_string_lossy()) {
                    return Some((record.path(), account.path().join(format!("$R{rest}"))));
                }
            }
        }
        None
    }

    #[test]
    #[cfg(windows)]
    fn what_goes_to_the_bin_is_in_the_bin_or_where_it_was_and_never_gone() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The long form of the path, as the bin records it: a runner's
        // temporary folder is often given in its short 8.3 form
        // (`C:\Users\RUNNER~1\...`), and the record names the long one.
        let long = std::fs::canonicalize(dir.path()).expect("canonical");
        let long = PathBuf::from(long.to_string_lossy().trim_start_matches(r"\\?\"));
        let folder = long.join("old-download");
        write(&folder.join("setup.exe"), 5000);
        if let Some(why) = bin_refused(&folder) {
            eprintln!("skipped: {why}");
            return;
        }
        match recycle(&folder) {
            Ok(()) => {
                assert!(!folder.exists(), "it left where it was");
                let (record, item) = in_the_recycle_bin(&folder).expect("and the bin records where it came from");
                assert!(item.join("setup.exe").exists(), "the bin holds it whole");
                // Out of the reader's bin again: a test leaves nothing there.
                std::fs::remove_dir_all(&item).expect("emptied");
                std::fs::remove_file(&record).expect("emptied");
            }
            Err((why, said)) => {
                // A machine whose bin will not take it: the one outcome that
                // must never happen is that it is gone and not in the bin.
                eprintln!("not recycled here ({why:?}): {said}");
                assert!(folder.join("setup.exe").exists(), "what the bin refused stays where it was");
            }
        }
    }
}
