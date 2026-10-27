//! The WSL distributions this account has, and the disk of each: what WSL
//! itself calls a disk midda finds.
//!
//! WSL keeps its list in the registry, under the account's own key: a name,
//! the folder of its disk, and the disk's file name. Read straight from
//! there - asking `wsl --list` would start WSL's service only to be told what
//! the registry already says, and would not say where the disks are.

use std::path::{Path, PathBuf};

/// A WSL distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distribution {
    /// What `wsl -d` takes: `Ubuntu`, `docker-desktop`.
    pub name: String,
    /// Its disk.
    pub disk: PathBuf,
}

/// The name of the distribution whose disk is `path`, among `distributions`.
#[must_use]
pub fn name_of<'a>(path: &Path, distributions: &'a [Distribution]) -> Option<&'a str> {
    let wanted = comparable(path);
    distributions
        .iter()
        .find(|distribution| comparable(&distribution.disk) == wanted)
        .map(|distribution| distribution.name.as_str())
}

/// A path as two spellings of it compare: no `\\?\` before it, one kind of
/// slash, one case - Windows folds case in a name.
fn comparable(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    text.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// This account's distributions, as WSL lists them; none where there is no
/// WSL, or on another platform.
#[must_use]
pub fn distributions() -> Vec<Distribution> {
    #[cfg(windows)]
    {
        registry::distributions()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
#[allow(unsafe_code, reason = "WSL's list is in the registry, which std does not read")]
mod registry {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW};

    use super::Distribution;

    const LIST: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain(Some(0)).collect()
    }

    /// A string value of `subkey` under `key`.
    fn text(key: HKEY, subkey: &[u16], value: &str) -> Option<String> {
        let value = wide(value);
        let mut buffer = vec![0_u16; 1024];
        let mut bytes = u32::try_from(buffer.len() * 2).ok()?;
        // SAFETY: the key is open, both names end in a zero, and the buffer
        // is as long as the size said.
        let status = unsafe {
            RegGetValueW(
                key,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &raw mut bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let units = usize::try_from(bytes).ok()? / 2;
        let end = buffer[..units].iter().position(|&unit| unit == 0).unwrap_or(units);
        Some(OsString::from_wide(&buffer[..end]).to_string_lossy().into_owned())
    }

    pub(super) fn distributions() -> Vec<Distribution> {
        let list = wide(LIST);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: the name ends in a zero; the key is closed below.
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, list.as_ptr(), 0, KEY_READ, &raw mut key) } != ERROR_SUCCESS {
            return Vec::new();
        }
        let mut found = Vec::new();
        for index in 0.. {
            let mut name = vec![0_u16; 256];
            let mut length = u32::try_from(name.len()).unwrap_or(0);
            // SAFETY: the key is open and the buffer as long as said.
            let status = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    name.as_mut_ptr(),
                    &raw mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            let mut subkey = name[..usize::try_from(length).unwrap_or(0)].to_vec();
            subkey.push(0);
            let (Some(distribution), Some(base)) = (text(key, &subkey, "DistributionName"), text(key, &subkey, "BasePath")) else {
                continue;
            };
            let file = text(key, &subkey, "VhdFileName").unwrap_or_else(|| "ext4.vhdx".to_owned());
            found.push(Distribution {
                name: distribution,
                disk: PathBuf::from(base).join(file),
            });
        }
        // SAFETY: opened above, closed once.
        unsafe { RegCloseKey(key) };
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disk_is_known_by_its_path_however_it_is_spelled() {
        let distributions = [
            Distribution {
                name: "docker-desktop".to_owned(),
                disk: PathBuf::from(r"\\?\C:\Users\me\AppData\Local\Docker\wsl\main\ext4.vhdx"),
            },
            Distribution {
                name: "Ubuntu".to_owned(),
                disk: PathBuf::from(r"C:\Users\me\AppData\Local\wsl\{1234}\ext4.vhdx"),
            },
        ];
        assert_eq!(
            name_of(Path::new("c:/users/ME/AppData/Local/Docker/wsl/main/ext4.vhdx"), &distributions),
            Some("docker-desktop")
        );
        assert_eq!(
            name_of(Path::new(r"C:\Users\me\AppData\Local\wsl\{1234}\ext4.vhdx"), &distributions),
            Some("Ubuntu")
        );
        assert_eq!(
            name_of(Path::new(r"C:\Users\me\AppData\Local\Docker\wsl\disk\docker_data.vhdx"), &distributions),
            None
        );
    }
}
