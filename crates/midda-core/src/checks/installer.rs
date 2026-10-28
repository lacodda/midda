//! Whether what an installer installs is installed here.
//!
//! An installer says what it is: a Windows Installer package (`.msi`) in its
//! own tables — the product's name, maker and version, and two codes, one for
//! the product and one shared by every version of it — and a program
//! (`.exe`) in its version resource, the same block Explorer's Details tab
//! shows. The computer says what is installed: Windows Installer by those
//! codes, and the list Settings → Apps shows, which is the registry's
//! `Uninstall` keys for the machine, for 32-bit programs and for the account.
//!
//! The codes are exact, and asked first. A name is matched only as words — a
//! program called `Git` is not "installed" because `GitHub Desktop` is — and
//! only with its maker agreeing, when both say one: a guess that calls an
//! installer of something else "installed" would tell the reader to delete
//! the one copy they have.

use std::cmp::Ordering;
use std::path::Path;

use super::Outcome;

/// A program installed here, as the list of installed programs says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
}

/// What an installer says it installs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub name: Option<String>,
    pub publisher: Option<String>,
    pub version: Option<String>,
    /// A Windows Installer package's product code, `{…}`.
    pub product: Option<String>,
    /// And the code every version of the product shares.
    pub upgrade: Option<String>,
}

/// What a check of an installer found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// What a way back can be taken by; `None` when nothing it installs is
    /// installed, or it does not say what it installs.
    pub outcome: Option<Outcome>,
    /// What was found, as the reader is told it.
    pub about: String,
}

/// What Windows Installer says of its codes; the computer, or a test.
pub trait Codes {
    /// The product installed under this product code, when one is.
    fn product(&self, code: &str) -> Option<Program>;
    /// The products installed under this upgrade code.
    fn related(&self, upgrade: &str) -> Vec<Program>;
}

/// A computer with no Windows Installer to ask.
pub struct NoCodes;

impl Codes for NoCodes {
    fn product(&self, _: &str) -> Option<Program> {
        None
    }

    fn related(&self, _: &str) -> Vec<Program> {
        Vec::new()
    }
}

/// What of `identity` is installed, among `programs` and by `codes`.
#[must_use]
pub fn judge(identity: Option<&Identity>, programs: &[Program], codes: &dyn Codes) -> Verdict {
    let Some(identity) = identity.filter(|identity| identity.name.as_deref().is_some_and(|name| !words_of(name).is_empty()) || identity.product.is_some())
    else {
        return Verdict {
            outcome: None,
            about: "It does not say what it installs.".to_owned(),
        };
    };
    let by_code: Vec<Program> = identity
        .product
        .as_deref()
        .and_then(|code| codes.product(code))
        .into_iter()
        .chain(identity.upgrade.as_deref().map(|code| codes.related(code)).unwrap_or_default())
        .collect();
    let installed: Vec<&Program> = if by_code.is_empty() {
        programs.iter().filter(|program| same_program(identity, program)).collect()
    } else {
        by_code.iter().collect()
    };
    let wanted = identity.name.clone().unwrap_or_else(|| "what it installs".to_owned());
    let Some(nearest) = nearest(&installed, identity.version.as_deref()) else {
        let named = identity
            .version
            .as_deref()
            .map_or_else(|| wanted.clone(), |version| format!("{wanted} {version}"));
        return Verdict {
            outcome: None,
            about: format!("Nothing installed here is {named}."),
        };
    };
    let shown = nearest
        .version
        .as_deref()
        .map_or_else(|| nearest.name.clone(), |version| with_version(&nearest.name, version));
    let order = identity
        .version
        .as_deref()
        .zip(nearest.version.as_deref())
        .and_then(|(own, theirs)| compare(theirs, own));
    let this = identity.version.as_deref().unwrap_or("");
    let (outcome, about) = match order {
        Some(Ordering::Equal) => (Outcome::Installed, format!("{shown} is installed, the version this installs.")),
        Some(Ordering::Greater) => (Outcome::Installed, format!("{shown} is installed, newer than the {this} this installs.")),
        Some(Ordering::Less) => (Outcome::OlderInstalled, format!("{shown} is installed, older than the {this} this installs.")),
        None => (Outcome::Installed, format!("{shown} is installed.")),
    };
    Verdict { outcome: Some(outcome), about }
}

/// `name` and `version` as one: the version said once, though a program's
/// own name often holds it already — `7-Zip 24.08 (x64)`.
fn with_version(name: &str, version: &str) -> String {
    let said = name
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .any(|word| word.contains('.') && compare(word, version) == Some(Ordering::Equal));
    if said { name.to_owned() } else { format!("{name} {version}") }
}

/// The installed one to speak of: the newest that is as new as `version`,
/// else the newest of all — so "installed" is said whenever it is true. The
/// very version, when one of them is it: a runtime lists itself twice, once
/// by the bundle's numbering and once by its package's.
fn nearest<'a>(installed: &[&'a Program], version: Option<&str>) -> Option<&'a Program> {
    let same = version.and_then(|version| {
        installed
            .iter()
            .copied()
            .find(|program| program.version.as_deref().and_then(|theirs| compare(theirs, version)) == Some(Ordering::Equal))
    });
    if same.is_some() {
        return same;
    }
    let newest = |programs: &mut dyn Iterator<Item = &'a Program>| {
        programs.max_by(|a, b| match (a.version.as_deref(), b.version.as_deref()) {
            (Some(a), Some(b)) => compare(a, b).unwrap_or(Ordering::Equal),
            (a, b) => a.is_some().cmp(&b.is_some()),
        })
    };
    let as_new = version.and_then(|version| {
        newest(&mut installed.iter().copied().filter(|program| {
            program
                .version
                .as_deref()
                .is_some_and(|theirs| compare(theirs, version).is_some_and(Ordering::is_ge))
        }))
    });
    as_new.or_else(|| newest(&mut installed.iter().copied()))
}

/// Two versions, number by number — `4.41.2` against `4.41.2.0`, `24.08`
/// against `24.8` — or `None` when either has no number to start with.
#[must_use]
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let parts = |text: &str| -> Option<Vec<u64>> {
        let numbers: Vec<u64> = text
            .trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '_', ' ', '+'])
            .map_while(|part| part.parse().ok())
            .collect();
        (!numbers.is_empty()).then_some(numbers)
    };
    let (mut a, mut b) = (parts(a)?, parts(b)?);
    let long = a.len().max(b.len());
    a.resize(long, 0);
    b.resize(long, 0);
    Some(a.cmp(&b))
}

/// Words that say how a program is packed, not what it is: an installer's
/// name says `Setup`, the installed program's says `(User)`.
const PACKING: [&str; 11] = [
    "setup",
    "installer",
    "install",
    "bootstrapper",
    "web",
    "offline",
    "online",
    // Who it was installed for.
    "user",
    "machine",
    "per-user",
    "bit",
];

/// Which processor a program is built for, by the words that say it.
fn arch_of(word: &str) -> Option<&'static str> {
    match word {
        "x64" | "amd64" | "win64" | "64-bit" | "x86_64" => Some("x64"),
        "x86" | "win32" | "32-bit" | "i386" | "i686" => Some("x86"),
        "arm64" | "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// A program's name as it is matched: the words that say what it is, and
/// the processor, when it says one.
#[derive(Debug, PartialEq, Eq)]
struct Name {
    words: Vec<String>,
    arch: Option<&'static str>,
}

/// What a name says: lower case, without punctuation, versions or packing.
///
/// A version is a run of numbers with dots — `12.0.30501`, `v2.1` — or a
/// bare number; a year is not, nor `v14`: `Visual C++ 2013` and `Visual C++
/// v14` are two programs, installed side by side.
fn name_of(text: &str) -> Name {
    let mut arch = None;
    let words = text
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '+' | '#' | '-' | '.' | '_')))
        .map(|word| word.trim_matches(['-', '.', '_']).to_lowercase())
        .filter(|word| !word.is_empty())
        .filter(|word| match arch_of(word) {
            Some(found) => {
                arch = arch.or(Some(found));
                false
            }
            None => true,
        })
        .filter(|word| !is_version(word))
        .filter(|word| !PACKING.contains(&word.as_str()))
        .collect();
    Name { words, arch }
}

/// Whether `word` is a version rather than a part of a name.
fn is_version(word: &str) -> bool {
    let bare = word.strip_prefix('v').unwrap_or(word);
    let numbers: Vec<&str> = bare.split('.').collect();
    if !numbers.iter().all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    let year = numbers.len() == 1 && bare.len() == 4 && (bare.starts_with("19") || bare.starts_with("20"));
    let generation = numbers.len() == 1 && word.starts_with('v');
    !year && !generation
}

/// The words of a name that say what it is.
fn words_of(text: &str) -> Vec<String> {
    name_of(text).words
}

/// Words that say what kind of company, not which.
const COMPANY: [&str; 12] = [
    "the",
    "inc",
    "corp",
    "corporation",
    "ltd",
    "llc",
    "gmbh",
    "co",
    "company",
    "limited",
    "foundation",
    "s",
];

/// The first word of a maker's name that says which maker.
fn maker_of(publisher: &str) -> Option<String> {
    words_of(publisher).into_iter().find(|word| !COMPANY.contains(&word.as_str()))
}

/// Whether `program` is what `identity` installs: the makers agree when both
/// are said, and the names are the same words — once the maker's own name
/// and how the program is packed are left out of both. `Microsoft Visual
/// Studio Code (User)` is what `Visual Studio Code` installs; it is not what
/// `Microsoft Visual Studio` does, though every word of that is in it.
fn same_program(identity: &Identity, program: &Program) -> bool {
    let Some(name) = identity.name.as_deref() else {
        return false;
    };
    let ours_maker = identity.publisher.as_deref().and_then(maker_of);
    let theirs_maker = program.publisher.as_deref().and_then(maker_of);
    let maker = match (ours_maker, theirs_maker) {
        (Some(ours), Some(theirs)) if ours != theirs => return false,
        (Some(ours), Some(_)) => Some(ours),
        _ => None,
    };
    let ours = core_of(name, maker.as_deref());
    let theirs = core_of(&program.name, maker.as_deref());
    // A processor said on both sides must be the same: the x86 and x64
    // runtimes are two programs, and often both installed.
    let arch = ours.arch.zip(theirs.arch).is_none_or(|(ours, theirs)| ours == theirs);
    !ours.words.is_empty() && ours.words == theirs.words && arch
}

/// What `name` says of the program: its words without `maker`, unless the
/// maker's name is all it says — Python's maker is the Python Software
/// Foundation.
fn core_of(name: &str, maker: Option<&str>) -> Name {
    let Name { words, arch } = name_of(name);
    let without: Vec<String> = words.iter().filter(|word| Some(word.as_str()) != maker).cloned().collect();
    Name {
        words: if without.is_empty() { words } else { without },
        arch,
    }
}

/// What of the installer at `path` is installed, among `programs` and by
/// what Windows Installer says of its codes.
#[must_use]
pub fn look(path: &Path, programs: &[Program]) -> Verdict {
    let identity = platform::identify(path);
    judge(identity.as_ref(), programs, &platform::Installer)
}

/// The programs installed on this computer: none on another platform.
#[must_use]
pub fn installed() -> Vec<Program> {
    platform::installed()
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    pub(super) use super::NoCodes as Installer;
    use super::{Identity, Program};

    pub(super) fn identify(_: &Path) -> Option<Identity> {
        None
    }

    pub(super) fn installed() -> Vec<Program> {
        Vec::new()
    }
}

#[cfg(windows)]
#[allow(unsafe_code, reason = "installers and the list of installed programs are read through Windows' own calls")]
mod platform {
    use std::ffi::{OsStr, OsString, c_void};
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
    use std::path::Path;

    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows_sys::Win32::System::ApplicationInstallationAndServicing::{
        INSTALLSTATE_DEFAULT, MSIDBOPEN_READONLY, MSIHANDLE, MsiCloseHandle, MsiDatabaseOpenViewW, MsiEnumRelatedProductsW, MsiGetProductInfoW,
        MsiOpenDatabaseW, MsiQueryProductStateW, MsiRecordGetStringW, MsiViewExecute, MsiViewFetch,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
        RegOpenKeyExW,
    };

    use super::{Codes, Identity, Program};

    fn wide(text: &str) -> Vec<u16> {
        OsStr::new(text).encode_wide().chain(Some(0)).collect()
    }

    fn from_wide(units: &[u16]) -> String {
        let end = units.iter().position(|&unit| unit == 0).unwrap_or(units.len());
        OsString::from_wide(&units[..end]).to_string_lossy().into_owned()
    }

    fn said(text: String) -> Option<String> {
        let text = text.trim().to_owned();
        (!text.is_empty()).then_some(text)
    }

    /// A handle Windows Installer gave, closed when dropped.
    struct Handle(MSIHANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            if self.0 != 0 {
                // SAFETY: a handle Windows Installer opened, closed once.
                unsafe { MsiCloseHandle(self.0) };
            }
        }
    }

    /// The `Property` table of a package, for the names asked.
    fn properties(path: &Path, names: &[&str]) -> Option<Vec<Option<String>>> {
        let file: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut database = Handle(0);
        // SAFETY: the path ends in a zero; read only, so the package is not
        // changed; the handle is closed by `Handle`.
        if unsafe { MsiOpenDatabaseW(file.as_ptr(), MSIDBOPEN_READONLY, &raw mut database.0) } != ERROR_SUCCESS {
            return None;
        }
        let mut values = Vec::with_capacity(names.len());
        for name in names {
            // The names are this module's own, plain words: nothing to quote.
            let query = wide(&format!("SELECT `Value` FROM `Property` WHERE `Property` = '{name}'"));
            let mut view = Handle(0);
            let mut record = Handle(0);
            // SAFETY: the database is open; the query ends in a zero.
            let found = unsafe {
                MsiDatabaseOpenViewW(database.0, query.as_ptr(), &raw mut view.0) == ERROR_SUCCESS
                    && MsiViewExecute(view.0, 0) == ERROR_SUCCESS
                    && MsiViewFetch(view.0, &raw mut record.0) == ERROR_SUCCESS
            };
            values.push(found.then(|| record_text(&record)).flatten());
        }
        Some(values)
    }

    /// The first field of a record.
    fn record_text(record: &Handle) -> Option<String> {
        let mut buffer = vec![0_u16; 256];
        loop {
            let mut length = u32::try_from(buffer.len()).ok()?;
            // SAFETY: the record is open and the buffer as long as said.
            let status = unsafe { MsiRecordGetStringW(record.0, 1, buffer.as_mut_ptr(), &raw mut length) };
            match status {
                ERROR_SUCCESS => return said(from_wide(&buffer)),
                ERROR_MORE_DATA => buffer.resize(usize::try_from(length).ok()? + 1, 0),
                _ => return None,
            }
        }
    }

    /// What the version resource of a program says, by the names asked —
    /// in the first language it is written in.
    fn version_strings(path: &Path, names: &[&str]) -> Option<Vec<Option<String>>> {
        let file: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut ignored = 0;
        // SAFETY: the path ends in a zero. Reading the resource maps the file
        // as data; nothing in it runs.
        let size = unsafe { GetFileVersionInfoSizeW(file.as_ptr(), &raw mut ignored) };
        if size == 0 {
            return None;
        }
        let mut block = vec![0_u8; usize::try_from(size).ok()?];
        // SAFETY: the block is as long as the size asked for.
        if unsafe { GetFileVersionInfoW(file.as_ptr(), 0, size, block.as_mut_ptr().cast()) } == 0 {
            return None;
        }
        let query = |sub: &str| -> Option<(*const c_void, u32)> {
            let sub = wide(sub);
            let mut value: *mut c_void = std::ptr::null_mut();
            let mut length = 0;
            // SAFETY: the block was filled above; what comes back points
            // into it, and is read before the block goes.
            let found = unsafe { VerQueryValueW(block.as_ptr().cast(), sub.as_ptr(), &raw mut value, &raw mut length) };
            (found != 0 && !value.is_null() && length > 0).then_some((value.cast_const(), length))
        };
        let language = query(r"\VarFileInfo\Translation")
            .filter(|&(_, length)| length >= 4)
            .map(|(value, _)| {
                // SAFETY: four bytes at least, as the length said: a language
                // and a code page.
                let pair = unsafe { std::slice::from_raw_parts(value.cast::<u16>(), 2) };
                format!("{:04x}{:04x}", pair[0], pair[1])
            })
            .unwrap_or_else(|| "040904b0".to_owned());
        Some(
            names
                .iter()
                .map(|name| {
                    query(&format!(r"\StringFileInfo\{language}\{name}")).and_then(|(value, length)| {
                        // SAFETY: `length` characters of UTF-16 inside the block.
                        let units = unsafe { std::slice::from_raw_parts(value.cast::<u16>(), usize::try_from(length).ok()?) };
                        said(from_wide(units))
                    })
                })
                .collect(),
        )
    }

    pub(super) fn identify(path: &Path) -> Option<Identity> {
        let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
        if extension == "msi" {
            let mut values = properties(path, &["ProductName", "Manufacturer", "ProductVersion", "ProductCode", "UpgradeCode"])?.into_iter();
            let mut next = || values.next().flatten();
            return Some(Identity {
                name: next(),
                publisher: next(),
                version: next(),
                product: next(),
                upgrade: next(),
            });
        }
        let mut values = version_strings(path, &["ProductName", "CompanyName", "ProductVersion", "FileVersion"])?.into_iter();
        let mut next = || values.next().flatten();
        let (name, publisher, product_version, file_version) = (next(), next(), next(), next());
        Some(Identity {
            name,
            publisher,
            version: product_version.or(file_version),
            product: None,
            upgrade: None,
        })
    }

    /// Windows Installer, asked of its codes.
    pub(super) struct Installer;

    /// What Windows Installer says of an installed product, by name.
    fn product_info(code: &[u16], attribute: &str) -> Option<String> {
        let attribute = wide(attribute);
        let mut buffer = vec![0_u16; 256];
        let mut length = u32::try_from(buffer.len()).ok()?;
        // SAFETY: both end in a zero and the buffer is as long as said.
        let status = unsafe { MsiGetProductInfoW(code.as_ptr(), attribute.as_ptr(), buffer.as_mut_ptr(), &raw mut length) };
        (status == ERROR_SUCCESS).then(|| said(from_wide(&buffer))).flatten()
    }

    fn product_of(code: &[u16]) -> Option<Program> {
        // SAFETY: the code ends in a zero.
        if unsafe { MsiQueryProductStateW(code.as_ptr()) } != INSTALLSTATE_DEFAULT {
            return None;
        }
        Some(Program {
            name: product_info(code, "InstalledProductName").or_else(|| product_info(code, "ProductName"))?,
            version: product_info(code, "VersionString"),
            publisher: product_info(code, "Publisher"),
        })
    }

    impl Codes for Installer {
        fn product(&self, code: &str) -> Option<Program> {
            product_of(&wide(code))
        }

        fn related(&self, upgrade: &str) -> Vec<Program> {
            let upgrade = wide(upgrade);
            let mut found = Vec::new();
            for index in 0.. {
                // A product code: 38 characters and the zero.
                let mut code = [0_u16; 39];
                // SAFETY: the upgrade code ends in a zero; the buffer holds a
                // code, as the call requires.
                let status = unsafe { MsiEnumRelatedProductsW(upgrade.as_ptr(), 0, index, code.as_mut_ptr()) };
                if status != ERROR_SUCCESS {
                    break;
                }
                found.extend(product_of(&code));
            }
            found
        }
    }

    /// A string value of `subkey` under `key`.
    fn text(key: HKEY, subkey: &[u16], value: &str) -> Option<String> {
        let value = wide(value);
        let mut buffer = vec![0_u16; 512];
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
        said(from_wide(&buffer[..units.min(buffer.len())]))
    }

    /// The programs one `Uninstall` key lists.
    fn listed(root: HKEY, view: u32, into: &mut Vec<Program>) {
        let list = wide(r"Software\Microsoft\Windows\CurrentVersion\Uninstall");
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: the name ends in a zero; the key is closed below.
        if unsafe { RegOpenKeyExW(root, list.as_ptr(), 0, KEY_READ | view, &raw mut key) } != ERROR_SUCCESS {
            return;
        }
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
            if let Some(display) = text(key, &subkey, "DisplayName") {
                into.push(Program {
                    name: display,
                    version: text(key, &subkey, "DisplayVersion"),
                    publisher: text(key, &subkey, "Publisher"),
                });
            }
        }
        // SAFETY: opened above, closed once.
        unsafe { RegCloseKey(key) };
    }

    pub(super) fn installed() -> Vec<Program> {
        let mut programs = Vec::new();
        listed(HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, &mut programs);
        listed(HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, &mut programs);
        listed(HKEY_CURRENT_USER, 0, &mut programs);
        programs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(name: &str, version: Option<&str>, publisher: Option<&str>) -> Program {
        Program {
            name: name.to_owned(),
            version: version.map(str::to_owned),
            publisher: publisher.map(str::to_owned),
        }
    }

    fn identity(name: &str, version: Option<&str>, publisher: Option<&str>) -> Identity {
        Identity {
            name: Some(name.to_owned()),
            version: version.map(str::to_owned),
            publisher: publisher.map(str::to_owned),
            ..Identity::default()
        }
    }

    fn machine() -> Vec<Program> {
        vec![
            program("7-Zip 24.08 (x64)", Some("24.08"), Some("Igor Pavlov")),
            program("Docker Desktop", Some("4.41.2"), Some("Docker Inc.")),
            program("GitHub Desktop", Some("3.4.9"), Some("GitHub, Inc.")),
            program("Microsoft Visual Studio Code (User)", Some("1.95.0"), Some("Microsoft Corporation")),
            program("Python 3.12.4 (64-bit)", Some("3.12.4150.0"), Some("Python Software Foundation")),
        ]
    }

    fn judged(identity: &Identity) -> Verdict {
        judge(Some(identity), &machine(), &NoCodes)
    }

    #[test]
    fn the_same_program_in_the_same_or_a_newer_version_is_installed() {
        let docker = judged(&identity("Docker Desktop Installer", Some("4.41.2.0"), Some("Docker Inc.")));
        assert_eq!(docker.outcome, Some(Outcome::Installed));
        assert_eq!(docker.about, "Docker Desktop 4.41.2 is installed, the version this installs.");

        let old = judged(&identity("7-Zip", Some("23.01"), Some("Igor Pavlov")));
        assert_eq!(old.outcome, Some(Outcome::Installed));
        assert_eq!(old.about, "7-Zip 24.08 (x64) is installed, newer than the 23.01 this installs.");

        let code = judged(&identity("Visual Studio Code", Some("1.96.0"), Some("Microsoft Corporation")));
        assert_eq!(code.outcome, Some(Outcome::OlderInstalled), "an update not yet run");
        assert!(code.about.ends_with("older than the 1.96.0 this installs."), "{}", code.about);
    }

    #[test]
    fn a_name_is_matched_as_whole_words_and_with_the_maker() {
        let git = judged(&identity("Git", Some("2.47.0"), Some("The Git Development Community")));
        assert_eq!(git.outcome, None, "GitHub Desktop is not Git");
        assert_eq!(git.about, "Nothing installed here is Git 2.47.0.");

        let other_maker = judged(&identity("Docker Desktop", Some("4.41.2"), Some("Someone Else")));
        assert_eq!(other_maker.outcome, None, "the same name from another maker is another program");

        let other_program = judged(&identity("Microsoft Visual Studio", Some("17.12"), Some("Microsoft Corporation")));
        assert_eq!(
            other_program.outcome, None,
            "every word of it is in Visual Studio Code, which is another program"
        );
        let silent = judged(&identity("Visual Studio Code", Some("1.95.0"), None));
        assert_eq!(silent.outcome, None, "without a maker to agree, a part of a name is not enough");
        let silent_whole = judged(&identity("Docker Desktop", None, None));
        assert_eq!(silent_whole.outcome, Some(Outcome::Installed), "the same words are");
        assert_eq!(silent_whole.about, "Docker Desktop 4.41.2 is installed.");
    }

    #[test]
    fn a_year_a_generation_and_a_processor_are_parts_of_a_name() {
        // As this computer listed its runtimes.
        let microsoft = Some("Microsoft Corporation");
        let runtimes = vec![
            program("Microsoft Visual C++ 2013 Redistributable (x64) - 12.0.30501", Some("12.0.30501.0"), microsoft),
            program("Microsoft Visual C++ v14 Redistributable (x64) - 14.50.35719", Some("14.50.35719.0"), microsoft),
            program("Microsoft Windows Desktop Runtime - 8.0.4 (x64)", Some("64.16.12024"), microsoft),
            program("Microsoft Windows Desktop Runtime - 8.0.4 (x64)", Some("8.0.4.33519"), microsoft),
        ];
        let judged = |name: &str, version: &str| judge(Some(&identity(name, Some(version), microsoft)), &runtimes, &NoCodes);

        let older = judged("Microsoft Visual C++ 2013 Redistributable (x64) - 12.0.30501", "12.0.30501.0");
        assert_eq!(
            older.about, "Microsoft Visual C++ 2013 Redistributable (x64) - 12.0.30501 is installed, the version this installs.",
            "not v14"
        );

        let x86 = judged("Microsoft Visual C++ v14 Redistributable (x86) - 14.50.35719", "14.50.35719.0");
        assert_eq!(x86.outcome, None, "only the x64 one is installed: {}", x86.about);
        let untold = judged("Microsoft Visual C++ v14 Redistributable", "14.50.35719.0");
        assert_eq!(
            untold.outcome,
            Some(Outcome::Installed),
            "a processor said on one side only says nothing against"
        );

        let studio = judge(
            Some(&identity("Visual Studio Community 2019", Some("16.11.0"), microsoft)),
            &[program("Visual Studio Community 2022", Some("17.12.0"), microsoft)],
            &NoCodes,
        );
        assert_eq!(studio.outcome, None, "2019 is not an older 2022: {}", studio.about);
        let generation = judge(
            Some(&identity("Microsoft Visual C++ v14 Redistributable (x64)", Some("14.50"), microsoft)),
            &[program("Microsoft Visual C++ Redistributable (x64)", Some("9.0"), microsoft)],
            &NoCodes,
        );
        assert_eq!(generation.outcome, None, "v14 names a generation: {}", generation.about);

        let runtime = judged("Microsoft Windows Desktop Runtime - 8.0.4 (x64)", "8.0.4.33519");
        assert_eq!(
            runtime.about,
            "Microsoft Windows Desktop Runtime - 8.0.4 (x64) 8.0.4.33519 is installed, the version this installs."
        );
    }

    #[test]
    fn an_installer_that_says_nothing_is_not_installed() {
        for nameless in [None, Some(identity("Setup", Some("1.0"), Some("Docker Inc."))), Some(Identity::default())] {
            let verdict = judge(nameless.as_ref(), &machine(), &NoCodes);
            assert_eq!(verdict.outcome, None);
            assert_eq!(verdict.about, "It does not say what it installs.");
        }
    }

    struct Codes2 {
        product: Option<Program>,
        related: Vec<Program>,
    }

    impl Codes for Codes2 {
        fn product(&self, _: &str) -> Option<Program> {
            self.product.clone()
        }

        fn related(&self, _: &str) -> Vec<Program> {
            self.related.clone()
        }
    }

    #[test]
    fn a_package_s_codes_are_asked_before_any_name() {
        let package = Identity {
            name: Some("Node.js".to_owned()),
            version: Some("22.23.4".to_owned()),
            product: Some("{A}".to_owned()),
            upgrade: Some("{B}".to_owned()),
            ..Identity::default()
        };
        let installed = Codes2 {
            product: Some(program("Node.js", Some("22.23.4"), None)),
            related: Vec::new(),
        };
        assert_eq!(
            judge(Some(&package), &[], &installed).outcome,
            Some(Outcome::Installed),
            "by its product code, with no list"
        );

        let upgraded = Codes2 {
            product: None,
            related: vec![
                program("Node.js", Some("23.0.0"), None),
                program("Node.js", Some("20.1.0"), None),
                program("Node.js", Some("24.0.0"), None),
            ],
        };
        let verdict = judge(Some(&package), &[], &upgraded);
        assert_eq!(verdict.outcome, Some(Outcome::Installed), "a newer version shares the upgrade code");
        assert_eq!(verdict.about, "Node.js 24.0.0 is installed, newer than the 22.23.4 this installs.");

        let nothing = judge(Some(&package), &[], &NoCodes);
        assert_eq!(nothing.about, "Nothing installed here is Node.js 22.23.4.");
    }

    #[test]
    fn versions_compare_number_by_number() {
        assert_eq!(compare("4.41.2", "4.41.2.0"), Some(Ordering::Equal));
        assert_eq!(compare("24.08", "24.8"), Some(Ordering::Equal));
        assert_eq!(compare("1.10", "1.9"), Some(Ordering::Greater));
        assert_eq!(compare("v2.0", "1.99"), Some(Ordering::Greater));
        assert_eq!(compare("3.12.4150.0", "3.12.4"), Some(Ordering::Greater));
        assert_eq!(compare("beta", "1.0"), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_program_s_version_resource_is_read() {
        let windows = std::env::var_os("SystemRoot").expect("Windows says where it is");
        let kernel = Path::new(&windows).join(r"System32\kernel32.dll");
        let identity = platform::identify(&kernel).expect("kernel32 has a version resource");
        assert!(identity.name.as_deref().is_some_and(|name| name.contains("Windows")), "{identity:?}");
        assert!(
            identity
                .version
                .as_deref()
                .and_then(|version| compare(version, "6.0"))
                .is_some_and(Ordering::is_ge)
        );
        assert!(!installed().is_empty(), "a Windows has programs installed");
    }
}
