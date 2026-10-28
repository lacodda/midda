//! Where a shortcut points, read from the file itself.
//!
//! A `.lnk` is a Shell Link ([MS-SHLLINK]): a header, then — each when a flag
//! says so — the shell's own list of items, where the target is (`LinkInfo`:
//! the path on a local volume, or a share), a few strings (among them the
//! target relative to the shortcut), and blocks of extra data (among them the
//! target written with variables, `%ProgramFiles%\…`). midda reads the paths
//! and nothing else, with its own code: the format is small and fixed, and
//! the shell's `IShellLink` would be COM, a thread of its own, and a
//! `Resolve` that searches the disk and may rewrite the shortcut.
//!
//! A shortcut is broken when every path it gives is a path on a volume that
//! is here, and nothing is at any of them. Anything less certain is not
//! broken: a target on a share (asking could wait on the network), on a
//! drive not connected, behind a folder this account may not read, or one
//! only the shell's item list names — a Control Panel page, a Store app.
//! An advertised shortcut (Windows Installer's) names a product, not a
//! file, and is never broken here.
//!
//! [MS-SHLLINK]: https://learn.microsoft.com/openspecs/windows_protocols/ms-shllink/

use std::path::{Component, Path, PathBuf};

use super::Seen;

/// The paths a shortcut gives for its target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Link {
    /// Where `LinkInfo` says the target is on a local volume.
    pub local: Option<String>,
    /// The target with variables in it, as the extra data writes it.
    pub environment: Option<String>,
    /// The target relative to the shortcut's own folder.
    pub relative: Option<String>,
    /// `LinkInfo` names a share.
    pub network: bool,
    /// Windows Installer's: the target is the product's to find.
    pub advertised: bool,
}

/// The header's size and class, which every shell link starts with.
const HEADER: usize = 0x4C;
const CLASS: [u8; 16] = [0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];

// LinkFlags.
const HAS_ID_LIST: u32 = 0x1;
const HAS_LINK_INFO: u32 = 0x2;
const HAS_NAME: u32 = 0x4;
const HAS_RELATIVE_PATH: u32 = 0x8;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const HAS_ICON_LOCATION: u32 = 0x40;
const IS_UNICODE: u32 = 0x80;
const FORCE_NO_LINK_INFO: u32 = 0x100;
const HAS_DARWIN_ID: u32 = 0x1000;

// LinkInfoFlags.
const VOLUME_AND_LOCAL_PATH: u32 = 0x1;
const NETWORK_AND_SUFFIX: u32 = 0x2;

/// The extra data block that holds the target with variables in it.
const ENVIRONMENT_BLOCK: u32 = 0xA000_0001;
const ENVIRONMENT_SIZE: usize = 0x314;

/// Bytes of a shortcut, read with the bounds checked.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn u16(&self, at: usize) -> Result<u16, String> {
        self.0.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(truncated)
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        self.0
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(truncated)
    }

    fn offset(&self, at: usize) -> Result<usize, String> {
        usize::try_from(self.u32(at)?).map_err(|_| truncated())
    }

    /// A string of single bytes up to a zero, starting at `at` and ending
    /// before `end`. Only when it is ASCII: the bytes are in the code page of
    /// the computer that made the shortcut, which is not this one's to know,
    /// and a path decoded in the wrong one names something else.
    fn narrow(&self, at: usize, end: usize) -> Result<Option<String>, String> {
        let bytes = self.0.get(at..end.min(self.0.len())).ok_or_else(truncated)?;
        let text = &bytes[..bytes.iter().position(|&b| b == 0).ok_or_else(truncated)?];
        Ok(text.is_ascii().then(|| String::from_utf8_lossy(text).into_owned()))
    }

    /// A string of UTF-16 units up to a zero, starting at `at` and ending
    /// before `end`.
    fn wide(&self, at: usize, end: usize) -> Result<String, String> {
        let bytes = self.0.get(at..end.min(self.0.len())).ok_or_else(truncated)?;
        let units = units_of(bytes);
        let end = units.iter().position(|&unit| unit == 0).ok_or_else(truncated)?;
        Ok(String::from_utf16_lossy(&units[..end]))
    }
}

/// Little-endian UTF-16 units; an odd byte at the end is no unit.
fn units_of(bytes: &[u8]) -> Vec<u16> {
    bytes.as_chunks::<2>().0.iter().map(|&pair| u16::from_le_bytes(pair)).collect()
}

fn truncated() -> String {
    "the shortcut ends before what it says is in it".to_owned()
}

/// Reads the paths out of a shortcut's bytes.
///
/// # Errors
///
/// When the bytes are not a shell link, or end before what they say is in
/// them.
pub fn parse(bytes: &[u8]) -> Result<Link, String> {
    let data = Bytes(bytes);
    if data.u32(0)? as usize != HEADER || bytes.get(4..20) != Some(&CLASS[..]) {
        return Err("not a shell link".to_owned());
    }
    let flags = data.u32(20)?;
    let mut link = Link {
        advertised: flags & HAS_DARWIN_ID != 0,
        ..Link::default()
    };
    let mut at = HEADER;
    if flags & HAS_ID_LIST != 0 {
        at += 2 + usize::from(data.u16(at)?);
    }
    if flags & HAS_LINK_INFO != 0 && flags & FORCE_NO_LINK_INFO == 0 {
        let size = data.offset(at)?;
        let end = at.checked_add(size).filter(|&end| end <= bytes.len()).ok_or_else(truncated)?;
        let header = data.offset(at + 4)?;
        let info = data.u32(at + 8)?;
        if info & VOLUME_AND_LOCAL_PATH != 0 {
            let base = if header >= 0x24 {
                Some(data.wide(at + data.offset(at + 28)?, end)?)
            } else {
                data.narrow(at + data.offset(at + 16)?, end)?
            };
            let suffix = if header >= 0x24 {
                Some(data.wide(at + data.offset(at + 32)?, end)?)
            } else {
                data.narrow(at + data.offset(at + 24)?, end)?
            };
            link.local = base.zip(suffix).map(|(base, suffix)| base + &suffix);
        }
        link.network = info & NETWORK_AND_SUFFIX != 0;
        at = end;
    }
    let unicode = flags & IS_UNICODE != 0;
    for (flag, keep) in [
        (HAS_NAME, false),
        (HAS_RELATIVE_PATH, true),
        (HAS_WORKING_DIR, false),
        (HAS_ARGUMENTS, false),
        (HAS_ICON_LOCATION, false),
    ] {
        if flags & flag == 0 {
            continue;
        }
        let count = usize::from(data.u16(at)?);
        let bytes_long = if unicode { count * 2 } else { count };
        let start = at + 2;
        let end = start + bytes_long;
        let text = bytes.get(start..end).ok_or_else(truncated)?;
        if keep {
            link.relative = Some(if unicode {
                String::from_utf16_lossy(&units_of(text))
            } else if text.is_ascii() {
                String::from_utf8_lossy(text).into_owned()
            } else {
                String::new()
            })
            .filter(|relative| !relative.is_empty());
        }
        at = end;
    }
    // Extra data, to the terminal block: a size under four.
    while let Ok(size) = data.offset(at) {
        if size < 8 {
            break;
        }
        let end = at.checked_add(size).filter(|&end| end <= bytes.len()).ok_or_else(truncated)?;
        if data.u32(at + 4)? == ENVIRONMENT_BLOCK && size >= ENVIRONMENT_SIZE {
            let unicode = data.wide(at + 8 + 260, end)?;
            link.environment = Some(unicode).filter(|target| !target.is_empty());
        }
        at = end;
    }
    Ok(link)
}

/// What is at a path, as far as it can be told without waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Exists,
    Missing,
    /// Not on a volume that is here and local, or the answer was not "there
    /// is nothing": a folder this account may not read is not empty.
    Unknown,
}

/// Whether `link`, a shortcut at `at`, is broken — with `vars` for the
/// variables in its target and `probe` asking what is at a path.
#[must_use]
pub fn judge(link: &Link, at: &Path, vars: &dyn Fn(&str) -> Option<String>, probe: &dyn Fn(&Path) -> Probe) -> Seen {
    if link.advertised || (link.network && link.local.is_none()) {
        return Seen::Fine;
    }
    let named: Vec<String> = [link.local.clone(), link.environment.as_deref().and_then(|target| expand(target, vars))]
        .into_iter()
        .flatten()
        .collect();
    let at = at.to_string_lossy();
    let relative = link.relative.as_deref().and_then(|relative| {
        let folder = &at[..at.rfind(['\\', '/'])?];
        Some(format!("{folder}\\{relative}"))
    });
    let mut missing = None;
    for target in named.iter().chain(relative.iter()) {
        if !whole(target) {
            continue;
        }
        match probe(Path::new(target)) {
            Probe::Exists | Probe::Unknown => return Seen::Fine,
            Probe::Missing => missing = missing.or(Some(target)),
        }
    }
    // A shortcut is called broken by a path it names itself, the way its
    // properties show it - not by the relative one worked out here.
    match (missing, named.first()) {
        (Some(_), Some(target)) => Seen::Broken { target: target.clone() },
        _ => Seen::Fine,
    }
}

/// Whether `target` is a whole path, as a shortcut writes one: a drive and a
/// separator, or a share. Read as text, not by this platform's `Path`: a
/// shortcut's paths are Windows paths wherever they are read — and on
/// another platform, its own absolute paths are whole too.
fn whole(target: &str) -> bool {
    let bytes = target.as_bytes();
    let drive = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/');
    drive || target.starts_with(r"\\") || (cfg!(not(windows)) && target.starts_with('/'))
}

/// `%NAME%` in `text` replaced by what `vars` says; `None` when a name has
/// no value, since a path with a gap in it names nothing.
fn expand(text: &str, vars: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        out.push_str(&vars(&after[..end])?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// The variables of the profile a shortcut sits in, where it sits in one —
/// `%USERPROFILE%` in a shortcut on another account's Desktop is that
/// account's folder, not the reader's — and the process's otherwise.
fn variables(at: &Path) -> impl Fn(&str) -> Option<String> + '_ {
    let profile = profile_of(at);
    move |name: &str| {
        if let Some(profile) = &profile {
            let under = |tail: &str| Some(profile.join(tail).to_string_lossy().into_owned());
            match name.to_ascii_uppercase().as_str() {
                "USERPROFILE" => return Some(profile.to_string_lossy().into_owned()),
                "APPDATA" => return under(r"AppData\Roaming"),
                "LOCALAPPDATA" => return under(r"AppData\Local"),
                _ => {}
            }
        }
        std::env::var(name).ok()
    }
}

/// `X:\Users\name` when `at` is under it.
fn profile_of(at: &Path) -> Option<PathBuf> {
    let mut components = at.components();
    let mut profile = PathBuf::new();
    for component in components.by_ref() {
        profile.push(component);
        if let Component::Normal(_) = component {
            break;
        }
    }
    let users = profile.file_name()?.to_string_lossy().eq_ignore_ascii_case("Users");
    let name = components.next()?;
    (users && matches!(name, Component::Normal(_))).then(|| profile.join(name))
}

/// What is at `path`, on this computer.
fn probe(path: &Path) -> Probe {
    if !platform::local_volume(path) {
        return Probe::Unknown;
    }
    match std::fs::symlink_metadata(path) {
        Ok(_) => Probe::Exists,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
        Err(_) => Probe::Unknown,
    }
}

/// Whether the shortcut at `path` points to nothing. A file that cannot be
/// read, or read as a shortcut, is none of the rule's: unknown is not
/// broken.
#[must_use]
pub fn look(path: &Path) -> Seen {
    /// A shortcut is a few kilobytes; one larger than this is not one.
    const LARGEST: u64 = 1 << 20;
    let read = std::fs::File::open(path).and_then(|file| {
        use std::io::Read as _;
        let mut bytes = Vec::new();
        file.take(LARGEST).read_to_end(&mut bytes).map(|_| bytes)
    });
    match read.ok().and_then(|bytes| parse(&bytes).ok()) {
        Some(link) => judge(&link, path, &variables(path), &probe),
        None => Seen::Fine,
    }
}

#[cfg(windows)]
#[allow(unsafe_code, reason = "whether a drive is local is GetDriveTypeW's to say")]
mod platform {
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::{Component, Path, Prefix};

    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    // What GetDriveTypeW answers for a drive that is here and not on the
    // network: removable, fixed, a RAM disk.
    const REMOVABLE: u32 = 2;
    const FIXED: u32 = 3;
    const RAM_DISK: u32 = 6;

    /// Whether `path` is on a drive letter that is here, and is not a share
    /// or a disc: asking what is on a share can wait on the network.
    pub(super) fn local_volume(path: &Path) -> bool {
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return false;
        };
        let letter = match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
            _ => return false,
        };
        let root: Vec<u16> = std::ffi::OsStr::new(&format!("{}:\\", char::from(letter)))
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: the root ends in a zero.
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        matches!(kind, REMOVABLE | FIXED | RAM_DISK)
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    pub(super) fn local_volume(path: &Path) -> bool {
        path.is_absolute()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()
    }

    /// What a shortcut is made of, for [`bytes_of`].
    #[derive(Default)]
    struct Made<'a> {
        id_list: bool,
        local: Option<&'a str>,
        /// Write the local path in `LinkInfo`'s Unicode fields as well.
        unicode_info: bool,
        network: bool,
        name: Option<&'a str>,
        relative: Option<&'a str>,
        environment: Option<&'a str>,
        advertised: bool,
    }

    /// A shortcut's bytes, laid out as MS-SHLLINK says.
    fn bytes_of(made: &Made<'_>) -> Vec<u8> {
        let mut flags = IS_UNICODE;
        flags |= if made.id_list { HAS_ID_LIST } else { 0 };
        flags |= if made.local.is_some() || made.network { HAS_LINK_INFO } else { 0 };
        flags |= if made.name.is_some() { HAS_NAME } else { 0 };
        flags |= if made.relative.is_some() { HAS_RELATIVE_PATH } else { 0 };
        flags |= if made.advertised { HAS_DARWIN_ID } else { 0 };
        let mut out = Vec::new();
        out.extend_from_slice(&(HEADER as u32).to_le_bytes());
        out.extend_from_slice(&CLASS);
        out.extend_from_slice(&flags.to_le_bytes());
        out.resize(HEADER, 0);
        if made.id_list {
            // One item of six bytes, then the terminating zero.
            out.extend_from_slice(&8u16.to_le_bytes());
            out.extend_from_slice(&[6, 0, 0x1F, 0x50, 0, 0, 0, 0]);
        }
        if made.local.is_some() || made.network {
            let header: u32 = if made.unicode_info { 0x24 } else { 0x1C };
            let base = made.local.unwrap_or("");
            let mut info = vec![0u8; header as usize];
            let volume = info.len();
            info.extend_from_slice(&[0x10, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0x10, 0, 0, 0]);
            let narrow = info.len();
            info.extend_from_slice(base.as_bytes());
            info.push(0);
            let suffix = info.len();
            info.push(0);
            let (wide, wide_suffix) = (info.len(), info.len() + utf16(base).len());
            if made.unicode_info {
                info.extend_from_slice(&utf16(base));
                info.extend_from_slice(&utf16(""));
            }
            let size = info.len() as u32;
            let put = |info: &mut Vec<u8>, at: usize, value: u32| info[at..at + 4].copy_from_slice(&value.to_le_bytes());
            put(&mut info, 0, size);
            put(&mut info, 4, header);
            put(
                &mut info,
                8,
                if made.local.is_some() { VOLUME_AND_LOCAL_PATH } else { 0 } | if made.network { NETWORK_AND_SUFFIX } else { 0 },
            );
            put(&mut info, 12, volume as u32);
            put(&mut info, 16, narrow as u32);
            put(&mut info, 24, suffix as u32);
            if made.unicode_info {
                put(&mut info, 28, wide as u32);
                put(&mut info, 32, wide_suffix as u32);
            }
            out.extend_from_slice(&info);
        }
        for text in [made.name, made.relative].into_iter().flatten() {
            let units: Vec<u16> = text.encode_utf16().collect();
            out.extend_from_slice(&(units.len() as u16).to_le_bytes());
            out.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
        }
        if let Some(target) = made.environment {
            let mut block = vec![0u8; ENVIRONMENT_SIZE];
            block[..4].copy_from_slice(&(ENVIRONMENT_SIZE as u32).to_le_bytes());
            block[4..8].copy_from_slice(&ENVIRONMENT_BLOCK.to_le_bytes());
            let wide = utf16(target);
            block[8 + 260..8 + 260 + wide.len()].copy_from_slice(&wide);
            out.extend_from_slice(&block);
        }
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    #[test]
    fn the_paths_are_read_from_where_the_format_puts_them() {
        let link = parse(&bytes_of(&Made {
            id_list: true,
            local: Some(r"C:\Program Files\Tool\tool.exe"),
            name: Some("Tool"),
            relative: Some(r"..\..\Program Files\Tool\tool.exe"),
            environment: Some(r"%ProgramFiles%\Tool\tool.exe"),
            ..Made::default()
        }))
        .expect("a shortcut");
        assert_eq!(link.local.as_deref(), Some(r"C:\Program Files\Tool\tool.exe"));
        assert_eq!(
            link.relative.as_deref(),
            Some(r"..\..\Program Files\Tool\tool.exe"),
            "the name before it is skipped"
        );
        assert_eq!(link.environment.as_deref(), Some(r"%ProgramFiles%\Tool\tool.exe"));
        assert!(!link.advertised && !link.network);

        let unicode = parse(&bytes_of(&Made {
            local: Some(r"C:\Programs\Tool\tool.exe"),
            unicode_info: true,
            ..Made::default()
        }))
        .expect("a shortcut");
        assert_eq!(
            unicode.local.as_deref(),
            Some(r"C:\Programs\Tool\tool.exe"),
            "the Unicode fields when the header has them"
        );

        // The narrow field holds what no code page can be trusted to say.
        let cyrillic = parse(&bytes_of(&Made {
            local: Some(r"C:\Документы\x.txt"),
            unicode_info: true,
            ..Made::default()
        }))
        .expect("a shortcut");
        assert_eq!(cyrillic.local.as_deref(), Some(r"C:\Документы\x.txt"), "read from the Unicode fields");
    }

    #[test]
    fn what_is_not_a_shortcut_or_ends_early_is_refused() {
        assert!(parse(b"not a shortcut at all, but long enough to have a header in it, well past seventy-six bytes").is_err());
        let whole = bytes_of(&Made {
            local: Some(r"C:\x.exe"),
            ..Made::default()
        });
        for cut in [10, HEADER + 6, whole.len() - 12] {
            assert!(parse(&whole[..cut]).is_err(), "cut at {cut}");
        }
        let mut overlong = whole.clone();
        overlong[HEADER..HEADER + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&overlong).is_err(), "a LinkInfo longer than the file");
    }

    #[test]
    fn a_path_in_an_unknown_code_page_is_not_guessed() {
        let mut bytes = bytes_of(&Made {
            local: Some(r"C:\Docs\x.txt"),
            ..Made::default()
        });
        let at = bytes.windows(4).position(|w| w == b"Docs").expect("the path");
        bytes[at] = 0xC4; // a letter in some code page, not ASCII
        assert_eq!(parse(&bytes).expect("a shortcut").local, None);
    }

    /// `judge` with the files in `present` there, every other path missing,
    /// and `C:` the only volume.
    fn judged(link: &Link, at: &str, present: &[&str]) -> Seen {
        let vars: HashMap<&str, &str> = HashMap::from([("ProgramFiles", r"C:\Program Files")]);
        let present: Vec<String> = present.iter().map(|path| path.to_ascii_lowercase()).collect();
        judge(link, Path::new(at), &|name| vars.get(name).map(|value| (*value).to_owned()), &|path| {
            let text = path.to_string_lossy().to_ascii_lowercase();
            if !text.starts_with("c:") && !text.starts_with('/') {
                Probe::Unknown
            } else if present.iter().any(|there| *there == text || there.starts_with(&format!("{text}\\"))) {
                Probe::Exists
            } else {
                Probe::Missing
            }
        })
    }

    #[test]
    fn a_shortcut_is_broken_only_when_nothing_is_at_any_path_it_gives() {
        let tool = Link {
            local: Some(r"C:\Program Files\Tool\tool.exe".to_owned()),
            environment: Some(r"%ProgramFiles%\Tool\tool.exe".to_owned()),
            ..Link::default()
        };
        let desktop = r"C:\Users\me\Desktop\Tool.lnk";
        assert_eq!(judged(&tool, desktop, &[r"C:\Program Files\Tool\tool.exe"]), Seen::Fine);
        assert_eq!(
            judged(&tool, desktop, &[]),
            Seen::Broken {
                target: r"C:\Program Files\Tool\tool.exe".to_owned()
            }
        );

        let moved = Link {
            relative: Some(r"..\Tool\tool.exe".to_owned()),
            ..tool.clone()
        };
        assert_eq!(
            judged(&moved, desktop, &[r"C:\Users\me\Desktop\..\Tool\tool.exe"]),
            Seen::Fine,
            "what the relative path finds works"
        );

        let elsewhere = Link {
            local: Some(r"D:\Games\game.exe".to_owned()),
            ..Link::default()
        };
        assert_eq!(
            judged(&elsewhere, desktop, &[]),
            Seen::Fine,
            "a drive that is not here is not a broken shortcut"
        );

        let shared = Link {
            network: true,
            ..Link::default()
        };
        assert_eq!(judged(&shared, desktop, &[]), Seen::Fine, "nor is a share");
        let advertised = Link {
            advertised: true,
            ..tool.clone()
        };
        assert_eq!(judged(&advertised, desktop, &[]), Seen::Fine, "nor what Windows Installer finds");
        assert_eq!(judged(&Link::default(), desktop, &[]), Seen::Fine, "nor a place only the shell names");

        let unknown_variable = Link {
            environment: Some(r"C:\Tools\%NOT_SET%\x.exe".to_owned()),
            ..Link::default()
        };
        assert_eq!(judged(&unknown_variable, desktop, &[]), Seen::Fine, "a path with a gap names nothing");
    }

    #[test]
    fn a_profile_s_variables_are_the_profile_the_shortcut_is_in() {
        if !cfg!(windows) {
            return;
        }
        let vars = variables(Path::new(r"C:\Users\someone\Desktop\x.lnk"));
        assert_eq!(vars("USERPROFILE").as_deref(), Some(r"C:\Users\someone"));
        assert_eq!(vars("localappdata").as_deref(), Some(r"C:\Users\someone\AppData\Local"));
        assert_eq!(profile_of(Path::new(r"C:\ProgramData\Microsoft\x.lnk")), None);
    }

    #[test]
    fn a_shortcut_on_disk_is_read_and_judged() {
        let folder = tempfile::tempdir().expect("a folder");
        let target = folder.path().join("there.txt");
        std::fs::write(&target, b"x").expect("a target");
        let target = target.to_string_lossy().into_owned();
        let gone = folder.path().join("gone.txt").to_string_lossy().into_owned();
        let write = |name: &str, local: &str| {
            let path = folder.path().join(name);
            std::fs::write(
                &path,
                bytes_of(&Made {
                    local: Some(local),
                    unicode_info: true,
                    ..Made::default()
                }),
            )
            .expect("a shortcut");
            path
        };
        assert_eq!(look(&write("works.lnk", &target)), Seen::Fine);
        assert_eq!(look(&write("broken.lnk", &gone)), Seen::Broken { target: gone });
        std::fs::write(folder.path().join("junk.lnk"), b"junk").expect("junk");
        assert_eq!(look(&folder.path().join("junk.lnk")), Seen::Fine, "unreadable as a shortcut is not broken");
    }
}
