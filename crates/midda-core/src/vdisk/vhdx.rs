//! The VHDX container (MS-VHDX 1.0), as far as midda reads it: how big the
//! disk inside is, how much of it the file holds, and the disk's bytes at any
//! offset.
//!
//! A VHDX file is a disk cut into blocks of 1 to 256 MiB. A block table says,
//! for each block of the disk, whether the file holds it and where. A block the
//! machine inside never wrote has no room in the file; a block it wrote and then
//! let go of — a trim, a discard — leaves the table, but its room in the file
//! stays until the file is compacted. That room is what compacting gives back,
//! and the table is the only place that says how much of it there is.
//!
//! Pure: bytes in through [`Read`] and [`Seek`], numbers out. The tests write
//! their files with [`tests::Image`].

use std::io::{Read, Seek, SeekFrom};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

/// What a VHDX file starts with.
const SIGNATURE: &[u8; 8] = b"vhdxfile";

/// Where the two copies of the header are; the one written last is current.
const HEADERS: [u64; 2] = [64 * KIB, 128 * KIB];
const HEADER_BYTES: usize = 4 * 1024;

/// Where the two copies of the region table are: they are the same.
const REGIONS: [u64; 2] = [192 * KIB, 256 * KIB];
const REGION_BYTES: usize = 64 * 1024;

/// The regions and metadata items, by their ids as written in the file.
const BAT: [u8; 16] = guid(0x2DC2_7766, 0xF623, 0x4200, [0x9D, 0x64, 0x11, 0x5E, 0x9B, 0xFD, 0x4A, 0x08]);
const METADATA: [u8; 16] = guid(0x8B7C_A206, 0x4790, 0x4B9A, [0xB8, 0xFE, 0x57, 0x5F, 0x05, 0x0F, 0x88, 0x6E]);
const FILE_PARAMETERS: [u8; 16] = guid(0xCAA1_6737, 0xFA36, 0x4D43, [0xB3, 0xB6, 0x33, 0xF0, 0xAA, 0x44, 0xE7, 0x6B]);
const VIRTUAL_DISK_SIZE: [u8; 16] = guid(0x2FA5_4224, 0xCD1B, 0x4876, [0xB2, 0x11, 0x5D, 0xBE, 0xD8, 0x3B, 0xF4, 0xB8]);
const PAGE_83: [u8; 16] = guid(0xBECA_12AB, 0xB2E6, 0x4523, [0x93, 0xEF, 0xC3, 0x09, 0xE0, 0x00, 0xC7, 0x46]);
const LOGICAL_SECTOR: [u8; 16] = guid(0x8141_BF1D, 0xA96F, 0x4709, [0xBA, 0x47, 0xF2, 0x33, 0xA8, 0xFA, 0xAB, 0x5F]);
const PHYSICAL_SECTOR: [u8; 16] = guid(0xCDA3_48C7, 0x445D, 0x4471, [0x9C, 0xC9, 0xE9, 0x88, 0x52, 0x51, 0xC5, 0x56]);
const PARENT_LOCATOR: [u8; 16] = guid(0xA8D3_5F2D, 0xB30B, 0x454D, [0xAB, 0xF7, 0xD3, 0xD8, 0x48, 0x34, 0xAB, 0x0C]);

/// The metadata items this reader knows. A file that requires another is one
/// it cannot read correctly, and the format says to refuse it.
const KNOWN_ITEMS: [[u8; 16]; 6] = [FILE_PARAMETERS, VIRTUAL_DISK_SIZE, PAGE_83, LOGICAL_SECTOR, PHYSICAL_SECTOR, PARENT_LOCATOR];

/// A block table entry's state: the file holds the block whole, or holds the
/// sectors of it a differencing disk wrote.
const FULLY_PRESENT: u64 = 6;
const PARTIALLY_PRESENT: u64 = 7;

/// A GUID the way Windows writes it: the first three fields little-endian.
const fn guid(first: u32, second: u16, third: u16, rest: [u8; 8]) -> [u8; 16] {
    let a = first.to_le_bytes();
    let b = second.to_le_bytes();
    let c = third.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], rest[0], rest[1], rest[2], rest[3], rest[4], rest[5], rest[6], rest[7],
    ]
}

/// CRC-32C, the checksum of every header and region table.
const fn crc_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut at = 0;
    while at < 256 {
        #[allow(clippy::cast_possible_truncation, reason = "below 256")]
        let mut crc = at as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 0 { crc >> 1 } else { (crc >> 1) ^ 0x82F6_3B78 };
            bit += 1;
        }
        table[at] = crc;
        at += 1;
    }
    table
}

static CRC: [u32; 256] = crc_table();

pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    !bytes
        .iter()
        .fold(!0_u32, |crc, &byte| CRC[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8))
}

/// Whether a block of `bytes` carries its own checksum at offset 4: computed
/// with those four bytes as zero.
fn checksummed(bytes: &[u8]) -> bool {
    let Some(stored) = u32_at(bytes, 4) else {
        return false;
    };
    let mut copy = bytes.to_vec();
    copy[4..8].fill(0);
    crc32c(&copy) == stored
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

/// Whether `first` — the first bytes of a file — is a VHDX file's start.
#[must_use]
pub fn is_vhdx(first: &[u8]) -> bool {
    first.starts_with(SIGNATURE)
}

/// A VHDX file, read: its geometry and its block table.
#[derive(Debug)]
pub struct Vhdx<R> {
    file: R,
    /// The size of the disk inside, as the machine sees it.
    pub capacity: u64,
    pub block_bytes: u64,
    /// The disk's sector: what a partition table counts in.
    pub sector_bytes: u64,
    /// The file records changes to another disk — a checkpoint — and the
    /// blocks it does not hold are in that disk, not empty.
    pub differencing: bool,
    /// The log holds writes not yet in their place: the file is open for
    /// writing, or was not closed cleanly. What is read is the disk as of its
    /// last settled write.
    pub pending: bool,
    /// What the file holds besides the disk's blocks: headers, the log, the
    /// metadata and the block table — what a compacted file still is.
    pub overhead: u64,
    /// Per block of the disk: the raw table entry.
    blocks: Vec<u64>,
}

/// One entry of the region table.
struct Region {
    id: [u8; 16],
    at: u64,
    bytes: u64,
    required: bool,
}

impl<R: Read + Seek> Vhdx<R> {
    /// Reads the container's headers, metadata and block table.
    ///
    /// # Errors
    ///
    /// When the file is not a VHDX file, or is one this reader cannot read
    /// truthfully: neither header nor region table checks out, a region or
    /// metadata item it does not know is required, or a size is out of the
    /// format's range. The message says which, to a reader.
    pub fn read(mut file: R) -> Result<Self, String> {
        let mut start = [0_u8; 8];
        read_at(&mut file, 0, &mut start).map_err(|error| format!("its start could not be read: {error}"))?;
        if !is_vhdx(&start) {
            return Err("it is not a VHDX file".to_owned());
        }

        let header = HEADERS
            .iter()
            .filter_map(|&at| {
                let mut bytes = vec![0_u8; HEADER_BYTES];
                read_at(&mut file, at, &mut bytes).ok()?;
                (bytes.starts_with(b"head") && checksummed(&bytes) && u16_at(&bytes, 66) == Some(1)).then_some(bytes)
            })
            .max_by_key(|bytes| u64_at(bytes, 8).unwrap_or(0))
            .ok_or("neither copy of its header checks out")?;
        let pending = header[48..64].iter().any(|&byte| byte != 0);
        let log_bytes = u64::from(u32_at(&header, 68).unwrap_or(0));

        let regions = REGIONS
            .iter()
            .find_map(|&at| region_table(&mut file, at))
            .ok_or("neither copy of its region table checks out")?;
        if regions.iter().any(|region| region.required && region.id != BAT && region.id != METADATA) {
            return Err("it requires a region this reader does not know".to_owned());
        }
        let find = |id: [u8; 16], what: &str| {
            regions
                .iter()
                .find(|region| region.id == id)
                .map(|region| (region.at, region.bytes))
                .ok_or(format!("it has no {what}"))
        };
        let (bat_at, bat_bytes) = find(BAT, "block table")?;
        let (metadata_at, metadata_bytes) = find(METADATA, "metadata")?;

        let metadata = read_region(&mut file, metadata_at, metadata_bytes, "metadata")?;
        let items = metadata_items(&metadata)?;
        let item = |id: [u8; 16]| items.iter().find(|(item, _, _)| *item == id).map(|&(_, at, length)| (at, length));
        let value = |id: [u8; 16], length: usize, what: &str| {
            let (at, held) = item(id).ok_or(format!("its metadata has no {what}"))?;
            if held < length {
                return Err(format!("its {what} is cut short"));
            }
            metadata.get(at..at + length).ok_or(format!("its {what} is outside its metadata"))
        };
        let parameters = value(FILE_PARAMETERS, 8, "file parameters")?;
        let block_bytes = u64::from(u32_at(parameters, 0).unwrap_or(0));
        let differencing = u32_at(parameters, 4).unwrap_or(0) & 2 != 0;
        let capacity = u64_at(value(VIRTUAL_DISK_SIZE, 8, "disk size")?, 0).unwrap_or(0);
        let sector_bytes = u64::from(u32_at(value(LOGICAL_SECTOR, 4, "sector size")?, 0).unwrap_or(0));

        if !block_bytes.is_power_of_two() || !(MIB..=256 * MIB).contains(&block_bytes) {
            return Err(format!("its block size, {block_bytes} bytes, is outside the format"));
        }
        if sector_bytes != 512 && sector_bytes != 4096 {
            return Err(format!("its sector size, {sector_bytes} bytes, is outside the format"));
        }
        if capacity == 0 || capacity > 64 * 1024 * 1024 * MIB {
            return Err("its disk size is outside the format".to_owned());
        }

        // A block of sector bitmap follows every `chunk` blocks of the disk in
        // the table, whether or not the file is a differencing one.
        let chunk = (1_u64 << 23) * sector_bytes / block_bytes;
        let count = capacity.div_ceil(block_bytes);
        let entries = count + (count - 1) / chunk;
        if entries.saturating_mul(8) > bat_bytes {
            return Err("its block table is shorter than its disk".to_owned());
        }
        let table = read_region(&mut file, bat_at, entries * 8, "block table")?;
        let blocks = (0..count)
            .map(|block| {
                let at = usize::try_from((block + block / chunk) * 8).expect("the table was read whole");
                u64_at(&table, at).unwrap_or(0)
            })
            .collect();

        Ok(Self {
            file,
            capacity,
            block_bytes,
            sector_bytes,
            differencing,
            pending,
            overhead: MIB + log_bytes + metadata_bytes + bat_bytes,
            blocks,
        })
    }

    /// What the file holds of the disk: every block it has room for, whole.
    #[must_use]
    pub fn written(&self) -> u64 {
        self.blocks.iter().filter(|&&entry| held(entry).is_some()).count() as u64 * self.block_bytes
    }

    /// Reads the disk's bytes at `at`. A block the file does not hold reads
    /// as zeroes, as it does to the machine inside — except on a differencing
    /// disk, whose missing blocks are another disk's, and which is refused.
    ///
    /// # Errors
    ///
    /// Past the end of the disk, on a differencing disk, or when the file
    /// cannot be read.
    pub fn read_at(&mut self, mut at: u64, mut into: &mut [u8]) -> std::io::Result<()> {
        if self.differencing {
            return Err(std::io::Error::other("a checkpoint's blocks are partly in the disk it was taken from"));
        }
        if at.checked_add(into.len() as u64).is_none_or(|end| end > self.capacity) {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "past the end of the disk"));
        }
        while !into.is_empty() {
            let block = at / self.block_bytes;
            let within = at % self.block_bytes;
            let length = into.len().min(usize::try_from(self.block_bytes - within).unwrap_or(usize::MAX));
            let (piece, rest) = into.split_at_mut(length);
            let entry = self.blocks[usize::try_from(block).expect("a block of the disk")];
            match held(entry) {
                Some(offset) => read_at(&mut self.file, offset + within, piece)?,
                None => piece.fill(0),
            }
            at += length as u64;
            into = rest;
        }
        Ok(())
    }
}

/// Where the file holds the block of `entry`, when it does.
fn held(entry: u64) -> Option<u64> {
    matches!(entry & 7, FULLY_PRESENT | PARTIALLY_PRESENT).then_some((entry >> 20) << 20)
}

fn read_at<R: Read + Seek>(file: &mut R, at: u64, into: &mut [u8]) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(at))?;
    file.read_exact(into)
}

/// A region the size the table gives it, refused past a size no real file
/// has: a damaged table must not make the reader allocate gigabytes.
fn read_region<R: Read + Seek>(file: &mut R, at: u64, bytes: u64, what: &str) -> Result<Vec<u8>, String> {
    if bytes > 256 * MIB {
        return Err(format!("its {what} is larger than the format allows"));
    }
    let mut region = vec![0_u8; usize::try_from(bytes).map_err(|_| format!("its {what} is too large"))?];
    read_at(file, at, &mut region).map_err(|error| format!("its {what} could not be read: {error}"))?;
    Ok(region)
}

/// The region table at `at`, when it checks out.
fn region_table<R: Read + Seek>(file: &mut R, at: u64) -> Option<Vec<Region>> {
    let mut bytes = vec![0_u8; REGION_BYTES];
    read_at(file, at, &mut bytes).ok()?;
    if !bytes.starts_with(b"regi") || !checksummed(&bytes) {
        return None;
    }
    let count = usize::try_from(u32_at(&bytes, 8)?).ok()?;
    if count > 2047 {
        return None;
    }
    (0..count)
        .map(|entry| {
            let at = 16 + entry * 32;
            Some(Region {
                id: bytes.get(at..at + 16)?.try_into().ok()?,
                at: u64_at(&bytes, at + 16)?,
                bytes: u64::from(u32_at(&bytes, at + 24)?),
                required: u32_at(&bytes, at + 28)? & 1 != 0,
            })
        })
        .collect()
}

/// The items of a metadata region: id, where its value is in the region, and
/// how long it is.
fn metadata_items(region: &[u8]) -> Result<Vec<([u8; 16], usize, usize)>, String> {
    if !region.starts_with(b"metadata") {
        return Err("its metadata does not say so".to_owned());
    }
    let count = usize::from(u16_at(region, 10).unwrap_or(0));
    if count > 2047 {
        return Err("its metadata lists more items than the format allows".to_owned());
    }
    let mut items = Vec::with_capacity(count);
    for entry in 0..count {
        let at = 32 + entry * 32;
        let read = || -> Option<([u8; 16], usize, usize, bool)> {
            Some((
                region.get(at..at + 16)?.try_into().ok()?,
                usize::try_from(u32_at(region, at + 16)?).ok()?,
                usize::try_from(u32_at(region, at + 20)?).ok()?,
                u32_at(region, at + 24)? & 4 != 0,
            ))
        };
        let (id, offset, length, required) = read().ok_or("its metadata table is cut short")?;
        if required && !KNOWN_ITEMS.contains(&id) {
            return Err("it requires a metadata item this reader does not know".to_owned());
        }
        items.push((id, offset, length));
    }
    Ok(items)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use super::*;

    /// A VHDX file written the way Hyper-V lays one out: headers, region
    /// tables, metadata at 1 MiB, the block table at 2 MiB, blocks after.
    pub(crate) struct Image {
        pub capacity: u64,
        pub block_bytes: u64,
        pub sector_bytes: u64,
        pub differencing: bool,
        pub pending: bool,
        /// The disk's bytes, by offset; blocks no write touches are not held.
        pub writes: Vec<(u64, Vec<u8>)>,
        /// Blocks held but let go of: in the table as unmapped, still in the
        /// file.
        pub unmapped: Vec<u64>,
    }

    impl Image {
        pub(crate) fn new(capacity: u64) -> Self {
            Self {
                capacity,
                block_bytes: MIB,
                sector_bytes: 512,
                differencing: false,
                pending: false,
                writes: Vec::new(),
                unmapped: Vec::new(),
            }
        }

        pub(crate) fn write(mut self, at: u64, bytes: &[u8]) -> Self {
            self.writes.push((at, bytes.to_vec()));
            self
        }

        fn header(&self, sequence: u64) -> Vec<u8> {
            let mut header = vec![0_u8; HEADER_BYTES];
            header[0..4].copy_from_slice(b"head");
            header[8..16].copy_from_slice(&sequence.to_le_bytes());
            if self.pending {
                header[48] = 1;
            }
            header[66..68].copy_from_slice(&1_u16.to_le_bytes());
            header[68..72].copy_from_slice(&u32::try_from(MIB).unwrap().to_le_bytes());
            header[72..80].copy_from_slice(&MIB.to_le_bytes());
            let crc = crc32c(&header);
            header[4..8].copy_from_slice(&crc.to_le_bytes());
            header
        }

        fn regions(bat_bytes: u64) -> Vec<u8> {
            let mut table = vec![0_u8; REGION_BYTES];
            table[0..4].copy_from_slice(b"regi");
            table[8..12].copy_from_slice(&2_u32.to_le_bytes());
            for (entry, (id, at, bytes)) in [(METADATA, MIB, MIB), (BAT, 2 * MIB, bat_bytes)].into_iter().enumerate() {
                let at_entry = 16 + entry * 32;
                table[at_entry..at_entry + 16].copy_from_slice(&id);
                table[at_entry + 16..at_entry + 24].copy_from_slice(&at.to_le_bytes());
                table[at_entry + 24..at_entry + 28].copy_from_slice(&u32::try_from(bytes).unwrap().to_le_bytes());
                table[at_entry + 28..at_entry + 32].copy_from_slice(&1_u32.to_le_bytes());
            }
            let crc = crc32c(&table);
            table[4..8].copy_from_slice(&crc.to_le_bytes());
            table
        }

        fn metadata(&self) -> Vec<u8> {
            let mut region = vec![0_u8; usize::try_from(MIB).unwrap()];
            region[0..8].copy_from_slice(b"metadata");
            let mut flags = 0_u32.to_le_bytes();
            if self.differencing {
                flags = 2_u32.to_le_bytes();
            }
            let parameters = [u32::try_from(self.block_bytes).unwrap().to_le_bytes(), flags].concat();
            let items: [([u8; 16], Vec<u8>); 3] = [
                (FILE_PARAMETERS, parameters),
                (VIRTUAL_DISK_SIZE, self.capacity.to_le_bytes().to_vec()),
                (LOGICAL_SECTOR, u32::try_from(self.sector_bytes).unwrap().to_le_bytes().to_vec()),
            ];
            region[10..12].copy_from_slice(&u16::try_from(items.len()).unwrap().to_le_bytes());
            let mut value_at = 64 * 1024;
            for (entry, (id, value)) in items.iter().enumerate() {
                let at = 32 + entry * 32;
                region[at..at + 16].copy_from_slice(id);
                region[at + 16..at + 20].copy_from_slice(&u32::try_from(value_at).unwrap().to_le_bytes());
                region[at + 20..at + 24].copy_from_slice(&u32::try_from(value.len()).unwrap().to_le_bytes());
                region[at + 24..at + 28].copy_from_slice(&6_u32.to_le_bytes());
                region[value_at..value_at + value.len()].copy_from_slice(value);
                value_at += 4096;
            }
            region
        }

        /// The file's bytes.
        pub(crate) fn bytes(&self) -> Vec<u8> {
            let chunk = (1_u64 << 23) * self.sector_bytes / self.block_bytes;
            let count = self.capacity.div_ceil(self.block_bytes);
            let entries = count + (count - 1) / chunk;
            let bat_bytes = (entries * 8).div_ceil(MIB) * MIB;
            let first_block = 2 * MIB + bat_bytes;

            let mut held: Vec<u64> = self
                .writes
                .iter()
                .flat_map(|(at, bytes)| (at / self.block_bytes)..=((at + bytes.len() as u64 - 1) / self.block_bytes))
                .chain(self.unmapped.iter().copied())
                .collect();
            held.sort_unstable();
            held.dedup();

            let mut file = vec![0_u8; usize::try_from(first_block + held.len() as u64 * self.block_bytes).unwrap()];
            file[0..8].copy_from_slice(SIGNATURE);
            file[64 * 1024..68 * 1024].copy_from_slice(&self.header(1));
            file[128 * 1024..132 * 1024].copy_from_slice(&self.header(2));
            let regions = Self::regions(bat_bytes);
            file[192 * 1024..256 * 1024].copy_from_slice(&regions);
            file[256 * 1024..320 * 1024].copy_from_slice(&regions);
            file[1024 * 1024..2048 * 1024].copy_from_slice(&self.metadata());
            for (slot, &block) in held.iter().enumerate() {
                let offset = first_block + slot as u64 * self.block_bytes;
                let state = if self.unmapped.contains(&block) { 3 } else { FULLY_PRESENT };
                let entry = ((offset >> 20) << 20) | state;
                let at = usize::try_from(2 * MIB + (block + block / chunk) * 8).unwrap();
                file[at..at + 8].copy_from_slice(&entry.to_le_bytes());
            }
            for (at, bytes) in &self.writes {
                for (cursor, &byte) in (*at..).zip(bytes) {
                    let block = cursor / self.block_bytes;
                    let slot = held.iter().position(|&held| held == block).unwrap() as u64;
                    let offset = first_block + slot * self.block_bytes + cursor % self.block_bytes;
                    file[usize::try_from(offset).unwrap()] = byte;
                }
            }
            file
        }

        pub(crate) fn open(&self) -> Vhdx<Cursor<Vec<u8>>> {
            Vhdx::read(Cursor::new(self.bytes())).expect("the image reads")
        }
    }

    #[test]
    fn the_checksum_is_castagnolis() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(b""), 0);
    }

    #[test]
    fn a_guid_is_written_with_its_first_fields_little_endian() {
        assert_eq!(BAT[..4], [0x66, 0x77, 0xC2, 0x2D]);
        assert_eq!(BAT[4..8], [0x23, 0xF6, 0x00, 0x42]);
        assert_eq!(BAT[8..], [0x9D, 0x64, 0x11, 0x5E, 0x9B, 0xFD, 0x4A, 0x08]);
    }

    #[test]
    fn the_disk_reads_as_written_and_what_it_never_wrote_reads_as_zeroes() {
        let mut disk = Image::new(8 * MIB).write(5, b"hello").write(3 * MIB - 2, b"across").open();
        assert_eq!((disk.capacity, disk.block_bytes, disk.sector_bytes), (8 * MIB, MIB, 512));
        let mut bytes = [0_u8; 7];
        disk.read_at(4, &mut bytes).expect("reads");
        assert_eq!(&bytes, b"\0hello\0");
        let mut across = [0_u8; 6];
        disk.read_at(3 * MIB - 2, &mut across).expect("reads across two blocks");
        assert_eq!(&across, b"across");
        let mut never = [9_u8; 4];
        disk.read_at(6 * MIB, &mut never).expect("reads");
        assert_eq!(never, [0; 4]);
        assert!(disk.read_at(8 * MIB - 2, &mut [0; 4]).is_err(), "past the end");
    }

    #[test]
    fn what_the_file_holds_is_the_blocks_in_its_table_and_not_those_let_go_of() {
        let mut image = Image::new(16 * MIB).write(0, b"a").write(5 * MIB, b"b");
        image.unmapped = vec![9, 10];
        let disk = Vhdx::read(Cursor::new(image.bytes())).expect("reads");
        assert_eq!(disk.written(), 2 * MIB, "two blocks held; the two unmapped ones are room in the file");
        assert_eq!(disk.overhead, MIB + MIB + MIB + MIB, "headers, log, metadata and a table of one mebibyte");
        assert!(!disk.pending && !disk.differencing);
    }

    #[test]
    fn the_table_skips_a_sector_bitmap_entry_after_every_chunk() {
        // 1 MiB blocks of 512-byte sectors: a bitmap entry after every 4096
        // blocks. A block past the first chunk is read from the next entry
        // but one.
        let disk_bytes = 4100 * MIB;
        let mut disk = Image::new(disk_bytes).write(4097 * MIB + 10, b"far").open();
        let mut bytes = [0_u8; 3];
        disk.read_at(4097 * MIB + 10, &mut bytes).expect("reads");
        assert_eq!(&bytes, b"far");
        assert_eq!(disk.written(), MIB);
    }

    #[test]
    fn the_header_written_last_is_the_one_read() {
        let mut image = Image::new(4 * MIB);
        image.pending = true;
        let mut bytes = image.bytes();
        // The older copy says nothing is pending; the newer one says the log
        // has writes in it.
        image.pending = false;
        let older = image.header(1);
        bytes[64 * 1024..68 * 1024].copy_from_slice(&older);
        assert!(Vhdx::read(Cursor::new(bytes.clone())).expect("reads").pending);
        // A torn newer copy leaves the older one.
        bytes[128 * 1024 + 100] ^= 0xFF;
        assert!(!Vhdx::read(Cursor::new(bytes)).expect("reads").pending);
    }

    #[test]
    fn a_file_that_is_not_one_or_is_damaged_is_refused_by_name() {
        assert!(Vhdx::read(Cursor::new(vec![0_u8; 512 * 1024])).expect_err("refused").contains("not a VHDX"));
        let good = Image::new(4 * MIB).bytes();
        let mut headless = good.clone();
        headless[64 * 1024] = 0;
        headless[128 * 1024] = 0;
        assert!(Vhdx::read(Cursor::new(headless)).expect_err("refused").contains("header"));
        let mut tableless = good.clone();
        tableless[192 * 1024 + 20] ^= 1;
        tableless[256 * 1024 + 20] ^= 1;
        assert!(Vhdx::read(Cursor::new(tableless)).expect_err("refused").contains("region table"));
        let mut odd = Image::new(4 * MIB);
        odd.block_bytes = 3 * MIB;
        assert!(Vhdx::read(Cursor::new(odd.bytes())).expect_err("refused").contains("block size"));
    }

    #[test]
    fn a_checkpoint_says_so_and_is_not_read_as_a_disk() {
        let mut image = Image::new(4 * MIB).write(0, b"x");
        image.differencing = true;
        let mut disk = Vhdx::read(Cursor::new(image.bytes())).expect("reads");
        assert!(disk.differencing);
        assert!(disk.read_at(0, &mut [0; 1]).is_err(), "its missing blocks are another disk's");
    }
}
