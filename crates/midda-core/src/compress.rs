//! Space back from a cold folder without deleting anything: NTFS compression.
//!
//! A fossil ([`crate::layers`]) is often not rubbish. The photos of 2019, a
//! finished project, the installers of a game nobody plays: nothing has
//! touched them in years, and the reader would rather keep them. Compressed,
//! they stay where they are, open as they did - a little slower, which for a
//! folder nobody opens is nothing - and give back what compression takes out.
//!
//! # Which compression
//!
//! NTFS's own (LZNT1), as `compact /c` and Explorer's "Compress contents to
//! save disk space" apply it - not the Windows Overlay Filter's (`compact
//! /exe`), which packs tighter. NTFS compression is a fact the reader can see
//! and undo without midda: Explorer draws the names blue and the checkbox
//! takes it off; a folder marked compressed compresses what is put in it
//! later; and both scanners already read it as the `COMPRESSED` bit. The
//! overlay keeps a file behind a reparse point other tools read differently,
//! undoes itself the moment a file is written, and is invisible in Explorer -
//! a compression only midda could explain. What Windows compressed with the
//! overlay itself, midda explains ([`crate::traits::Traits::COMPRESSED`]) and
//! leaves alone.
//!
//! # What is left as it is
//!
//! Each file is asked first ([`left_as_is`]): what is compressed already, by
//! NTFS or by the overlay; a cloud placeholder, whose bytes are not here; a
//! sparse file and a virtual disk, which WSL and Hyper-V refuse to mount
//! compressed ("virtual hard disk files must be uncompressed and unencrypted
//! and must not be sparse"); and a file whose format is compressed already -
//! a photo, a video, an archive - which costs time and fragments the volume
//! for no space at all. What the volume refuses - a file in use, one this
//! account may not change, an encrypted one - is counted and said, not
//! retried. Windows' own files are not offered at all: a boot-start driver
//! NTFS-compressed is a machine that does not start, and Windows has its own
//! way to compress itself (CompactOS).
//!
//! # What does not move
//!
//! Compressing changes how a file is stored, not the file: measured
//! 2026-11-02, neither the write time nor the last access time of a
//! compressed file moved, and a read-only file compressed with its attribute
//! as it was. A fossil compressed is still a fossil, and its age is still
//! its own. See `docs/adr/0017-the-age-of-the-layers.md`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::consumers::{Classified, Consumer};
use crate::scanner::Progress;
use crate::size::Size;
use crate::traits::Traits;
use crate::tree::{NodeId, Tree};

/// Why a file was left as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Left {
    /// Compressed already: by NTFS, or by Windows' overlay.
    Compressed,
    /// A cloud placeholder: its bytes are not on this disk.
    Placeholder,
    /// Kept by Windows in a way of its own (a reparse point that is not a
    /// link), which compression is not for.
    Elsewhere,
    /// Sparse: a database or a disk keeps it so.
    Sparse,
    /// A virtual disk: WSL and Hyper-V refuse to mount a compressed one.
    VirtualDisk,
    /// Its format is compressed already.
    Packed,
    /// Encrypted, which NTFS does not compress.
    Encrypted,
    /// Another program holds it.
    InUse,
    /// This account may not change it.
    Denied,
    /// The volume refused for another reason.
    Failed,
}

impl Left {
    /// What the window says of the files left for this reason.
    #[must_use]
    pub const fn says(self) -> &'static str {
        match self {
            Self::Compressed => "compressed already",
            Self::Placeholder => "in the cloud, not on this disk",
            Self::Elsewhere => "kept by Windows in a way of its own",
            Self::Sparse => "sparse, as a database or a disk keeps its files",
            Self::VirtualDisk => "virtual disks, which WSL and Hyper-V refuse compressed",
            Self::Packed => "compressed by their format already: photos, video, archives",
            Self::Encrypted => "encrypted, which NTFS does not compress",
            Self::InUse => "held open by another program",
            Self::Denied => "not this account's to change",
            Self::Failed => "refused by the volume",
        }
    }
}

/// The extensions of virtual disks: Hyper-V's and WSL's, which refuse to
/// mount compressed, and their checkpoints.
const VIRTUAL_DISKS: &[&str] = &["vhd", "vhdx", "avhd", "avhdx"];

/// The extensions of formats that are compressed already: compression finds
/// nothing in them to take out.
///
/// The common ones, not every one: a format missing here is compressed for
/// nothing, which costs time and gives back zero, and the result says so. A
/// format here wrongly would be space left on the table - so only formats
/// whose contents are compressed by definition are here: not PDF, not TIFF,
/// not a raw photo, which often are not.
const PACKED: &[&str] = &[
    // Archives and packages, git's packs among them: every object in a pack
    // is deflated.
    "7z", "apk", "appx", "br", "bz2", "cab", "crx", "gz", "jar", "lz", "lz4", "lzma", "msix", "nupkg", "pack", "rar", "tbz2", "tgz", "txz", "whl", "xz", "zip",
    "zst", // Office files and books, which are zip archives.
    "docx", "epub", "odp", "ods", "odt", "pptx", "xlsx", // Images.
    "avif", "gif", "heic", "heif", "jpeg", "jpg", "jxl", "png", "webp", // Audio.
    "aac", "flac", "m4a", "mp3", "ogg", "opus", "wma", // Video.
    "avi", "m4v", "mkv", "mov", "mp4", "webm", "wmv",
];

/// The lowercase extension of `name`, when it has one.
fn extension(name: &str) -> Option<String> {
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty()).then(|| extension.to_ascii_lowercase())
}

/// Why a file called `name` with `traits` is left as it is, when it is - the
/// one test both the estimate from the tree and the run on the volume ask.
#[must_use]
pub fn left_as_is(name: &str, traits: Traits) -> Option<Left> {
    if traits.has(Traits::COMPRESSED) {
        return Some(Left::Compressed);
    }
    if traits.has(Traits::PLACEHOLDER) {
        return Some(Left::Placeholder);
    }
    if traits.has(Traits::SPARSE) {
        return Some(Left::Sparse);
    }
    let extension = extension(name)?;
    if VIRTUAL_DISKS.contains(&extension.as_str()) {
        Some(Left::VirtualDisk)
    } else if PACKED.contains(&extension.as_str()) {
        Some(Left::Packed)
    } else {
        None
    }
}

/// Whether an entry can be compressed, and if so, how much compression would
/// work on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// It can: compression would read this much. How much comes back is the
    /// files' to say, and is measured afterwards.
    Can { reads: Size },
    /// It cannot, and why.
    Not(Why),
}

/// Why an entry is not offered for compression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "why", content = "detail")]
pub enum Why {
    /// The volume does not compress files: what [`refused_on`] said.
    Volume(String),
    /// Windows' own files are in it.
    Windows,
    /// It is compressed already: what in it would compress, is.
    Compressed,
    /// Nothing in it would compress: formats compressed by themselves, a
    /// virtual disk, files in the cloud - and none of it compressed. Not the
    /// same as `Compressed`, which a reader is glad to be told; this one they
    /// would be misled by.
    Nothing,
}

/// Whether the entry `id` of `tree` can be compressed: never when the volume
/// cannot (`volume`, what [`refused_on`] said of it) or Windows' own files are
/// in it, as `classified` says ([`crate::consumers`]); otherwise when
/// anything in it is left to compress.
#[must_use]
pub fn verdict(tree: &Tree, id: NodeId, classified: &Classified, volume: Option<&str>) -> Verdict {
    if let Some(refusal) = volume {
        return Verdict::Not(Why::Volume(refusal.to_owned()));
    }
    let mut reads = Size::zero();
    let mut compressed = false;
    let mut stack = vec![id];
    while let Some(at) = stack.pop() {
        if classified.of(at) == Some(Consumer::System) {
            return Verdict::Not(Why::Windows);
        }
        let node = tree.node(at);
        if node.is_directory() {
            stack.extend_from_slice(&node.children);
            continue;
        }
        match left_as_is(&node.name, node.traits) {
            None => reads.add(node.size),
            Some(Left::Compressed) => compressed = true,
            Some(_) => {}
        }
    }
    if reads.allocated > 0 {
        Verdict::Can { reads }
    } else if compressed {
        Verdict::Not(Why::Compressed)
    } else {
        Verdict::Not(Why::Nothing)
    }
}

/// Why the volume `path` is on cannot compress files, when it cannot.
///
/// NTFS compresses; ReFS, FAT and exFAT do not. NTFS itself does not on a
/// volume whose clusters are larger than 4 KB, and not at all on a machine
/// where `NtfsDisableCompression` is set.
#[must_use]
pub fn refused_on(path: &Path) -> Option<String> {
    imp::refused_on(path)
}

/// What compressing did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compressed {
    /// Files compressed now.
    pub files: u64,
    /// What they read as, in bytes.
    pub bytes: u64,
    /// Folders marked, so what is put in them later is compressed too.
    pub folders: u64,
    /// The files left as they were, by why - the most first.
    pub left: Vec<Kept>,
    /// The reader stopped it before the end.
    pub stopped: bool,
}

/// The files left as they were for one reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub why: Left,
    pub files: u64,
    /// What they read as, in bytes.
    pub bytes: u64,
    /// The first refusal the volume gave, when it gave one: what a reader
    /// asks about when the volume refused "for another reason".
    pub first: Option<String>,
}

impl Compressed {
    fn leave(&mut self, why: Left, bytes: u64, refusal: Option<String>) {
        if let Some(kept) = self.left.iter_mut().find(|kept| kept.why == why) {
            kept.files += 1;
            kept.bytes += bytes;
            if kept.first.is_none() {
                kept.first = refusal;
            }
        } else {
            self.left.push(Kept {
                why,
                files: 1,
                bytes,
                first: refusal,
            });
        }
    }
}

/// Compresses the file or folder at `path` with NTFS compression: every file
/// in it not left as it is ([`left_as_is`]), and every folder, so what is
/// put there later is compressed too. Links are never followed.
///
/// Stops between two files when `progress` is cancelled, and says it
/// stopped: what was compressed until then stays compressed, which is a
/// folder in two states - each of them a file that opens. `progress` counts
/// the files gone through and what they read as.
///
/// # Errors
///
/// When the volume cannot compress ([`refused_on`]), or `path` cannot be
/// read at all.
pub fn compress(path: &Path, progress: &Progress) -> Result<Compressed, String> {
    if let Some(refusal) = refused_on(path) {
        return Err(refusal);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    let mut done = Compressed::default();
    if metadata.file_type().is_symlink() {
        return Ok(done);
    }
    if metadata.is_dir() {
        let mut folders = vec![path.to_path_buf()];
        while let Some(folder) = folders.pop() {
            if progress.cancelled() {
                done.stopped = true;
                break;
            }
            // The folder first: whatever happens to the files in it, what is
            // put there later is compressed.
            if imp::set_compression(&folder, true).is_ok() {
                done.folders += 1;
            }
            let Ok(listing) = std::fs::read_dir(&folder) else {
                continue;
            };
            for entry in listing.flatten() {
                if progress.cancelled() {
                    done.stopped = true;
                    break;
                }
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    folders.push(entry.path());
                } else {
                    file(&entry.path(), &entry.file_name().to_string_lossy(), &metadata, &mut done);
                    progress.advance(1, metadata.len());
                }
            }
        }
    } else {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        file(path, &name, &metadata, &mut done);
        progress.advance(1, metadata.len());
    }
    done.left.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.why.cmp(&b.why)));
    Ok(done)
}

/// Compresses one file, or says why it was left.
fn file(path: &Path, name: &str, metadata: &std::fs::Metadata, done: &mut Compressed) {
    let bytes = metadata.len();
    if let Some(why) = imp::left_on_volume(path, metadata).or_else(|| left_as_is(name, imp::traits_of(path, metadata))) {
        done.leave(why, bytes, None);
        return;
    }
    match imp::set_compression(path, false) {
        Ok(()) => {
            done.files += 1;
            done.bytes += bytes;
        }
        Err((why, refusal)) => done.leave(why, bytes, Some(refusal)),
    }
}

#[cfg(windows)]
#[allow(unsafe_code, reason = "NTFS compression is an ioctl on a handle, which std does not expose")]
mod imp {
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::fs::MetadataExt as _;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        COMPRESSION_FORMAT_DEFAULT, CreateFileW, FILE_ATTRIBUTE_READONLY, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_DATA,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_DATA, GetFileAttributesW, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING, SetFileAttributesW,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_COMPRESSION;

    use super::Left;
    use crate::traits::Traits;

    /// `FILE_FILE_COMPRESSION`: the volume compresses files.
    const FILE_COMPRESSION: u32 = 0x0000_0010;
    /// `FILE_ATTRIBUTE_ENCRYPTED`.
    const ENCRYPTED: u32 = 0x0000_4000;
    /// `FILE_ATTRIBUTE_REPARSE_POINT`.
    const REPARSE_POINT: u32 = 0x0000_0400;
    /// The largest cluster NTFS compresses on.
    const LARGEST_CLUSTER: u64 = 4096;

    pub(super) fn refused_on(path: &Path) -> Option<String> {
        let Some((filesystem, flags)) = crate::mft::volume::describe(path) else {
            return Some("the volume this is on could not be asked what it can do".to_owned());
        };
        if filesystem != "NTFS" {
            return Some(format!("{filesystem} does not compress files; NTFS does"));
        }
        if crate::platform::filesystem_setting("NtfsDisableCompression").is_some_and(|value| value != 0) {
            return Some("NTFS compression is turned off on this machine (NtfsDisableCompression)".to_owned());
        }
        if let Some(cluster) = crate::platform::cluster_bytes_of(path)
            && cluster > LARGEST_CLUSTER
        {
            return Some(format!(
                "NTFS compresses only on volumes of 4 KB clusters or smaller, and this one's are {}",
                crate::words::bytes(cluster)
            ));
        }
        if flags & FILE_COMPRESSION == 0 {
            return Some("this volume does not compress files".to_owned());
        }
        None
    }

    /// What the volume says of a file that the tree's traits do not:
    /// encrypted, or kept behind a reparse point that is not a link.
    pub(super) fn left_on_volume(_path: &Path, metadata: &std::fs::Metadata) -> Option<Left> {
        let attributes = metadata.file_attributes();
        if attributes & ENCRYPTED != 0 {
            Some(Left::Encrypted)
        } else if attributes & REPARSE_POINT != 0 && !crate::platform::traits_from_attributes(attributes).has(Traits::PLACEHOLDER) {
            Some(Left::Elsewhere)
        } else {
            None
        }
    }

    /// The traits the scanners would give the file, the overlay's included.
    pub(super) fn traits_of(path: &Path, metadata: &std::fs::Metadata) -> Traits {
        let traits = crate::platform::traits_from_attributes(metadata.file_attributes());
        if !traits.any() && crate::platform::overlaid(path) {
            traits.with(Traits::COMPRESSED)
        } else {
            traits
        }
    }

    /// Marks `path` compressed. A file is compressed then and there; a folder
    /// is marked, so what is put in it later is.
    pub(super) fn set_compression(path: &Path, folder: bool) -> Result<(), (Left, String)> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let flags = if folder { FILE_FLAG_BACKUP_SEMANTICS } else { 0 } | FILE_FLAG_OPEN_REPARSE_POINT;
        // NTFS compresses through a handle that may read and write the data.
        let outcome = compress_through(&wide, flags);
        if folder || !matches!(outcome, Err((Left::Denied, _))) {
            return outcome;
        }
        // A read-only file refuses write access to its data, and `compact`
        // compresses it anyway: the attribute is taken off for the moment the
        // compression takes, and put back. Neither touches a time the file
        // keeps (measured 2026-11-02).
        // SAFETY: `wide` ends in a zero and outlives the call.
        let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
        if attributes == INVALID_FILE_ATTRIBUTES || attributes & FILE_ATTRIBUTE_READONLY == 0 {
            return outcome;
        }
        // SAFETY: as above.
        if unsafe { SetFileAttributesW(wide.as_ptr(), attributes & !FILE_ATTRIBUTE_READONLY) } == 0 {
            return outcome;
        }
        let again = compress_through(&wide, flags);
        // SAFETY: as above; the attributes are the file's own, as they were.
        unsafe { SetFileAttributesW(wide.as_ptr(), attributes) };
        again
    }

    /// Opens the entry at `wide` to read and write its data, and asks NTFS to
    /// compress it.
    fn compress_through(wide: &[u16], flags: u32) -> Result<(), (Left, String)> {
        // SAFETY: `wide` ends in a zero and outlives the call; the null
        // pointers are "no security attributes, no template".
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_READ_DATA | FILE_WRITE_DATA,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                flags,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(refusal());
        }
        let format = COMPRESSION_FORMAT_DEFAULT;
        let mut returned = 0_u32;
        // SAFETY: the handle is open; `format` is the u16 the ioctl takes,
        // read for its own size; no output is asked for.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_COMPRESSION,
                std::ptr::from_ref(&format).cast(),
                u32::try_from(size_of_val(&format)).unwrap_or(2),
                std::ptr::null_mut(),
                0,
                &raw mut returned,
                std::ptr::null_mut(),
            )
        };
        let outcome = if ok == 0 { Err(refusal()) } else { Ok(()) };
        // SAFETY: opened above, closed once.
        unsafe { CloseHandle(handle) };
        outcome
    }

    /// What the last failed call on this thread was refused for.
    fn refusal() -> (Left, String) {
        // SAFETY: reads the calling thread's last error, nothing else.
        let code = unsafe { GetLastError() };
        let why = match code {
            ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION => Left::InUse,
            ERROR_ACCESS_DENIED => Left::Denied,
            _ => Left::Failed,
        };
        #[allow(clippy::cast_possible_wrap, reason = "a Win32 error code is what from_raw_os_error takes")]
        (why, std::io::Error::from_raw_os_error(code as i32).to_string())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    use super::Left;
    use crate::traits::Traits;

    pub(super) fn refused_on(_path: &Path) -> Option<String> {
        Some("NTFS compression is Windows'".to_owned())
    }

    pub(super) const fn left_on_volume(_path: &Path, _metadata: &std::fs::Metadata) -> Option<Left> {
        None
    }

    pub(super) const fn traits_of(_path: &Path, _metadata: &std::fs::Metadata) -> Traits {
        Traits::none()
    }

    pub(super) fn set_compression(_path: &Path, _folder: bool) -> Result<(), (Left, String)> {
        Err((Left::Failed, "NTFS compression is Windows'".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::rules::RuleSet;
    use crate::tree::{Kind, Node, ROOT};

    #[test]
    fn what_is_compressed_elsewhere_or_by_its_format_is_left_as_it_is() {
        assert_eq!(left_as_is("notes.txt", Traits::none()), None);
        assert_eq!(left_as_is("app.exe", Traits::none()), None);
        assert_eq!(left_as_is("notes.txt", Traits::COMPRESSED), Some(Left::Compressed));
        assert_eq!(left_as_is("notes.txt", Traits::PLACEHOLDER), Some(Left::Placeholder));
        assert_eq!(left_as_is("data.db", Traits::SPARSE), Some(Left::Sparse));
        assert_eq!(left_as_is("ext4.VHDX", Traits::none()), Some(Left::VirtualDisk), "case does not hide a disk");
        assert_eq!(left_as_is("IMG_0001.JPG", Traits::none()), Some(Left::Packed));
        assert_eq!(left_as_is("backup.tar.gz", Traits::none()), Some(Left::Packed));
        assert_eq!(left_as_is(".zip", Traits::none()), None, "a name that is all extension is a name");
        assert_eq!(left_as_is("scan.pdf", Traits::none()), None, "a PDF often compresses, and is tried");
    }

    /// A volume's top, where `/Windows` is Windows'.
    const VOLUME: &str = if cfg!(windows) { "C:/" } else { "/" };

    fn tree() -> Tree {
        let mut tree = Tree::new(PathBuf::from(VOLUME), Some(4096));
        let old = tree.push(ROOT, Node::new("old".into(), ROOT, Kind::Directory));
        for (name, bytes, traits) in [
            ("a.txt", 1000, Traits::none()),
            ("b.jpg", 5000, Traits::none()),
            ("c.txt", 300, Traits::COMPRESSED),
        ] {
            tree.push(
                old,
                Node {
                    size: Size::flat(bytes),
                    traits,
                    ..Node::new(name.into(), old, Kind::File)
                },
            );
        }
        let windows = tree.push(ROOT, Node::new("Windows".into(), ROOT, Kind::Directory));
        let inside = tree.push(windows, Node::new("old-drivers".into(), windows, Kind::Directory));
        tree.push(
            inside,
            Node {
                size: Size::flat(100),
                ..Node::new("x.sys".into(), inside, Kind::File)
            },
        );
        tree.roll_up();
        tree
    }

    fn classified(tree: &Tree) -> Classified {
        let rules = RuleSet::shipped();
        crate::consumers::classify(rules, tree, &rules.survey(tree))
    }

    #[test]
    fn a_folder_offers_what_in_it_would_compress() {
        let tree = tree();
        let classified = classified(&tree);
        let old = tree.child(ROOT, "old").expect("old");
        assert_eq!(verdict(&tree, old, &classified, None), Verdict::Can { reads: Size::flat(1000) });
    }

    #[test]
    fn windows_own_files_and_a_volume_that_cannot_are_not_offered() {
        let tree = tree();
        let classified = classified(&tree);
        let drivers = tree.child(tree.child(ROOT, "Windows").expect("Windows"), "old-drivers").expect("drivers");
        assert_eq!(verdict(&tree, drivers, &classified, None), Verdict::Not(Why::Windows));
        let old = tree.child(ROOT, "old").expect("old");
        assert_eq!(
            verdict(&tree, old, &classified, Some("exFAT does not compress files; NTFS does")),
            Verdict::Not(Why::Volume("exFAT does not compress files; NTFS does".into()))
        );
    }

    #[test]
    fn a_folder_with_nothing_left_to_compress_says_whether_it_is_compressed_or_never_would_be() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let photos = tree.push(ROOT, Node::new("photos".into(), ROOT, Kind::Directory));
        tree.push(
            photos,
            Node {
                size: Size::flat(9000),
                ..Node::new("IMG_1.jpg".into(), photos, Kind::File)
            },
        );
        let vm = tree.push(ROOT, Node::new("vm".into(), ROOT, Kind::Directory));
        tree.push(
            vm,
            Node {
                size: Size::flat(9000),
                ..Node::new("old.vhdx".into(), vm, Kind::File)
            },
        );
        let done = tree.push(ROOT, Node::new("done".into(), ROOT, Kind::Directory));
        tree.push(
            done,
            Node {
                size: Size::flat(500),
                traits: Traits::COMPRESSED,
                ..Node::new("notes.txt".into(), done, Kind::File)
            },
        );
        tree.push(
            done,
            Node {
                size: Size::flat(9000),
                ..Node::new("IMG_2.jpg".into(), done, Kind::File)
            },
        );
        tree.roll_up();
        let classified = classified(&tree);
        assert_eq!(verdict(&tree, photos, &classified, None), Verdict::Not(Why::Nothing));
        // A virtual disk is not compressed, and is not to be called so: seen
        // on screen as a "compressed" badge on a folder holding one VHDX.
        assert_eq!(verdict(&tree, vm, &classified, None), Verdict::Not(Why::Nothing));
        assert_eq!(verdict(&tree, done, &classified, None), Verdict::Not(Why::Compressed));
    }

    #[cfg(windows)]
    fn write(path: &Path, bytes: usize) {
        std::fs::write(path, "compressible text, said again and again. ".repeat(bytes / 42 + 1)).expect("write");
    }

    #[test]
    #[cfg(windows)]
    fn a_folder_is_compressed_in_place_and_keeps_its_age() {
        use std::fs::FileTimes;
        use std::time::{Duration, SystemTime};

        let dir = tempfile::tempdir().expect("tempdir");
        if let Some(refusal) = refused_on(dir.path()) {
            eprintln!("skipped: {refusal}");
            return;
        }
        let folder = dir.path().join("cold");
        std::fs::create_dir_all(folder.join("inner")).expect("folders");
        write(&folder.join("a.txt"), 200_000);
        write(&folder.join("inner/b.log"), 100_000);
        write(&folder.join("photo.jpg"), 50_000);
        let read_only = folder.join("kept.txt");
        write(&read_only, 80_000);

        let written = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        let opened = SystemTime::UNIX_EPOCH + Duration::from_secs(1_650_000_000);
        for path in [folder.join("a.txt"), read_only.clone()] {
            let file = std::fs::File::options().write(true).open(&path).expect("open");
            file.set_times(FileTimes::new().set_modified(written).set_accessed(opened)).expect("times");
        }
        let mut permissions = std::fs::metadata(&read_only).expect("metadata").permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&read_only, permissions).expect("read-only");

        let progress = Progress::new();
        let done = compress(&folder, &progress).expect("compressed");
        assert_eq!(done.files, 3, "a.txt, b.log and the read-only kept.txt: {done:?}");
        assert_eq!(done.folders, 2);
        assert_eq!(done.left.len(), 1);
        assert_eq!((done.left[0].why, done.left[0].files), (Left::Packed, 1));
        assert!(!done.stopped);
        assert_eq!(progress.entries(), 4, "every file gone through is counted");

        for path in [folder.join("a.txt"), read_only.clone()] {
            let metadata = std::fs::metadata(&path).expect("metadata");
            let traits = crate::platform::traits_of(&metadata);
            assert!(traits.has(Traits::COMPRESSED), "{} is compressed", path.display());
            assert_eq!(metadata.modified().ok(), Some(written), "the write time of {} did not move", path.display());
            if crate::platform::access_times_kept(&path) {
                assert_eq!(metadata.accessed().ok(), Some(opened), "nor the time it was opened");
            }
            let measured = crate::platform::measure(&path, &metadata, 4096);
            assert!(measured.size.allocated < measured.size.logical, "{} takes less than it reads", path.display());
        }
        assert!(
            std::fs::metadata(&read_only).expect("metadata").permissions().readonly(),
            "read-only stays read-only"
        );
        // What is put in the folder later is compressed by the folder.
        let later = folder.join("inner/later.txt");
        write(&later, 50_000);
        assert!(crate::platform::traits_of(&std::fs::metadata(&later).expect("metadata")).has(Traits::COMPRESSED));

        let mut permissions = std::fs::metadata(&read_only).expect("metadata").permissions();
        #[allow(clippy::permissions_set_readonly_false, reason = "the temporary folder has to be removable")]
        permissions.set_readonly(false);
        std::fs::set_permissions(&read_only, permissions).expect("writable again");
    }

    #[test]
    #[cfg(windows)]
    fn a_stopped_compression_says_it_stopped() {
        let dir = tempfile::tempdir().expect("tempdir");
        if refused_on(dir.path()).is_some() {
            return;
        }
        write(&dir.path().join("a.txt"), 10_000);
        let progress = Progress::new();
        progress.cancel();
        let done = compress(dir.path(), &progress).expect("ran");
        assert!(done.stopped);
        assert_eq!(done.files, 0);
    }
}
