//! What can be scanned, and how full it already is.
//!
//! The first screen asks a question the user can only answer if they can see
//! the volumes: picking a folder is the deliberate case, picking a drive is the
//! common one. The overview of every volume at once is v0.2.0; this is the list
//! it grows out of.

use serde::{Deserialize, Serialize};

/// A drive the user can point a scan at.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    /// What to scan: `C:\` and the like.
    pub path: String,
    /// What to show: the label when the volume has one, the path otherwise.
    pub label: String,
    /// Total capacity in bytes, when it could be read.
    pub total: Option<u64>,
    /// Free space in bytes, when it could be read.
    pub free: Option<u64>,
}

/// Every volume this machine can scan.
///
/// # Errors
///
/// Never — a volume that cannot be queried is left out rather than failing the
/// list, because one unreadable network drive should not empty the first
/// screen.
#[tauri::command]
pub fn list_volumes() -> Result<Vec<Volume>, String> {
    Ok(imp::list())
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt as _;

    use super::Volume;

    /// A UTF-16, NUL-terminated path.
    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    }

    pub(super) fn list() -> Vec<Volume> {
        // `GetLogicalDrives` returns a bitmask, one bit per letter from A. It is
        // the cheapest way to ask and it does not touch a drive that is not
        // there, which matters: probing an empty optical drive spins it up.
        //
        // SAFETY: the call takes nothing and returns a `u32`.
        #[allow(unsafe_code, reason = "the set of drive letters is not exposed by std")]
        let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };

        (0..26_u32)
            .filter(|letter| mask & (1 << letter) != 0)
            .filter_map(|letter| {
                let letter = char::from(b'A' + u8::try_from(letter).ok()?);
                let path = format!("{letter}:\\");
                // A drive that is not ready — an empty card reader, a
                // disconnected network share — has no space to report. It is
                // listed anyway, without numbers, because it is still somewhere
                // a scan could be pointed once it is ready.
                let (total, free) = space(&path).map_or((None, None), |(total, free)| (Some(total), Some(free)));
                Some(Volume {
                    label: label(&path).unwrap_or_else(|| path.clone()),
                    path,
                    total,
                    free,
                })
            })
            .collect()
    }

    /// Total and free bytes on a volume.
    fn space(path: &str) -> Option<(u64, u64)> {
        let wide = wide(path);
        let mut free_to_caller: u64 = 0;
        let mut total: u64 = 0;
        let mut free: u64 = 0;

        // SAFETY: `wide` is NUL-terminated and outlives the call; the three
        // out-parameters are valid writable `u64`s written only through those
        // pointers.
        #[allow(unsafe_code, reason = "free space on a volume is not exposed by std")]
        let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &raw mut free_to_caller, &raw mut total, &raw mut free) };

        // `free_to_caller` is what a quota would leave this user; `free` is what
        // the volume actually has. midda reports the volume, because the number
        // it is about to compare against is the volume's too.
        (ok != 0).then_some((total, free))
    }

    /// The volume label, when it has one: "Windows", "Data".
    fn label(path: &str) -> Option<String> {
        let wide_path = wide(path);
        // MAX_PATH + 1, which is what the API documents as enough for a label.
        let mut buffer = [0_u16; 261];

        // SAFETY: `wide_path` is NUL-terminated and outlives the call; `buffer`
        // is a valid writable array and its true length is passed. The four
        // nulls are the documented "do not report these" arguments.
        #[allow(unsafe_code, reason = "a volume label is not exposed by std")]
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW(
                wide_path.as_ptr(),
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).ok()?,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };

        if ok == 0 {
            return None;
        }

        let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
        let label = String::from_utf16_lossy(&buffer[..end]);
        // An unlabelled volume answers with an empty string rather than
        // failing, and an empty label in a list is a blank row.
        (!label.is_empty()).then(|| format!("{label} ({})", path.trim_end_matches('\\')))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Volume;

    pub(super) fn list() -> Vec<Volume> {
        // Windows is the product's target. Elsewhere — which means CI, keeping
        // the walk honest per ADR 0001 — there is one root and no drive
        // letters, and offering it keeps the window testable off Windows.
        vec![Volume {
            path: "/".to_owned(),
            label: "/".to_owned(),
            total: None,
            free: None,
        }]
    }
}
