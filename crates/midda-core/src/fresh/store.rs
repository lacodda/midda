//! The index between runs: written when midda closes, read when the same
//! folder is opened again.
//!
//! The point of it is the first second. A walk of a system volume takes
//! minutes; the saved index is on screen as soon as it is read, labelled with
//! its age, while the walk or the change journal brings it up to date
//! (ADR 0009).
//!
//! # The format
//!
//! midda's own, written and read here, versioned by [`FORMAT`]: a file from
//! another version is not read but replaced — there is nothing in it a scan
//! cannot produce again. Integers are LEB128 varints, strings are a length and
//! UTF-8. The nodes are written in the order a fresh scan would have pushed
//! them, and only what a scan measures: a file's sizes, every entry's own
//! times and identity. Folder totals, entry counts and subtree times are
//! worked out again on reading — they are arithmetic, and a file that stored
//! them could store them wrong. A trailer repeats the node count, so a file
//! cut short by a crash is refused rather than read as a smaller disk.
//!
//! On a system volume of three million entries the file is around 130 MB.
//!
//! The same file, kept under another name, is a snapshot: a picture of the
//! folder at a moment, compared later with another ([`crate::snapshot`]).
//! One format for both — a snapshot is a saved index nobody brings up to date.

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{Index, JournalPosition};
use crate::links::Deduplicated;
use crate::platform::FileIdentity;
use crate::size::Size;
use crate::traits::Traits;
use crate::tree::{Kind, Node, NodeId, ROOT, Tree};

/// The version of the format. Raised with any change to what is written;
/// a file of another version is replaced, never migrated.
pub const FORMAT: u32 = 1;

const MAGIC: &[u8; 4] = b"MIDX";
const TRAILER: &[u8; 4] = b"XDIM";

/// Node flags: what follows the name and the parent.
const DIRECTORY: u8 = 1 << 0;
const HAS_MODIFIED: u8 = 1 << 1;
const HAS_LINKS: u8 = 1 << 2;
const HAS_IDENTITY: u8 = 1 << 3;
/// The identity is on a volume other than the root's — a mount point crossed.
const OTHER_VOLUME: u8 = 1 << 4;

/// Where the index of `root` is kept, under `directory`.
///
/// Named by a hash of the folder's path rather than the path itself: a path
/// has characters a file name cannot, and the path is inside the file anyway.
#[must_use]
pub fn file_for(directory: &Path, root: &Path) -> PathBuf {
    directory.join(format!("{}.midx", name_for(root)))
}

/// What everything kept for `root` is named by: a hash of its path as the
/// volume compares it.
#[must_use]
pub fn name_for(root: &Path) -> String {
    format!("{:016x}", fnv(&key_of(root)))
}

/// The path as the volume compares it: on Windows, without regard to case or
/// to which separator was typed.
fn key_of(root: &Path) -> String {
    let text = root.to_string_lossy();
    if cfg!(windows) {
        text.replace('/', "\\").trim_end_matches('\\').chars().flat_map(char::to_uppercase).collect()
    } else {
        text.trim_end_matches('/').to_owned()
    }
}

/// FNV-1a, 64 bits: a stable hash of a few dozen bytes, which is all a file
/// name needs. `std`'s hasher is not stable across releases.
fn fnv(text: &str) -> u64 {
    text.bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

/// Writes `index` where [`file_for`] says, replacing what was there only once
/// the new file is whole.
///
/// The live tree is not touched: the nodes are written in scan order by
/// walking the tree level by level, so a window holding ids keeps them.
///
/// # Errors
///
/// When the directory cannot be created or the file cannot be written.
pub fn save(index: &Index, directory: &Path, saved_at: SystemTime) -> std::io::Result<PathBuf> {
    encode(index, saved_at).write(directory)
}

/// An index written into memory, ready to be put on disk.
///
/// The two halves of [`save`], apart: writing into memory needs the tree and
/// takes a fraction of a second; putting the bytes on disk needs only the
/// bytes, and can take seconds — a file of a hundred megabytes, flushed, and
/// read through by an antivirus on its way. Holding the tree for the second
/// half would hold up every change waiting to be applied.
#[derive(Debug)]
pub struct Encoded {
    root: PathBuf,
    bytes: Vec<u8>,
}

/// Writes `index` into memory. See [`Encoded`].
#[must_use]
pub fn encode(index: &Index, saved_at: SystemTime) -> Encoded {
    let mut bytes = Vec::new();
    // Writing into a `Vec` cannot fail.
    let _ = write(&mut bytes, index, saved_at);
    Encoded {
        root: index.tree().root_path().to_path_buf(),
        bytes,
    }
}

impl Encoded {
    /// Puts the index on disk where [`file_for`] says, replacing what was
    /// there only once the new file is whole.
    ///
    /// # Errors
    ///
    /// When the directory cannot be created or the file cannot be written.
    pub fn write(&self, directory: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(directory)?;
        let path = file_for(directory, &self.root);
        self.write_to(&path)?;
        Ok(path)
    }

    /// Puts the index on disk at `path`, replacing what was there only once
    /// the new file is whole. The folder above `path` has to exist.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn write_to(&self, path: &Path) -> std::io::Result<()> {
        let partial = path.with_extension("midx.partial");
        {
            let mut out = File::create(&partial)?;
            out.write_all(&self.bytes)?;
            out.sync_all()?;
        }
        std::fs::rename(&partial, path)
    }

    /// How many bytes the file will take.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether there is nothing to write — which an encoded index never is.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Why a saved index was not used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unusable {
    /// Nothing is saved for this folder.
    #[error("nothing is saved for this folder")]
    Missing,
    /// Written by another version of midda.
    #[error("the saved index is from another version of midda")]
    OtherFormat,
    /// Cut short, or not an index at all.
    #[error("the saved index is damaged: {0}")]
    Damaged(String),
    /// The folder at this path is not the folder the index describes: it was
    /// deleted and made again since.
    #[error("the folder was replaced since the index was saved")]
    Replaced,
}

/// A saved index, read back.
#[derive(Debug)]
pub struct Saved {
    pub index: Index,
    /// When it was written.
    pub saved_at: SystemTime,
}

/// Reads the index saved for `root`, if there is one and it still describes
/// the folder at that path.
///
/// # Errors
///
/// Why it could not be used. Every reason is a reason to scan instead.
pub fn load(directory: &Path, root: &Path) -> Result<Saved, Unusable> {
    let saved = load_file(&file_for(directory, root))?;
    if !same_folder(saved.index.tree().root_path(), root) {
        return Err(Unusable::Damaged("it describes another folder".into()));
    }
    if let (Some(then), Some(now)) = (saved.index.tree().root().identity, crate::platform::own(root).identity)
        && then != now
    {
        return Err(Unusable::Replaced);
    }
    Ok(saved)
}

/// Reads the index in the file at `path`, whatever folder it describes and
/// whether or not that folder is still the one at its path.
///
/// What a snapshot is read with: a picture of a folder since deleted and
/// made again is still a picture of that path at that moment.
///
/// # Errors
///
/// Why it could not be read.
pub fn load_file(path: &Path) -> Result<Saved, Unusable> {
    read(&mut open(path)?).map_err(Unusable::from)
}

/// What a saved index says about itself, read without its nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The folder it is a picture of.
    pub root: PathBuf,
    /// When that folder was last read in full before it was saved.
    pub scanned_at: SystemTime,
    /// When it was written: the moment the picture is of.
    pub saved_at: SystemTime,
}

/// Reads what the file at `path` says about itself: a few hundred bytes, not
/// the hundred megabytes after them. What a list of snapshots is made from.
///
/// # Errors
///
/// Why the file is not an index this version can read.
pub fn header(path: &Path) -> Result<Header, Unusable> {
    let head = read_head(&mut open(path)?).map_err(Unusable::from)?;
    Ok(Header {
        root: head.root,
        scanned_at: head.scanned_at,
        saved_at: head.saved_at,
    })
}

/// Whether two paths name the same folder, as the volume compares them.
#[must_use]
pub fn same_folder(a: &Path, b: &Path) -> bool {
    key_of(a) == key_of(b)
}

fn open(path: &Path) -> Result<BufReader<File>, Unusable> {
    match File::open(path) {
        Ok(file) => Ok(BufReader::with_capacity(1 << 20, file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(Unusable::Missing),
        Err(error) => Err(Unusable::Damaged(error.to_string())),
    }
}

impl From<Problem> for Unusable {
    fn from(problem: Problem) -> Self {
        match problem {
            Problem::Format => Self::OtherFormat,
            Problem::Damaged(why) => Self::Damaged(why),
        }
    }
}

fn write(out: &mut impl Write, index: &Index, saved_at: SystemTime) -> std::io::Result<()> {
    let tree = index.tree();
    out.write_all(MAGIC)?;
    out.write_all(&FORMAT.to_le_bytes())?;

    text(out, &tree.root_path().to_string_lossy())?;
    text(out, tree.scanned_by())?;
    optional_text(out, tree.fallback())?;
    optional(out, tree.cluster_bytes())?;
    time(out, Some(index.scanned_at()))?;
    time(out, Some(saved_at))?;
    match index.journal() {
        Some(at) => {
            out.write_all(&[1])?;
            varint(out, at.journal)?;
            signed(out, at.next)?;
        }
        None => out.write_all(&[0])?,
    }
    let shared = tree.shared();
    varint(out, shared.shared_names)?;
    varint(out, shared.reclaimed.logical)?;
    varint(out, shared.reclaimed.allocated)?;
    varint(out, tree.skipped().len() as u64)?;
    for skipped in tree.skipped() {
        text(out, &skipped.path.to_string_lossy())?;
        text(out, &skipped.reason)?;
    }

    // Scan order: level by level, each folder's children in listing order —
    // which is the order they are held in. A node's parent is written as its
    // place in this order, so the reader can rebuild the arena by pushing.
    let order = scan_order(tree);
    let mut place = vec![NodeId::MAX; order.iter().copied().max().map_or(0, |max| max as usize + 1)];
    for (at, &id) in order.iter().enumerate() {
        place[id as usize] = NodeId::try_from(at).expect("fewer nodes than u32::MAX");
    }
    varint(out, order.len() as u64)?;
    let volume = tree.root().identity.map(|identity| identity.volume);
    for &id in &order {
        let node = tree.node(id);
        // The root names the volume; every node after it names one only when
        // it is on another.
        let named = if id == ROOT { None } else { volume };
        node_record(out, node, place[node.parent as usize], named)?;
    }
    out.write_all(TRAILER)?;
    varint(out, order.len() as u64)
}

/// Every live node, level by level, each folder's children in their order.
fn scan_order(tree: &Tree) -> Vec<NodeId> {
    let mut order = vec![ROOT];
    let mut next = 0;
    while next < order.len() {
        let id = order[next];
        order.extend_from_slice(&tree.node(id).children);
        next += 1;
    }
    order
}

fn node_record(out: &mut impl Write, node: &Node, parent: NodeId, volume: Option<u64>) -> std::io::Result<()> {
    let mut flags = 0;
    if node.is_directory() {
        flags |= DIRECTORY;
    }
    if node.modified.is_some() {
        flags |= HAS_MODIFIED;
    }
    if node.links.is_some() {
        flags |= HAS_LINKS;
    }
    if let Some(identity) = node.identity {
        flags |= HAS_IDENTITY;
        if Some(identity.volume) != volume {
            flags |= OTHER_VOLUME;
        }
    }
    text(out, &node.name)?;
    varint(out, u64::from(parent))?;
    out.write_all(&[flags])?;
    if !node.is_directory() {
        varint(out, node.size.logical)?;
        varint(out, node.size.allocated)?;
        out.write_all(&[node.traits.bits()])?;
    }
    if let Some(modified) = node.modified {
        time(out, Some(modified))?;
    }
    if let Some(links) = node.links {
        varint(out, u64::from(links))?;
    }
    if let Some(identity) = node.identity {
        if flags & OTHER_VOLUME != 0 {
            varint(out, identity.volume)?;
        }
        wide(out, identity.file)?;
    }
    Ok(())
}

/// What reading went wrong on.
enum Problem {
    Format,
    Damaged(String),
}

impl From<std::io::Error> for Problem {
    fn from(error: std::io::Error) -> Self {
        Self::Damaged(error.to_string())
    }
}

fn damaged(why: &str) -> Problem {
    Problem::Damaged(why.to_owned())
}

/// The fields before the journal position: everything that says what the
/// file is a picture of, and when.
struct Head {
    root: PathBuf,
    scanned_by: String,
    fallback: Option<String>,
    cluster_bytes: Option<u64>,
    scanned_at: SystemTime,
    saved_at: SystemTime,
}

fn read_head(input: &mut impl Read) -> Result<Head, Problem> {
    let mut magic = [0_u8; 4];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(damaged("it is not a midda index"));
    }
    let mut version = [0_u8; 4];
    input.read_exact(&mut version)?;
    if u32::from_le_bytes(version) != FORMAT {
        return Err(Problem::Format);
    }

    Ok(Head {
        root: PathBuf::from(read_text(input)?),
        scanned_by: read_text(input)?,
        fallback: read_optional_text(input)?,
        cluster_bytes: read_optional(input)?,
        scanned_at: read_time(input)?.ok_or_else(|| damaged("no scan time"))?,
        saved_at: read_time(input)?.ok_or_else(|| damaged("no save time"))?,
    })
}

fn read(input: &mut impl Read) -> Result<Saved, Problem> {
    let Head {
        root,
        scanned_by,
        fallback,
        cluster_bytes,
        scanned_at,
        saved_at,
    } = read_head(input)?;
    let journal = match byte(input)? {
        0 => None,
        1 => Some(JournalPosition {
            journal: read_varint(input)?,
            next: read_signed(input)?,
        }),
        _ => return Err(damaged("a journal position that is neither there nor not")),
    };
    let shared = Deduplicated {
        shared_names: read_varint(input)?,
        reclaimed: Size {
            logical: read_varint(input)?,
            allocated: read_varint(input)?,
        },
    };

    let mut tree = Tree::new(root, cluster_bytes);
    tree.record_scanner(&scanned_by);
    if let Some(reason) = fallback {
        tree.record_fallback(reason);
    }
    let skipped = read_varint(input)?;
    for _ in 0..skipped {
        let path = PathBuf::from(read_text(input)?);
        tree.skip(path, read_text(input)?);
    }

    let count = read_varint(input)?;
    if count == 0 {
        return Err(damaged("no root"));
    }
    let root_node = read_node(input, None)?;
    let volume = root_node.identity.map(|identity| identity.volume);
    tree.stamp_root(root_node.modified, root_node.identity);
    for at in 1..count {
        let node = read_node(input, volume)?;
        if u64::from(node.parent) >= at {
            return Err(damaged("a node listed before its folder"));
        }
        if !tree.node(node.parent).is_directory() {
            return Err(damaged("a node inside a file"));
        }
        tree.push(node.parent, node);
    }

    let mut trailer = [0_u8; 4];
    input.read_exact(&mut trailer)?;
    if &trailer != TRAILER || read_varint(input)? != count {
        return Err(damaged("it was cut short"));
    }

    tree.roll_up();
    tree.record_shared(shared);
    let mut index = Index::new(tree, scanned_at);
    index.follow(journal);
    Ok(Saved { index, saved_at })
}

fn read_node(input: &mut impl Read, volume: Option<u64>) -> Result<Node, Problem> {
    let name = read_text(input)?;
    let parent = NodeId::try_from(read_varint(input)?).map_err(|_| damaged("a parent past any tree"))?;
    let flags = byte(input)?;
    let kind = if flags & DIRECTORY == 0 { Kind::File } else { Kind::Directory };
    let mut node = Node::new(name, parent, kind);
    if kind == Kind::File {
        node.size = Size {
            logical: read_varint(input)?,
            allocated: read_varint(input)?,
        };
        node.traits = Traits::from_bits(byte(input)?);
    }
    if flags & HAS_MODIFIED != 0 {
        node.modified = read_time(input)?;
        node.subtree_modified = node.modified;
    }
    if flags & HAS_LINKS != 0 {
        node.links = Some(u32::try_from(read_varint(input)?).map_err(|_| damaged("a link count past any file"))?);
    }
    if flags & HAS_IDENTITY != 0 {
        let volume = if flags & OTHER_VOLUME == 0 {
            volume.ok_or_else(|| damaged("an identity on a volume never named"))?
        } else {
            read_varint(input)?
        };
        node.identity = Some(FileIdentity {
            volume,
            file: read_wide(input)?,
        });
    }
    Ok(node)
}

fn varint(out: &mut impl Write, mut value: u64) -> std::io::Result<()> {
    let mut bytes = [0_u8; 10];
    let mut length = 0;
    loop {
        // Seven bits at a time, low first; the high bit says more follow.
        #[allow(clippy::cast_possible_truncation, reason = "masked to seven bits")]
        let low = (value & 0x7F) as u8;
        value >>= 7;
        bytes[length] = if value == 0 { low } else { low | 0x80 };
        length += 1;
        if value == 0 {
            break;
        }
    }
    out.write_all(&bytes[..length])
}

fn wide(out: &mut impl Write, value: u128) -> std::io::Result<()> {
    // The high half of an NTFS file id is zero; a varint of each half keeps
    // the common case at the length of the low one.
    #[allow(clippy::cast_possible_truncation, reason = "split into halves on purpose")]
    let (low, high) = (value as u64, (value >> 64) as u64);
    varint(out, low)?;
    varint(out, high)
}

fn signed(out: &mut impl Write, value: i64) -> std::io::Result<()> {
    // Zigzag, so a small negative number is a small varint.
    #[allow(clippy::cast_sign_loss, reason = "zigzag maps the sign into the low bit")]
    varint(out, ((value << 1) ^ (value >> 63)) as u64)
}

fn text(out: &mut impl Write, value: &str) -> std::io::Result<()> {
    varint(out, value.len() as u64)?;
    out.write_all(value.as_bytes())
}

fn optional_text(out: &mut impl Write, value: Option<&str>) -> std::io::Result<()> {
    match value {
        Some(value) => {
            out.write_all(&[1])?;
            text(out, value)
        }
        None => out.write_all(&[0]),
    }
}

fn optional(out: &mut impl Write, value: Option<u64>) -> std::io::Result<()> {
    match value {
        Some(value) => {
            out.write_all(&[1])?;
            varint(out, value)
        }
        None => out.write_all(&[0]),
    }
}

/// A time as seconds and nanoseconds either side of the Unix epoch: exact on
/// every platform, where a `FILETIME` would round a Linux timestamp.
fn time(out: &mut impl Write, value: Option<SystemTime>) -> std::io::Result<()> {
    let Some(value) = value else {
        return out.write_all(&[0]);
    };
    let (after, since) = match value.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => (true, since),
        Err(before) => (false, before.duration()),
    };
    out.write_all(&[if after { 1 } else { 2 }])?;
    varint(out, since.as_secs())?;
    varint(out, u64::from(since.subsec_nanos()))
}

fn byte(input: &mut impl Read) -> Result<u8, Problem> {
    let mut one = [0_u8; 1];
    input.read_exact(&mut one)?;
    Ok(one[0])
}

fn read_varint(input: &mut impl Read) -> Result<u64, Problem> {
    let mut value = 0_u64;
    for shift in (0..64).step_by(7) {
        let next = byte(input)?;
        value |= u64::from(next & 0x7F) << shift;
        if next & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(damaged("a number longer than any number"))
}

fn read_wide(input: &mut impl Read) -> Result<u128, Problem> {
    let low = read_varint(input)?;
    let high = read_varint(input)?;
    Ok(u128::from(low) | (u128::from(high) << 64))
}

fn read_signed(input: &mut impl Read) -> Result<i64, Problem> {
    let zigzag = read_varint(input)?;
    #[allow(clippy::cast_possible_wrap, reason = "zigzag maps the low bit back into the sign")]
    Ok((zigzag >> 1) as i64 ^ -((zigzag & 1) as i64))
}

fn read_text(input: &mut impl Read) -> Result<String, Problem> {
    let length = usize::try_from(read_varint(input)?).map_err(|_| damaged("a name past any memory"))?;
    // A name, a path, a reason: none is longer than a few kilobytes, and a
    // length that says otherwise is a damaged file, not a reason to allocate.
    if length > 1 << 20 {
        return Err(damaged("a name longer than any name"));
    }
    let mut bytes = vec![0_u8; length];
    input.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| damaged("a name that is not text"))
}

fn read_optional_text(input: &mut impl Read) -> Result<Option<String>, Problem> {
    match byte(input)? {
        0 => Ok(None),
        1 => read_text(input).map(Some),
        _ => Err(damaged("a value that is neither there nor not")),
    }
}

fn read_optional(input: &mut impl Read) -> Result<Option<u64>, Problem> {
    match byte(input)? {
        0 => Ok(None),
        1 => read_varint(input).map(Some),
        _ => Err(damaged("a value that is neither there nor not")),
    }
}

fn read_time(input: &mut impl Read) -> Result<Option<SystemTime>, Problem> {
    let side = byte(input)?;
    if side == 0 {
        return Ok(None);
    }
    let seconds = read_varint(input)?;
    let nanos = u32::try_from(read_varint(input)?).map_err(|_| damaged("a time with too many nanoseconds"))?;
    let span = Duration::new(seconds, nanos);
    let time = match side {
        1 => SystemTime::UNIX_EPOCH.checked_add(span),
        2 => SystemTime::UNIX_EPOCH.checked_sub(span),
        _ => return Err(damaged("a time on neither side of the epoch")),
    };
    time.map(Some).ok_or_else(|| damaged("a time past any clock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_come_back_as_they_went() {
        let mut bytes = Vec::new();
        for value in [0, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX] {
            varint(&mut bytes, value).expect("write");
        }
        for value in [0_i64, -1, 1, i64::MIN, i64::MAX] {
            signed(&mut bytes, value).expect("write");
        }
        wide(&mut bytes, 0x0005_0000_0000_1234).expect("write");
        wide(&mut bytes, u128::MAX).expect("write");

        let mut input = bytes.as_slice();
        for value in [0, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX] {
            assert_eq!(read_varint(&mut input).ok(), Some(value));
        }
        for value in [0_i64, -1, 1, i64::MIN, i64::MAX] {
            assert_eq!(read_signed(&mut input).ok(), Some(value));
        }
        assert_eq!(read_wide(&mut input).ok(), Some(0x0005_0000_0000_1234));
        assert_eq!(read_wide(&mut input).ok(), Some(u128::MAX));
        assert!(input.is_empty());
    }

    #[test]
    fn a_time_comes_back_to_the_nanosecond_either_side_of_the_epoch() {
        let after = SystemTime::UNIX_EPOCH + Duration::new(1_790_000_000, 123_456_789);
        let before = SystemTime::UNIX_EPOCH - Duration::new(86_400 * 365, 1);
        let mut bytes = Vec::new();
        for value in [Some(after), Some(before), None] {
            time(&mut bytes, value).expect("write");
        }
        let mut input = bytes.as_slice();
        assert_eq!(read_time(&mut input).ok(), Some(Some(after)));
        assert_eq!(read_time(&mut input).ok(), Some(Some(before)));
        assert_eq!(read_time(&mut input).ok(), Some(None));
    }

    #[test]
    fn a_folder_has_one_file_name_however_its_path_is_typed() {
        let directory = Path::new("store");
        if cfg!(windows) {
            assert_eq!(file_for(directory, Path::new(r"C:\Projects")), file_for(directory, Path::new("c:/projects/")));
        }
        assert_ne!(file_for(directory, Path::new("/a")), file_for(directory, Path::new("/b")));
    }
}
