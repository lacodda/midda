//! What is on a virtual disk: the filesystems on it, how big each is and how
//! much of it is in use, read from the disk's own bytes.
//!
//! A WSL or Docker disk is one ext4 filesystem across the whole disk, with no
//! partition table; a Hyper-V machine's disk is partitioned, Windows on NTFS.
//! Both are read here without mounting anything and without starting the
//! machine: an ext4 superblock says how many blocks are free, an NTFS volume's
//! `$Bitmap` says which clusters are in use. What is not one of the two is
//! named when it can be and counted as wholly in use — the answer that never
//! promises more space than there is.

use crate::mft::format;

/// A disk whose bytes can be read at any offset.
pub trait Disk {
    /// How big the disk is.
    fn size(&self) -> u64;

    /// The disk's sector: what a partition table counts in.
    fn sector(&self) -> u64;

    /// Reads `into.len()` bytes at `at`.
    ///
    /// # Errors
    ///
    /// Past the end of the disk, or when the bytes cannot be read.
    fn read_at(&mut self, at: u64, into: &mut [u8]) -> std::io::Result<()>;
}

impl<R: std::io::Read + std::io::Seek> Disk for super::vhdx::Vhdx<R> {
    fn size(&self) -> u64 {
        self.capacity
    }

    fn sector(&self) -> u64 {
        self.sector_bytes
    }

    fn read_at(&mut self, at: u64, into: &mut [u8]) -> std::io::Result<()> {
        Self::read_at(self, at, into)
    }
}

/// One filesystem on a disk, or one stretch of it nothing here reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// What it is: `ext4`, `NTFS`, `FAT32` - or `None` when nothing here
    /// knows it.
    pub filesystem: Option<&'static str>,
    /// Its own name, when it has one.
    pub label: Option<String>,
    /// Where it starts on the disk.
    pub at: u64,
    pub capacity: u64,
    /// What of it is in use, when it could be read.
    pub used: Option<u64>,
}

/// What is on a disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contents {
    /// The filesystems, in the order they are on the disk.
    pub volumes: Vec<Volume>,
}

impl Contents {
    /// What the disk's filesystems use: what is measured, and all of what
    /// is not. `None` when nothing on the disk was found.
    #[must_use]
    pub fn used(&self) -> Option<u64> {
        if self.volumes.is_empty() {
            return None;
        }
        Some(self.volumes.iter().map(|volume| volume.used.unwrap_or(volume.capacity)).sum())
    }

    /// Whether every filesystem on the disk was measured.
    #[must_use]
    pub fn measured(&self) -> bool {
        !self.volumes.is_empty() && self.volumes.iter().all(|volume| volume.used.is_some())
    }
}

/// The most partitions a table is read for: GPT reserves room for 128.
const PARTITIONS: usize = 128;

/// Reads what is on `disk`.
///
/// # Errors
///
/// When the disk's first bytes cannot be read.
pub fn contents(disk: &mut impl Disk) -> Result<Contents, String> {
    let size = disk.size();
    if let Some(volume) = volume_at(disk, 0, size) {
        return Ok(Contents { volumes: vec![volume] });
    }
    let mut first = vec![0_u8; 512];
    disk.read_at(0, &mut first)
        .map_err(|error| format!("its first sector could not be read: {error}"))?;
    let mut volumes = Vec::new();
    for (at, bytes) in partitions(disk, &first) {
        volumes.push(volume_at(disk, at, bytes).unwrap_or(Volume {
            filesystem: None,
            label: None,
            at,
            capacity: bytes,
            used: None,
        }));
    }
    Ok(Contents { volumes })
}

/// The partitions a table at the start of the disk lists, as where each
/// starts and how long it is: from a GUID table when the first sector says
/// there is one, from the first sector's own four entries otherwise.
fn partitions(disk: &mut impl Disk, first: &[u8]) -> Vec<(u64, u64)> {
    if first.get(510..512) != Some(&[0x55, 0xAA]) {
        return Vec::new();
    }
    let sector = disk.sector();
    let size = disk.size();
    let primary: Vec<(u8, u64, u64)> = (0..4)
        .filter_map(|entry| {
            let at = 446 + entry * 16;
            let kind = *first.get(at + 4)?;
            let start = u64::from(u32_at(first, at + 8)?);
            let count = u64::from(u32_at(first, at + 12)?);
            (kind != 0 && count > 0).then_some((kind, start, count))
        })
        .collect();
    if primary.iter().any(|&(kind, _, _)| kind == 0xEE) {
        return guid_partitions(disk, sector, size);
    }
    primary
        .into_iter()
        // An extended partition holds a chain of further tables: what is in
        // it stays one stretch, counted whole.
        .map(|(_, start, count)| (start * sector, count * sector))
        .filter(|&(at, bytes)| at.checked_add(bytes).is_some_and(|end| end <= size))
        .collect()
}

fn guid_partitions(disk: &mut impl Disk, sector: u64, size: u64) -> Vec<(u64, u64)> {
    let mut header = vec![0_u8; 512];
    if disk.read_at(sector, &mut header).is_err() || !header.starts_with(b"EFI PART") {
        return Vec::new();
    }
    let (Some(table), Some(count), Some(entry_bytes)) = (u64_at(&header, 72), u32_at(&header, 80), u32_at(&header, 84)) else {
        return Vec::new();
    };
    let count = usize::try_from(count).unwrap_or(0).min(PARTITIONS);
    let entry_bytes = usize::try_from(entry_bytes).unwrap_or(0);
    if !(128..=4096).contains(&entry_bytes) {
        return Vec::new();
    }
    let mut entries = vec![0_u8; count * entry_bytes];
    if disk.read_at(table * sector, &mut entries).is_err() {
        return Vec::new();
    }
    entries
        .chunks(entry_bytes)
        .filter(|entry| entry[..16].iter().any(|&byte| byte != 0))
        .filter_map(|entry| {
            let first = u64_at(entry, 32)?;
            let last = u64_at(entry, 40)?;
            let at = first.checked_mul(sector)?;
            let bytes = last.checked_sub(first)?.checked_add(1)?.checked_mul(sector)?;
            at.checked_add(bytes).is_some_and(|end| end <= size).then_some((at, bytes))
        })
        .collect()
}

/// The filesystem that starts at `at`, when one this reader knows does.
fn volume_at(disk: &mut impl Disk, at: u64, bytes: u64) -> Option<Volume> {
    let mut boot = vec![0_u8; 2048];
    disk.read_at(at, &mut boot).ok()?;
    ext(&boot[1024..], at, bytes)
        .or_else(|| ntfs(disk, &boot[..512], at, bytes))
        .or_else(|| fat(&boot[..512], at, bytes))
}

/// An ext2, ext3 or ext4 filesystem, from its superblock: how many blocks it
/// has and how many are free. The free count is the one the filesystem keeps
/// for itself, written when it is unmounted - and now and then while it is.
///
/// What is in use is what `df` says inside: the blocks neither free nor kept
/// by the filesystem for itself from the day it was made ([`overhead`]). A
/// terabyte of ext4 keeps 17 GB for its inode tables, and counting those as
/// used would say a 150 MB disk file holds 17 GB.
fn ext(superblock: &[u8], at: u64, bytes: u64) -> Option<Volume> {
    if u16_at(superblock, 0x38)? != 0xEF53 {
        return None;
    }
    let shift = u32_at(superblock, 0x18)?;
    if shift > 6 {
        return None;
    }
    let block = 1024_u64 << shift;
    let features = Features {
        compatible: u32_at(superblock, 0x5C)?,
        incompatible: u32_at(superblock, 0x60)?,
        read_only: u32_at(superblock, 0x64)?,
    };
    let incompatible = features.incompatible;
    let wide = incompatible & 0x80 != 0;
    let count = |low: usize, high: usize| -> Option<u64> {
        let low = u64::from(u32_at(superblock, low)?);
        let high = if wide { u64::from(u32_at(superblock, high)?) } else { 0 };
        Some(high << 32 | low)
    };
    let blocks = count(0x04, 0x150)?;
    let free = count(0x0C, 0x158)?;
    if free > blocks {
        return None;
    }
    let capacity = blocks.checked_mul(block)?;
    // A filesystem larger than the stretch it is in is not one to believe.
    if capacity > bytes {
        return None;
    }
    let filesystem = if incompatible & 0x40 != 0 {
        "ext4"
    } else if features.compatible & HAS_JOURNAL != 0 {
        "ext3"
    } else {
        "ext2"
    };
    let used = overhead(superblock, blocks, block, features)
        .and_then(|kept| (blocks - free).checked_sub(kept))
        .and_then(|used| used.checked_mul(block));
    Some(Volume {
        filesystem: Some(filesystem),
        label: label(superblock.get(0x78..0x88)?),
        at,
        capacity,
        used,
    })
}

/// The feature bits of an ext filesystem.
#[derive(Debug, Clone, Copy)]
struct Features {
    compatible: u32,
    incompatible: u32,
    read_only: u32,
}

const HAS_JOURNAL: u32 = 0x4;
const SPARSE_SUPER2: u32 = 0x200;
const META_BG: u32 = 0x10;
const WIDE: u32 = 0x80;
const SPARSE_SUPER: u32 = 0x1;
const BIGALLOC: u32 = 0x200;

/// The blocks an ext filesystem keeps for itself from the day it was made:
/// the two bitmaps and the inode table of every group, the copies of the
/// superblock and of the group table, and the journal - what Linux leaves out
/// of a filesystem's size, as the kernel counts it.
///
/// `None` for a layout this does not count - groups of their own table
/// (`meta_bg`), clusters (`bigalloc`), copies placed by hand
/// (`sparse_super2`), a journal whose size the superblock does not keep: then
/// what is in use goes unsaid rather than said wrong.
fn overhead(superblock: &[u8], blocks: u64, block: u64, features: Features) -> Option<u64> {
    let recorded = u64::from(u32_at(superblock, 0x248)?);
    if recorded != 0 && features.read_only & BIGALLOC == 0 {
        return Some(recorded);
    }
    if features.incompatible & META_BG != 0 || features.compatible & SPARSE_SUPER2 != 0 || features.read_only & BIGALLOC != 0 {
        return None;
    }
    let first = u64::from(u32_at(superblock, 0x14)?);
    let per_group = u64::from(u32_at(superblock, 0x20)?);
    let inodes = u64::from(u32_at(superblock, 0x28)?);
    let inode_bytes = if u32_at(superblock, 0x4C)? >= 1 {
        u64::from(u16_at(superblock, 0x58)?)
    } else {
        128
    };
    let descriptor = if features.incompatible & WIDE != 0 {
        u64::from(u16_at(superblock, 0xFE)?).max(32)
    } else {
        32
    };
    if per_group == 0 || inode_bytes == 0 {
        return None;
    }
    let groups = blocks.checked_sub(first)?.div_ceil(per_group);
    let table = inodes.checked_mul(inode_bytes)?.div_ceil(block);
    let descriptors = groups.checked_mul(descriptor)?.div_ceil(block);
    let reserved = u64::from(u16_at(superblock, 0xCE)?);
    let sparse = features.read_only & SPARSE_SUPER != 0;
    let copies = (0..groups).filter(|&group| !sparse || has_copy(group)).count() as u64;
    let journal = if features.compatible & HAS_JOURNAL != 0 && u32_at(superblock, 0xE0)? != 0 {
        // The journal's size is kept in the superblock beside the copy of
        // its inode's block map, when that copy is kept.
        if superblock.get(0xFD)? != &1 {
            return None;
        }
        let high = u64::from(u32_at(superblock, 0x10C + 15 * 4)?);
        let low = u64::from(u32_at(superblock, 0x10C + 16 * 4)?);
        (high << 32 | low).div_ceil(block)
    } else {
        0
    };
    first
        .checked_add(groups.checked_mul(2 + table)?)?
        .checked_add(copies.checked_mul(1 + descriptors + reserved)?)?
        .checked_add(journal)
}

/// Whether a group holds a copy of the superblock, on a filesystem that
/// keeps copies only in some: the first two, and the powers of 3, 5 and 7.
fn has_copy(group: u64) -> bool {
    group <= 1 || [3, 5, 7].into_iter().any(|base| power_of(group, base))
}

fn power_of(mut number: u64, base: u64) -> bool {
    while number.is_multiple_of(base) {
        number /= base;
    }
    number == 1
}

/// An NTFS volume: its size from the boot sector, what is in use from its
/// `$Bitmap`, which has a bit for every cluster.
fn ntfs(disk: &mut impl Disk, boot: &[u8], at: u64, bytes: u64) -> Option<Volume> {
    let geometry = format::geometry(boot)?;
    let sectors = u64_at(boot, 0x28)?;
    let capacity = sectors.checked_mul(geometry.sector_bytes)?.min(bytes);
    let clusters = capacity / geometry.cluster_bytes;
    let used = ntfs_used(disk, &geometry, at, clusters);
    Some(Volume {
        filesystem: Some("NTFS"),
        label: None,
        at,
        capacity,
        used,
    })
}

/// The clusters `$Bitmap` says are in use, as bytes.
fn ntfs_used(disk: &mut impl Disk, geometry: &format::Geometry, at: u64, clusters: u64) -> Option<u64> {
    // `$Bitmap` is record 6; the first sixteen records are always together
    // at the start of the table.
    let mut record = vec![0_u8; usize::try_from(geometry.record_bytes).ok()?];
    disk.read_at(at.checked_add(geometry.table_at)?.checked_add(6 * geometry.record_bytes)?, &mut record)
        .ok()?;
    if !format::fix_up(&mut record) {
        return None;
    }
    let header = format::header(&record)?;
    let data = format::attributes(&record, &header).find(|attribute| attribute.kind() == format::DATA && attribute.name().is_none())?;
    let bitmap = match data.value() {
        Some(value) => value.to_vec(),
        None => {
            let (length, runs) = data.runs(geometry.cluster_bytes).ok()?;
            // A bit per cluster: 64 MiB of bitmap is a two-petabyte volume.
            if length > 64 << 20 {
                return None;
            }
            let mut bitmap = Vec::with_capacity(usize::try_from(length).ok()?);
            for run in runs {
                let start = bitmap.len();
                match run {
                    format::Run::Data { at: offset, bytes } => {
                        bitmap.resize(start + usize::try_from(bytes).ok()?, 0);
                        disk.read_at(at.checked_add(offset)?, &mut bitmap[start..]).ok()?;
                    }
                    format::Run::Hole { bytes } => bitmap.resize(start + usize::try_from(bytes).ok()?, 0),
                }
            }
            bitmap.truncate(usize::try_from(length).ok()?);
            bitmap
        }
    };
    let whole = usize::try_from(clusters / 8).ok()?;
    let mut set: u64 = bitmap.get(..whole)?.iter().map(|byte| u64::from(byte.count_ones())).sum();
    let rest = clusters % 8;
    if rest > 0 {
        let mask = (1_u8 << rest) - 1;
        set += u64::from((bitmap.get(whole)? & mask).count_ones());
    }
    set.checked_mul(geometry.cluster_bytes)
}

/// A FAT volume - an EFI system partition - named, not measured.
fn fat(boot: &[u8], at: u64, bytes: u64) -> Option<Volume> {
    if boot.get(510..512)? != [0x55, 0xAA] {
        return None;
    }
    let filesystem = if boot.get(0x52..0x5A)? == b"FAT32   " {
        "FAT32"
    } else if boot.get(0x36..0x39)? == b"FAT" {
        "FAT"
    } else {
        return None;
    };
    Some(Volume {
        filesystem: Some(filesystem),
        label: None,
        at,
        capacity: bytes,
        used: None,
    })
}

/// A name written in a fixed field, up to its first zero.
fn label(field: &[u8]) -> Option<String> {
    let end = field.iter().position(|&byte| byte == 0).unwrap_or(field.len());
    let text = String::from_utf8_lossy(&field[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A disk that is its bytes.
    pub(crate) struct Raw {
        pub bytes: Vec<u8>,
        pub sector: u64,
    }

    impl Disk for Raw {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }

        fn sector(&self) -> u64 {
            self.sector
        }

        fn read_at(&mut self, at: u64, into: &mut [u8]) -> std::io::Result<()> {
            let at = usize::try_from(at).map_err(std::io::Error::other)?;
            let bytes = self
                .bytes
                .get(at..at + into.len())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "past the end"))?;
            into.copy_from_slice(bytes);
            Ok(())
        }
    }

    /// An ext4 superblock: `blocks` of 4 KiB in groups of 32 768, `free` of
    /// them free, 64-bit counts when `wide`, sixteen inodes of 256 bytes a
    /// group, written at byte 1024 of `disk` from `at`. One group keeps five
    /// blocks for itself: two bitmaps, an inode table of one block, the
    /// superblock and a group table of one block.
    pub(crate) fn ext4(disk: &mut [u8], at: usize, blocks: u64, free: u64, wide: bool, name: &str) {
        let superblock = &mut disk[at + 1024..at + 2048];
        superblock[0x04..0x08].copy_from_slice(&(blocks as u32).to_le_bytes());
        superblock[0x0C..0x10].copy_from_slice(&(free as u32).to_le_bytes());
        superblock[0x18..0x1C].copy_from_slice(&2_u32.to_le_bytes());
        superblock[0x20..0x24].copy_from_slice(&32_768_u32.to_le_bytes());
        superblock[0x28..0x2C].copy_from_slice(&16_u32.to_le_bytes());
        superblock[0x38..0x3A].copy_from_slice(&0xEF53_u16.to_le_bytes());
        superblock[0x4C..0x50].copy_from_slice(&1_u32.to_le_bytes());
        superblock[0x58..0x5A].copy_from_slice(&256_u16.to_le_bytes());
        let incompatible: u32 = 0x40 | if wide { WIDE } else { 0 };
        superblock[0x60..0x64].copy_from_slice(&incompatible.to_le_bytes());
        superblock[0x64..0x68].copy_from_slice(&SPARSE_SUPER.to_le_bytes());
        if wide {
            superblock[0xFE..0x100].copy_from_slice(&64_u16.to_le_bytes());
            superblock[0x150..0x154].copy_from_slice(&((blocks >> 32) as u32).to_le_bytes());
            superblock[0x158..0x15C].copy_from_slice(&((free >> 32) as u32).to_le_bytes());
        }
        superblock[0x78..0x78 + name.len()].copy_from_slice(name.as_bytes());
    }

    #[test]
    fn a_whole_disk_ext4_is_measured_from_its_superblock() {
        let mut bytes = vec![0_u8; 64 * 4096];
        ext4(&mut bytes, 0, 64, 40, false, "data");
        let contents = contents(&mut Raw { bytes, sector: 512 }).expect("reads");
        assert_eq!(
            contents.volumes,
            [Volume {
                filesystem: Some("ext4"),
                label: Some("data".to_owned()),
                at: 0,
                capacity: 64 * 4096,
                used: Some(19 * 4096),
            }],
            "24 blocks not free, five of them the filesystem's own"
        );
        assert_eq!(contents.used(), Some(19 * 4096));
        assert!(contents.measured());
    }

    #[test]
    fn what_a_terabyte_of_ext4_keeps_for_itself_is_counted_as_the_kernel_counts_it() {
        // WSL's 1 TB disk: 8192 groups of 32 768 blocks, 8192 inodes of 256
        // bytes a group, 64-byte descriptors, 1024 blocks kept for the table
        // to grow, a 1 GiB journal. Copies of the superblock are in groups 0
        // and 1 and the powers of 3, 5 and 7 below 8192: 19 of them.
        let mut superblock = vec![0_u8; 1024];
        superblock[0x14..0x18].copy_from_slice(&0_u32.to_le_bytes());
        superblock[0x20..0x24].copy_from_slice(&32_768_u32.to_le_bytes());
        superblock[0x28..0x2C].copy_from_slice(&8192_u32.to_le_bytes());
        superblock[0x4C..0x50].copy_from_slice(&1_u32.to_le_bytes());
        superblock[0x58..0x5A].copy_from_slice(&256_u16.to_le_bytes());
        superblock[0xCE..0xD0].copy_from_slice(&1024_u16.to_le_bytes());
        superblock[0xE0..0xE4].copy_from_slice(&8_u32.to_le_bytes());
        superblock[0xFD] = 1;
        superblock[0xFE..0x100].copy_from_slice(&64_u16.to_le_bytes());
        superblock[0x10C + 16 * 4..0x10C + 17 * 4].copy_from_slice(&(1_u32 << 30).to_le_bytes());
        let features = Features {
            compatible: HAS_JOURNAL,
            incompatible: 0x40 | WIDE,
            read_only: SPARSE_SUPER,
        };
        let kept = overhead(&superblock, 268_435_456, 4096, features).expect("counted");
        assert_eq!(kept, 8192 * (2 + 512) + 19 * (1 + 128 + 1024) + 262_144);

        // What the filesystem records itself is taken as it is.
        superblock[0x248..0x24C].copy_from_slice(&777_u32.to_le_bytes());
        assert_eq!(overhead(&superblock, 268_435_456, 4096, features), Some(777));
        superblock[0x248..0x24C].fill(0);

        // A layout this does not count goes unsaid.
        let meta = Features {
            incompatible: features.incompatible | META_BG,
            ..features
        };
        assert_eq!(overhead(&superblock, 268_435_456, 4096, meta), None);
        superblock[0xFD] = 0;
        assert_eq!(overhead(&superblock, 268_435_456, 4096, features), None, "a journal of no known size");
    }

    #[test]
    fn a_copy_of_the_superblock_is_in_the_first_two_groups_and_the_powers_of_three_five_and_seven() {
        let copies: Vec<u64> = (0..130).filter(|&group| has_copy(group)).collect();
        assert_eq!(copies, [0, 1, 3, 5, 7, 9, 25, 27, 49, 81, 125]);
    }

    #[test]
    fn the_high_half_of_a_count_is_read_only_on_a_64_bit_filesystem() {
        // The same bytes past 0x150 mean nothing on a filesystem without
        // 64-bit counts.
        let superblock = |wide: bool| {
            let mut bytes = vec![0_u8; 2048];
            ext4(&mut bytes, 0, 100, 30, wide, "");
            bytes[1024 + 0x150..1024 + 0x154].copy_from_slice(&0_u32.to_le_bytes());
            bytes[1024 + 0x158..1024 + 0x15C].copy_from_slice(&7_u32.to_le_bytes());
            bytes
        };
        let narrow = ext(&superblock(false)[1024..], 0, u64::MAX).expect("reads");
        assert_eq!(narrow.used, Some((70 - 5) * 4096));
        // Wide: seven times 2^32 blocks free of a hundred is no filesystem.
        assert_eq!(ext(&superblock(true)[1024..], 0, u64::MAX), None);
    }

    #[test]
    fn a_filesystem_larger_than_its_disk_is_not_believed() {
        let mut bytes = vec![0_u8; 16 * 4096];
        ext4(&mut bytes, 0, 64, 40, false, "");
        assert_eq!(contents(&mut Raw { bytes, sector: 512 }).expect("reads").volumes, []);
    }

    /// A first sector with the given partitions, each a type and a stretch in
    /// sectors.
    fn mbr(disk: &mut [u8], partitions: &[(u8, u32, u32)]) {
        for (entry, &(kind, start, count)) in partitions.iter().enumerate() {
            let at = 446 + entry * 16;
            disk[at + 4] = kind;
            disk[at + 8..at + 12].copy_from_slice(&start.to_le_bytes());
            disk[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        }
        disk[510] = 0x55;
        disk[511] = 0xAA;
    }

    #[test]
    fn each_partition_of_a_table_is_its_own_filesystem() {
        let mut bytes = vec![0_u8; 400 * 4096];
        // One ext4 at sector 2048 (1 MiB), and a stretch nothing reads.
        mbr(&mut bytes, &[(0x83, 2048, 64 * 8), (0x07, 2048 + 64 * 8, 100)]);
        ext4(&mut bytes, 2048 * 512, 64, 16, false, "");
        let contents = contents(&mut Raw { bytes, sector: 512 }).expect("reads");
        assert_eq!(contents.volumes.len(), 2);
        assert_eq!(contents.volumes[0].used, Some(43 * 4096));
        assert_eq!((contents.volumes[1].filesystem, contents.volumes[1].used), (None, None));
        assert_eq!(contents.used(), Some(43 * 4096 + 100 * 512), "what is not measured counts as wholly in use");
        assert!(!contents.measured());
    }

    #[test]
    fn a_partition_the_table_puts_past_the_end_of_the_disk_is_not_one() {
        // A damaged or foreign table: counted, its stretch would be "in use"
        // whole, on a disk that does not have it.
        let mut bytes = vec![0_u8; 400 * 4096];
        mbr(&mut bytes, &[(0x83, 2048, 64 * 8), (0x07, 100_000, 1_000_000)]);
        ext4(&mut bytes, 2048 * 512, 64, 16, false, "");
        let contents = contents(&mut Raw { bytes, sector: 512 }).expect("reads");
        assert_eq!(contents.volumes.len(), 1, "{:?}", contents.volumes);
        assert_eq!(contents.used(), Some(43 * 4096));
    }

    #[test]
    fn a_guid_table_is_read_past_its_protective_first_sector() {
        let sector = 512_usize;
        let mut bytes = vec![0_u8; 400 * 4096];
        mbr(&mut bytes, &[(0xEE, 1, 0xFFFF_FFFF)]);
        let header = &mut bytes[sector..2 * sector];
        header[..8].copy_from_slice(b"EFI PART");
        header[72..80].copy_from_slice(&2_u64.to_le_bytes());
        header[80..84].copy_from_slice(&128_u32.to_le_bytes());
        header[84..88].copy_from_slice(&128_u32.to_le_bytes());
        // One partition, sectors 2048 to 2048 + 512 - 1, of any type.
        let entry = &mut bytes[2 * sector..2 * sector + 128];
        entry[..16].fill(0xAB);
        entry[32..40].copy_from_slice(&2048_u64.to_le_bytes());
        entry[40..48].copy_from_slice(&(2048_u64 + 512 - 1).to_le_bytes());
        ext4(&mut bytes, 2048 * sector, 64, 50, false, "");
        let contents = contents(&mut Raw { bytes, sector: 512 }).expect("reads");
        assert_eq!(contents.volumes.len(), 1);
        assert_eq!((contents.volumes[0].at, contents.volumes[0].used), (2048 * 512, Some(9 * 4096)));
    }

    #[test]
    fn an_ntfs_volume_is_measured_from_its_bitmap() {
        // 4 KiB clusters, a table at cluster 4 of 1 KiB records, 100 clusters
        // of which 37 are in use - the bitmap's last byte only partly counts.
        let cluster = 4096_usize;
        let mut bytes = vec![0_u8; 128 * cluster];
        let boot = &mut bytes[..512];
        boot[3..11].copy_from_slice(b"NTFS    ");
        boot[0x0B..0x0D].copy_from_slice(&512_u16.to_le_bytes());
        boot[0x0D] = 8;
        boot[0x28..0x30].copy_from_slice(&(100_u64 * 8).to_le_bytes());
        boot[0x30..0x38].copy_from_slice(&4_u64.to_le_bytes());
        boot[0x40] = 0xF6; // -10: records of 2^10 bytes
        boot[510] = 0x55;
        boot[511] = 0xAA;
        let mut bitmap = vec![0xFF_u8; 12];
        bitmap.push(0b0001_1111);
        // Bits past the hundredth cluster are not clusters.
        bitmap.extend([0xFF; 3]);
        // Bitmap bytes before the last whole one are cleared down to 32
        // clusters in use: four full bytes, then nothing, then the last five.
        bitmap[4..12].fill(0);
        bytes[16 * cluster..16 * cluster + bitmap.len()].copy_from_slice(&bitmap);
        // `$Bitmap`'s record: an unnamed stream of one cluster, at cluster 16.
        let size = crate::size::Size {
            logical: bitmap.len() as u64,
            allocated: 4096,
        };
        let mut record = crate::mft::format::tests::Writer::new(1, format::IN_USE, 0)
            .data("", 0, size, 0, &[0x11, 0x01, 0x10])
            .finish();
        protect(&mut record);
        let at = 4 * cluster + 6 * 1024;
        bytes[at..at + record.len()].copy_from_slice(&record);
        let contents = contents(&mut Raw { bytes, sector: 512 }).expect("reads");
        let volume = &contents.volumes[0];
        assert_eq!(volume.filesystem, Some("NTFS"));
        assert_eq!(volume.capacity, 100 * 4096);
        assert_eq!(volume.used, Some(36 * 4096), "32 + 4 of the clusters, not the bits past the last");
    }

    /// Gives a record the torn-write protection NTFS writes it with: a check
    /// value at the end of each 512-byte stretch, the real bytes at 0x30.
    fn protect(record: &mut [u8]) {
        record[4..6].copy_from_slice(&0x30_u16.to_le_bytes());
        record[6..8].copy_from_slice(&3_u16.to_le_bytes());
        record[0x30..0x32].copy_from_slice(&[1, 0]);
        for stretch in 1..3 {
            let end = stretch * 512;
            let saved = 0x30 + stretch * 2;
            let real = [record[end - 2], record[end - 1]];
            record[saved..saved + 2].copy_from_slice(&real);
            record[end - 2..end].copy_from_slice(&[1, 0]);
        }
    }

    #[test]
    fn a_disk_with_nothing_known_on_it_has_no_contents() {
        let contents = contents(&mut Raw {
            bytes: vec![0_u8; 8192],
            sector: 512,
        })
        .expect("reads");
        assert_eq!(contents.used(), None);
        assert!(!contents.measured());
    }
}
