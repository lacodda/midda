//! What the shell says of this account: where it keeps the reader's folders,
//! and whose an account is.
//!
//! Downloads is in the profile until a policy or the reader moves it - to
//! `D:\Downloads`, or into OneDrive - and the shell is what knows where it is
//! now. Its registry key, `User Shell Folders`, is the store behind the
//! answer, not the answer: `SHGetKnownFolderPath` expands what the key holds,
//! follows a redirection the key does not show, and is what Explorer itself
//! asks. Only this account's folders can be asked about; another account's
//! are looked for in its profile ([`crate::rules::pattern::Known`]).

use std::path::PathBuf;

use crate::rules::pattern::KnownFolder;

/// Where this account's shell keeps each known folder it can say: those it
/// cannot - a folder never made, an account with no profile - are left out.
#[must_use]
pub fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
    KnownFolder::ALL
        .into_iter()
        .filter_map(|folder| imp::known_folder(folder).map(|path| (folder, path)))
        .collect()
}

/// The name of the account `sid` is - `ada` - when this machine can say: a
/// deleted account, or one of a domain out of reach, has none.
#[must_use]
pub fn account_name(sid: &str) -> Option<String> {
    imp::account_name(sid)
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::{LocalFree, S_OK};
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{LookupAccountSidW, PSID, SID_NAME_USE};
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    };
    use windows_sys::core::{GUID, PWSTR};

    use crate::rules::pattern::KnownFolder;

    pub(super) fn known_folder(folder: KnownFolder) -> Option<PathBuf> {
        let id: &GUID = match folder {
            KnownFolder::Desktop => &FOLDERID_Desktop,
            KnownFolder::Documents => &FOLDERID_Documents,
            KnownFolder::Downloads => &FOLDERID_Downloads,
            KnownFolder::Music => &FOLDERID_Music,
            KnownFolder::Pictures => &FOLDERID_Pictures,
            KnownFolder::Videos => &FOLDERID_Videos,
        };
        let mut path: PWSTR = std::ptr::null_mut();
        // SAFETY: `id` is one of the shell's own constants; a null token asks
        // about this account; on success `path` is a NUL-terminated string
        // the shell allocated, read once and freed here, and on failure the
        // shell sets it to null, which CoTaskMemFree takes.
        #[allow(unsafe_code, reason = "where the shell keeps a known folder is one shell call")]
        let read = unsafe {
            let said = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT as _, std::ptr::null_mut(), &raw mut path);
            let read = (said == S_OK && !path.is_null()).then(|| {
                let length = (0..).take_while(|&at| *path.add(at) != 0).count();
                OsString::from_wide(std::slice::from_raw_parts(path, length))
            });
            CoTaskMemFree(path.cast_const().cast());
            read
        };
        read.map(PathBuf::from).filter(|path| path.is_absolute())
    }

    pub(super) fn account_name(sid: &str) -> Option<String> {
        let wide: Vec<u16> = std::ffi::OsStr::new(sid).encode_wide().chain(std::iter::once(0)).collect();
        let mut binary: PSID = std::ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated and outlives the call; on success
        // `binary` is a SID the system allocated, freed below with LocalFree
        // as the call's documentation says.
        #[allow(unsafe_code, reason = "a SID written as text has to be made binary to be looked up")]
        if unsafe { ConvertStringSidToSidW(wide.as_ptr(), &raw mut binary) } == 0 {
            return None;
        }
        let mut name = vec![0_u16; 256];
        let mut domain = vec![0_u16; 256];
        let (mut name_length, mut domain_length) = (256_u32, 256_u32);
        let mut kind: SID_NAME_USE = 0;
        // SAFETY: `binary` is the SID made above; both buffers are as long as
        // the lengths passed with them, which the call shortens to what it
        // wrote. The SID is freed once, after the lookup.
        #[allow(unsafe_code, reason = "whose a SID is, is one call to the account database")]
        let found = unsafe {
            let found = LookupAccountSidW(
                std::ptr::null(),
                binary,
                name.as_mut_ptr(),
                &raw mut name_length,
                domain.as_mut_ptr(),
                &raw mut domain_length,
                &raw mut kind,
            ) != 0;
            LocalFree(binary.cast());
            found
        };
        if !found || name_length == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&name[..name_length as usize]))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    use crate::rules::pattern::KnownFolder;

    pub(super) const fn known_folder(_folder: KnownFolder) -> Option<PathBuf> {
        None
    }

    pub(super) const fn account_name(_sid: &str) -> Option<String> {
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn the_shell_says_where_this_accounts_folders_are() {
        let folders = known_folders();
        let downloads = folders
            .iter()
            .find(|(folder, _)| *folder == KnownFolder::Downloads)
            .map(|(_, path)| path)
            .expect("an account with a profile has a Downloads");
        assert!(downloads.is_absolute(), "{}", downloads.display());
    }

    #[test]
    fn an_account_is_named_by_its_sid_and_a_sid_of_nobody_is_not() {
        // The well-known SID of the local system, the same on every machine.
        assert!(
            account_name("S-1-5-18").is_some_and(|name| !name.is_empty()),
            "named in the language of the machine"
        );
        assert_eq!(account_name("not a sid"), None);
        assert_eq!(account_name("S-1-5-21-1-2-3-999999"), None, "an account this machine never had");
    }
}
