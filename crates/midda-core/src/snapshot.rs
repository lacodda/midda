//! Pictures of a folder kept for later: what a comparison is made between.
//!
//! A snapshot is a saved index nobody brings up to date — the same file, in
//! the same format ([`crate::fresh::store`]), kept under a name of its own.
//! One reader and one writer for both, and one format to freeze before 1.0.
//!
//! # Two kinds
//!
//! - **Last time.** Opening a folder reads the index saved when it was last
//!   closed. Before anything brings that index up to date, the file is kept
//!   as the picture of last time: what "what grew since last time" is
//!   measured from. One per folder, replaced each time the folder is opened.
//!   It is a hard link to the saved index, so it costs nothing on disk until
//!   the index is saved again — and then exactly one index.
//! - **Taken.** A picture the reader asked for — before a cleanup, say — kept
//!   until they delete it. midda does not thin these out on its own: they are
//!   the reader's, and the window says what they take.
//!
//! A snapshot takes as much as the index it was written from: some 45 bytes
//! an entry, 130 MB for a system volume of three million. A disk analyzer
//! that quietly kept a week of those would be its own best finding, so midda
//! keeps one picture of last time and nothing else unasked (ADR 0010).
//!
//! # Where they live
//!
//! In a folder per scanned folder, named like the saved index by a hash of
//! the path: `<snapshots>/<hash>/last.midx` and `taken-<millis>.midx`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::fresh::store::{self, Encoded, Saved, Unusable};

/// The picture of last time, in a folder's snapshot folder.
const LAST: &str = "last.midx";

/// The start of a taken snapshot's file name; the moment follows, in
/// milliseconds since the Unix epoch.
const TAKEN: &str = "taken-";

const EXTENSION: &str = "midx";

/// Which kind of picture a snapshot is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// The folder as it was when it was last closed.
    Last,
    /// Taken by the reader, and kept until they delete it.
    Taken,
}

/// One snapshot of a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Its file name, which is what the window names it by.
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    /// The moment it is a picture of.
    pub at: SystemTime,
    /// What its file takes.
    pub bytes: u64,
    /// Why it cannot be compared, when it cannot: written by another version
    /// of midda, or damaged. Listed anyway, so it can be deleted.
    pub unusable: Option<String>,
}

/// The folder where the snapshots of `root` are kept, under `snapshots`.
#[must_use]
pub fn folder_for(snapshots: &Path, root: &Path) -> PathBuf {
    snapshots.join(store::name_for(root))
}

/// Keeps the saved index at `index` as the picture of last time in
/// `folder`, replacing the one there.
///
/// A hard link where the volume allows one, a copy where it does not. The
/// saved index is replaced by a rename when it is next written, so the link
/// goes on holding the bytes it was made to.
///
/// # Errors
///
/// When the folder cannot be made, or the index can be neither linked nor
/// copied.
pub fn keep_last(folder: &Path, index: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let last = folder.join(LAST);
    let partial = last.with_extension("midx.partial");
    match std::fs::remove_file(&partial) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    if std::fs::hard_link(index, &partial).is_err() {
        std::fs::copy(index, &partial)?;
    }
    std::fs::rename(&partial, &last)?;
    Ok(last)
}

/// Puts `encoded` — an index written into memory at `at` — on disk as a
/// snapshot the reader took.
///
/// # Errors
///
/// When the folder cannot be made or the file cannot be written.
pub fn keep(encoded: &Encoded, folder: &Path, at: SystemTime) -> std::io::Result<Snapshot> {
    std::fs::create_dir_all(folder)?;
    let millis = at.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    // Two taken within a millisecond: the second gets a suffix rather than
    // the first's place.
    let mut name = format!("{TAKEN}{millis}.{EXTENSION}");
    let mut again = 1;
    while folder.join(&name).exists() {
        name = format!("{TAKEN}{millis}-{again}.{EXTENSION}");
        again += 1;
    }
    let path = folder.join(&name);
    encoded.write_to(&path)?;
    Ok(Snapshot {
        name,
        path,
        kind: Kind::Taken,
        at,
        bytes: encoded.len() as u64,
        unusable: None,
    })
}

/// Every snapshot of `root` kept in `folder`, the most recent first.
///
/// What each is a picture of, and when, is read from its own header; a file
/// that cannot say — another version's, a damaged one — is listed with the
/// reason, at the moment its name or its file time gives.
#[must_use]
pub fn list(folder: &Path, root: &Path) -> Vec<Snapshot> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut snapshots: Vec<Snapshot> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let kind = kind_of(&name)?;
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            let (at, unusable) = match store::header(&path) {
                Ok(header) if store::same_folder(&header.root, root) => (header.saved_at, None),
                Ok(_) => (fallback_moment(&name, &metadata), Some("it is a picture of another folder".to_owned())),
                Err(why) => (fallback_moment(&name, &metadata), Some(why.to_string())),
            };
            Some(Snapshot {
                name,
                path,
                kind,
                at,
                bytes: metadata.len(),
                unusable,
            })
        })
        .collect();
    snapshots.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.name.cmp(&b.name)));
    snapshots
}

/// Reads the snapshot called `name` in `folder`.
///
/// # Errors
///
/// When there is no such snapshot, or it cannot be read.
pub fn load(folder: &Path, name: &str) -> Result<Saved, Unusable> {
    let path = path_of(folder, name).ok_or(Unusable::Missing)?;
    store::load_file(&path)
}

/// Deletes the snapshot called `name` in `folder`.
///
/// # Errors
///
/// When `name` is not a snapshot's name, or the file cannot be removed.
pub fn remove(folder: &Path, name: &str) -> std::io::Result<()> {
    let path = path_of(folder, name).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not the name of a snapshot"))?;
    std::fs::remove_file(path)
}

/// The path of the snapshot called `name`, when `name` is one: a bare file
/// name of either kind, never a path that climbs out of the folder.
fn path_of(folder: &Path, name: &str) -> Option<PathBuf> {
    let bare = Path::new(name).file_name().is_some_and(|file| file == name);
    (bare && kind_of(name).is_some()).then(|| folder.join(name))
}

fn kind_of(name: &str) -> Option<Kind> {
    if name == LAST {
        return Some(Kind::Last);
    }
    let stem = name.strip_prefix(TAKEN)?.strip_suffix(EXTENSION)?.strip_suffix('.')?;
    let digits = stem.split('-').next()?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(Kind::Taken)
}

/// The moment a snapshot whose header could not be read is listed at: the
/// one in its name, or when its file was last written.
fn fallback_moment(name: &str, metadata: &std::fs::Metadata) -> SystemTime {
    name.strip_prefix(TAKEN)
        .and_then(|rest| rest.split(['-', '.']).next())
        .and_then(|digits| digits.parse::<u64>().ok())
        .map(|millis| SystemTime::UNIX_EPOCH + Duration::from_millis(millis))
        .or_else(|| metadata.modified().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_is_named_by_its_kind_and_nothing_else_is_one() {
        assert_eq!(kind_of("last.midx"), Some(Kind::Last));
        assert_eq!(kind_of("taken-1790000000000.midx"), Some(Kind::Taken));
        assert_eq!(kind_of("taken-1790000000000-2.midx"), Some(Kind::Taken));
        assert_eq!(kind_of("last.midx.partial"), None);
        assert_eq!(kind_of("taken-.midx"), None);
        assert_eq!(kind_of("taken-12ab.midx"), None);
        assert_eq!(kind_of("0123456789abcdef.midx"), None, "a saved index is not a snapshot");
    }

    #[test]
    fn a_name_from_the_window_cannot_reach_outside_the_folder() {
        let folder = Path::new("snapshots/abc");
        assert!(path_of(folder, "taken-1.midx").is_some());
        assert!(path_of(folder, "../taken-1.midx").is_none());
        assert!(path_of(folder, "..\\last.midx").is_none());
        assert!(path_of(folder, "C:\\Windows\\last.midx").is_none());
        assert!(path_of(folder, "notes.txt").is_none());
    }

    #[test]
    fn an_unreadable_snapshot_is_dated_by_its_name() {
        let metadata = std::fs::metadata(".").expect("the working directory");
        let at = fallback_moment("taken-1790000000123-1.midx", &metadata);
        assert_eq!(at, SystemTime::UNIX_EPOCH + Duration::from_millis(1_790_000_000_123));
    }
}
