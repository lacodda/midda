//! The NTFS on-disk format, as far as the table reader needs it, read from
//! bytes.
//!
//! The reader needs six things from the format: where the table starts and how
//! big its records are (the boot sector), which records are live files and
//! which extend another (the record header), the attributes inside a record,
//! the names and parents in `$FILE_NAME`, the write time and attribute bits in
//! `$STANDARD_INFORMATION`, and both sizes of a `$DATA` stream — including,
//! for a compressed or sparse stream, the clusters it really holds, which is
//! what `AllocationSize` reports and what midda shows.
//!
//! That last number is why this is midda's own code. `ntfs-reader` read the
//! format until 0.4; from 0.5 its attribute headers are private and a stream
//! is reported by its logical size alone, so the number the product exists to
//! show is no longer reachable through it. The structures read here have not
//! changed since NTFS 3.1 (Windows XP), and every field is read by offset out
//! of a byte slice and bounds-checked — nothing is cast to a packed struct.
//!
//! Pure: bytes in, numbers out, no volume. Built and tested on every platform
//! with records written by the tests.

#![cfg_attr(not(windows), allow(dead_code, reason = "only the volume reader, which is Windows-only, reads records"))]

/// `$STANDARD_INFORMATION`.
pub const STANDARD_INFORMATION: u32 = 0x10;
/// `$FILE_NAME`.
pub const FILE_NAME: u32 = 0x30;
/// `$DATA`.
pub const DATA: u32 = 0x80;
/// `$REPARSE_POINT`.
pub const REPARSE_POINT: u32 = 0xC0;
/// The type that ends a record's attributes.
const END: u32 = 0xFFFF_FFFF;

/// Record header flag: the record describes a live file.
pub const IN_USE: u16 = 0x0001;
/// Record header flag: the file is a directory.
pub const IS_DIRECTORY: u16 = 0x0002;

/// Attribute header flag: the stream is compressed.
const ATTRIBUTE_COMPRESSED: u16 = 0x0001;
/// Attribute header flag: the stream is sparse.
const ATTRIBUTE_SPARSE: u16 = 0x8000;

/// `FILE_ATTRIBUTE_REPARSE_POINT`, as a name's copy of the attribute bits
/// carries it.
const REPARSE_POINT_BIT: u32 = 0x0400;

/// The namespace of a DOS 8.3 short name generated beside a long one.
const DOS_NAMESPACE: u8 = 2;

/// The stride of the update sequence array: NTFS protects each 512-byte stretch
/// of a record, whatever the disk's own sector size.
const PROTECTED_STRETCH: usize = 512;

/// Reads `N` bytes at `at`, when they are there.
fn bytes<const N: usize>(slice: &[u8], at: usize) -> Option<[u8; N]> {
    slice.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn u8_at(slice: &[u8], at: usize) -> Option<u8> {
    slice.get(at).copied()
}

fn u16_at(slice: &[u8], at: usize) -> Option<u16> {
    bytes(slice, at).map(u16::from_le_bytes)
}

fn u32_at(slice: &[u8], at: usize) -> Option<u32> {
    bytes(slice, at).map(u32::from_le_bytes)
}

fn u64_at(slice: &[u8], at: usize) -> Option<u64> {
    bytes(slice, at).map(u64::from_le_bytes)
}

/// UTF-16 code units, little-endian, as a string.
///
/// Lossy on purpose: the walk reads a name through `to_string_lossy`, so an
/// unpaired surrogate becomes U+FFFD on both sides and the two scanners still
/// agree about the name.
fn utf16(slice: &[u8]) -> String {
    let units: Vec<u16> = slice.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
}

/// Where a volume keeps its table, and the sizes it is read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// The volume's logical sector: the unit every raw read is aligned to.
    pub sector_bytes: u64,
    pub cluster_bytes: u64,
    /// The size of one table record: 1 KiB on nearly every volume, 4 KiB on
    /// some formatted for 4K-native disks.
    pub record_bytes: u64,
    /// Where the table's first record is, in bytes from the start of the
    /// volume.
    pub table_at: u64,
}

/// Reads the boot sector.
///
/// `None` for anything that is not a plausible NTFS boot sector: another
/// filesystem, a sector size that is not a power of two, a record smaller than
/// one protected stretch. A volume read with a wrong geometry would produce a
/// tree of garbage rather than an error, so the checks are strict.
#[must_use]
pub fn geometry(boot: &[u8]) -> Option<Geometry> {
    if boot.get(3..11)? != b"NTFS    " {
        return None;
    }
    let sector_bytes = u64::from(u16_at(boot, 0x0B)?);
    if !sector_bytes.is_power_of_two() || !(256..=4096).contains(&sector_bytes) {
        return None;
    }
    // Clusters above 64 KiB (allowed since Windows 10 1709) do not fit the
    // byte as a count of sectors; the high values are a negative shift.
    let per_cluster = u8_at(boot, 0x0D)?;
    let sectors_per_cluster = if per_cluster <= 0x80 {
        u64::from(per_cluster)
    } else {
        1_u64.checked_shl(256 - u32::from(per_cluster))?
    };
    let cluster_bytes = sector_bytes.checked_mul(sectors_per_cluster)?;
    if cluster_bytes == 0 {
        return None;
    }
    let table_cluster = u64_at(boot, 0x30)?;
    // Positive: clusters per record. Negative: the record is 2^-n bytes, which
    // is how a record smaller than a cluster is written.
    let per_record = i8::from_le_bytes(bytes(boot, 0x40)?);
    let record_bytes = if per_record > 0 {
        cluster_bytes.checked_mul(u64::try_from(per_record).ok()?)?
    } else {
        1_u64.checked_shl(u32::from(per_record.unsigned_abs()))?
    };
    if !record_bytes.is_power_of_two() || !(512..=65_536).contains(&record_bytes) {
        return None;
    }
    Some(Geometry {
        sector_bytes,
        cluster_bytes,
        record_bytes,
        table_at: table_cluster.checked_mul(cluster_bytes)?,
    })
}

/// Undoes NTFS's torn-write protection on one record, in place.
///
/// The last two bytes of every 512-byte stretch of a record are replaced on
/// disk by a check value, and the real bytes kept in an array near the start.
/// A stretch whose check value does not match was not written whole; the
/// record is then not to be trusted, and is skipped rather than half-read.
pub fn fix_up(record: &mut [u8]) -> bool {
    let (Some(offset), Some(count)) = (u16_at(record, 4), u16_at(record, 6)) else {
        return false;
    };
    let (offset, count) = (usize::from(offset), usize::from(count));
    if count == 0 || offset + count * 2 > record.len() {
        return false;
    }
    let check = [record[offset], record[offset + 1]];
    for stretch in 1..count {
        let end = stretch * PROTECTED_STRETCH;
        if end > record.len() {
            return false;
        }
        if record[end - 2..end] != check {
            return false;
        }
        let saved = offset + stretch * 2;
        record[end - 2] = record[saved];
        record[end - 1] = record[saved + 1];
    }
    true
}

/// What a record's header says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Which use of this record number this is.
    pub sequence: u16,
    /// [`IN_USE`] and [`IS_DIRECTORY`].
    pub flags: u16,
    /// The base record's reference, when this record extends another.
    pub base: Option<u64>,
    /// Where the attributes start.
    attributes_at: usize,
    /// How much of the record is in use.
    used: usize,
}

impl Header {
    /// Whether the record describes a live file.
    #[must_use]
    pub const fn in_use(&self) -> bool {
        self.flags & IN_USE != 0
    }

    /// Whether the file is a directory.
    #[must_use]
    pub const fn is_directory(&self) -> bool {
        self.flags & IS_DIRECTORY != 0
    }
}

/// Reads a record's header, after [`fix_up`].
///
/// `None` for a record that does not say `FILE`, or whose header points
/// outside itself — an unused slot of the table, or one torn beyond repair.
#[must_use]
pub fn header(record: &[u8]) -> Option<Header> {
    if record.get(0..4)? != b"FILE" {
        return None;
    }
    let attributes_at = usize::from(u16_at(record, 0x14)?);
    let used = usize::try_from(u32_at(record, 0x18)?).ok()?;
    // The header itself ends at 0x2A; attributes cannot start inside it.
    if used > record.len() || attributes_at < 0x2A || attributes_at >= used {
        return None;
    }
    let base = u64_at(record, 0x20)?;
    Some(Header {
        sequence: u16_at(record, 0x10)?,
        flags: u16_at(record, 0x16)?,
        base: (base != 0).then_some(base),
        attributes_at,
        used,
    })
}

/// The attributes of a record, in the order they are stored.
///
/// Stops at the end marker, and at the first attribute whose length is not
/// plausible — what follows a broken length cannot be found.
pub fn attributes<'a>(record: &'a [u8], header: &Header) -> impl Iterator<Item = Attribute<'a>> + use<'a> {
    let used = &record[..header.used];
    let mut at = header.attributes_at;
    std::iter::from_fn(move || {
        if u32_at(used, at)? == END {
            return None;
        }
        let length = usize::try_from(u32_at(used, at + 4)?).ok()?;
        // Sixteen bytes is the common header every attribute has.
        if length < 16 || at + length > used.len() {
            return None;
        }
        let attribute = Attribute { bytes: &used[at..at + length] };
        at += length;
        Some(attribute)
    })
}

/// One attribute of a record: its header, and its value or the map of where
/// the value is.
#[derive(Debug, Clone, Copy)]
pub struct Attribute<'a> {
    bytes: &'a [u8],
}

impl<'a> Attribute<'a> {
    /// The attribute's type: [`DATA`], [`FILE_NAME`], …
    #[must_use]
    pub fn kind(&self) -> u32 {
        u32_at(self.bytes, 0).unwrap_or(END)
    }

    /// Whether the value is in the record (`true`) or in clusters elsewhere.
    #[must_use]
    pub fn is_resident(&self) -> bool {
        u8_at(self.bytes, 8) == Some(0)
    }

    /// The attribute's own name — a named stream's — when it has one.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        let units = usize::from(u8_at(self.bytes, 9)?);
        if units == 0 {
            return None;
        }
        let at = usize::from(u16_at(self.bytes, 10)?);
        Some(utf16(self.bytes.get(at..at + units * 2)?))
    }

    fn flags(&self) -> u16 {
        u16_at(self.bytes, 12).unwrap_or(0)
    }

    /// The value of a resident attribute, when it lies inside the attribute.
    #[must_use]
    pub fn value(&self) -> Option<&'a [u8]> {
        if !self.is_resident() {
            return None;
        }
        let length = usize::try_from(u32_at(self.bytes, 0x10)?).ok()?;
        let at = usize::from(u16_at(self.bytes, 0x14)?);
        self.bytes.get(at..at.checked_add(length)?)
    }

    /// The length of a resident value, whether or not the value itself is
    /// readable.
    fn resident_length(&self) -> Option<u64> {
        self.is_resident().then(|| u32_at(self.bytes, 0x10).map(u64::from))?
    }

    /// The first cluster of the stream this piece of a non-resident value
    /// covers. Only the piece starting at zero carries the stream's sizes.
    fn lowest_vcn(&self) -> Option<u64> {
        if self.is_resident() {
            return None;
        }
        u64_at(self.bytes, 0x10)
    }

    /// Both sizes of a `$DATA` stream: what it reads as, and what it occupies.
    ///
    /// A resident stream occupies its length rounded to the eight-byte
    /// alignment of an attribute, inside the record — measured through
    /// `AllocationSize`: 100 bytes occupy 104 (see `crate::platform`). A
    /// non-resident one occupies its allocated clusters, except when it is
    /// compressed or sparse: then the span it covers is in the usual field and
    /// the clusters it really holds are in the one after, which is what
    /// `AllocationSize` reports. Measured: 45 056 for a 360 000-byte compressed
    /// file.
    ///
    /// `None` for a later piece of a stream split across records: only the
    /// piece that starts at the beginning carries the sizes.
    #[must_use]
    pub fn stream_size(&self) -> Option<crate::size::Size> {
        if let Some(length) = self.resident_length() {
            return Some(super::assemble::resident_size(length));
        }
        if self.lowest_vcn()? != 0 {
            return None;
        }
        let spread = u64_at(self.bytes, 0x28)?;
        let logical = u64_at(self.bytes, 0x30)?;
        let allocated = if self.flags() & (ATTRIBUTE_COMPRESSED | ATTRIBUTE_SPARSE) == 0 {
            spread
        } else {
            u64_at(self.bytes, 0x40).unwrap_or(spread)
        };
        Some(crate::size::Size { logical, allocated })
    }

    /// `$STANDARD_INFORMATION`: the write time and the attribute bits.
    #[must_use]
    pub fn standard_information(&self) -> Option<StandardInformation> {
        if self.kind() != STANDARD_INFORMATION {
            return None;
        }
        let value = self.value()?;
        Some(StandardInformation {
            modified: u64_at(value, 0x08)?,
            attributes: u32_at(value, 0x20)?,
        })
    }

    /// `$FILE_NAME`: one name, and the directory it is in.
    #[must_use]
    pub fn file_name(&self) -> Option<FileName> {
        if self.kind() != FILE_NAME {
            return None;
        }
        let value = self.value()?;
        let units = usize::from(u8_at(value, 0x40)?);
        Some(FileName {
            parent: u64_at(value, 0)?,
            attributes: u32_at(value, 0x38)?,
            reparse_tag: u32_at(value, 0x3C)?,
            namespace: u8_at(value, 0x41)?,
            name: utf16(value.get(0x42..0x42 + units * 2)?),
        })
    }

    /// The tag of a `$REPARSE_POINT`: what kind of reparse point the file is.
    #[must_use]
    pub fn reparse_tag(&self) -> Option<u32> {
        if self.kind() != REPARSE_POINT {
            return None;
        }
        u32_at(self.value()?, 0)
    }

    /// Where a non-resident value lives on the volume, as byte ranges, and how
    /// long the value is.
    ///
    /// # Errors
    ///
    /// A description of what is wrong with the map: a resident attribute, a
    /// run that runs off its own bytes, an offset before the volume, runs that
    /// do not cover the declared length — which is what a table whose own map
    /// continues in another record looks like from here.
    pub fn runs(&self, cluster_bytes: u64) -> Result<(u64, Vec<Run>), &'static str> {
        if self.is_resident() {
            return Err("the attribute is resident");
        }
        let length = u64_at(self.bytes, 0x30).ok_or("the attribute is too short")?;
        let mut runs = Vec::new();
        if length == 0 {
            return Ok((0, runs));
        }
        let at = usize::from(u16_at(self.bytes, 0x20).ok_or("the attribute is too short")?);
        let map = self.bytes.get(at..).ok_or("the map is outside the attribute")?;

        let mut cursor = 0_usize;
        let mut cluster = 0_i128;
        let mut covered = 0_u64;
        loop {
            let descriptor = *map.get(cursor).ok_or("the map has no end")?;
            if descriptor == 0 {
                break;
            }
            let count_bytes = usize::from(descriptor & 0x0F);
            let offset_bytes = usize::from(descriptor >> 4);
            if count_bytes == 0 || count_bytes > 8 || offset_bytes > 8 {
                return Err("a run has an impossible header");
            }
            cursor += 1;

            let mut count = [0_u8; 8];
            count[..count_bytes].copy_from_slice(map.get(cursor..cursor + count_bytes).ok_or("a run ends early")?);
            let clusters = u64::from_le_bytes(count);
            if clusters == 0 {
                return Err("a run is empty");
            }
            cursor += count_bytes;
            let run_bytes = clusters.checked_mul(cluster_bytes).ok_or("a run is too long")?;
            covered = covered.checked_add(run_bytes).ok_or("the runs are too long")?;

            if offset_bytes == 0 {
                runs.push(Run::Hole { bytes: run_bytes });
                continue;
            }
            // The offset is signed and relative to the previous run's start,
            // stored in as few bytes as it needs: sign-extend from the top.
            let mut offset = [0_u8; 8];
            offset[..offset_bytes].copy_from_slice(map.get(cursor..cursor + offset_bytes).ok_or("a run ends early")?);
            let empty = (8 - offset_bytes) * 8;
            let delta = (i64::from_le_bytes(offset) << empty) >> empty;
            cursor += offset_bytes;

            cluster += i128::from(delta);
            let start = u64::try_from(cluster).map_err(|_| "a run starts before the volume")?;
            runs.push(Run::Data {
                at: start.checked_mul(cluster_bytes).ok_or("a run starts past any volume")?,
                bytes: run_bytes,
            });
        }

        if covered < length {
            return Err("the runs are shorter than the value");
        }
        Ok((length, runs))
    }
}

/// One stretch of a non-resident value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Run {
    /// Bytes on the volume: where they start and how many.
    Data { at: u64, bytes: u64 },
    /// A stretch that reads as zeroes and occupies nothing.
    Hole { bytes: u64 },
}

/// What `$STANDARD_INFORMATION` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardInformation {
    /// When the file was last written, as a `FILETIME`.
    pub modified: u64,
    /// The `FILE_ATTRIBUTE_*` bits.
    pub attributes: u32,
}

/// What one `$FILE_NAME` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileName {
    /// The directory this name is in, as a full file reference.
    pub parent: u64,
    /// The name's copy of the attribute bits.
    attributes: u32,
    /// The name's copy of the reparse tag, meaningful when the bits say the
    /// file is a reparse point.
    reparse_tag: u32,
    namespace: u8,
    pub name: String,
}

impl FileName {
    /// The reparse tag this name carries, when the file is a reparse point.
    ///
    /// A copy: it stands in only until `$REPARSE_POINT` itself is read.
    #[must_use]
    pub const fn reparse_tag(&self) -> Option<u32> {
        if self.attributes & REPARSE_POINT_BIT != 0 {
            Some(self.reparse_tag)
        } else {
            None
        }
    }

    /// Whether this is the DOS 8.3 alias NTFS generates beside a long name —
    /// not a name of its own, and never listed by the walk.
    #[must_use]
    pub const fn is_dos_alias(&self) -> bool {
        self.namespace == DOS_NAMESPACE
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::size::Size;

    /// Builds a boot sector with the given geometry fields.
    fn boot(sector: u16, per_cluster: u8, table_cluster: u64, per_record: i8) -> Vec<u8> {
        let mut boot = vec![0_u8; 512];
        boot[3..11].copy_from_slice(b"NTFS    ");
        boot[0x0B..0x0D].copy_from_slice(&sector.to_le_bytes());
        boot[0x0D] = per_cluster;
        boot[0x30..0x38].copy_from_slice(&table_cluster.to_le_bytes());
        boot[0x40] = per_record.to_le_bytes()[0];
        boot
    }

    #[test]
    fn a_boot_sector_says_where_the_table_is() {
        let geometry = geometry(&boot(512, 8, 786_432, -10)).expect("a plausible boot sector");
        assert_eq!(
            geometry,
            Geometry {
                sector_bytes: 512,
                cluster_bytes: 4096,
                record_bytes: 1024,
                table_at: 786_432 * 4096,
            }
        );
    }

    #[test]
    fn a_large_cluster_and_a_record_of_whole_clusters_are_read() {
        // 0xF8 is -8: 2^8 sectors of 512 bytes, a 128 KiB cluster.
        let large = geometry(&boot(512, 0xF8, 4, -12)).expect("a large-cluster volume");
        assert_eq!(large.cluster_bytes, 128 << 10);
        assert_eq!(large.record_bytes, 4096);

        // A positive count is clusters per record.
        let whole = geometry(&boot(512, 1, 4, 2)).expect("records of two clusters");
        assert_eq!(whole.record_bytes, 1024);
    }

    #[test]
    fn what_is_not_an_ntfs_boot_sector_has_no_geometry() {
        let mut fat = boot(512, 8, 4, -10);
        fat[3..11].copy_from_slice(b"EXFAT   ");
        assert_eq!(geometry(&fat), None);
        assert_eq!(geometry(&boot(500, 8, 4, -10)), None, "a sector that is not a power of two");
        assert_eq!(geometry(&boot(512, 0, 4, -10)), None, "a cluster of no sectors");
        assert_eq!(geometry(&boot(512, 8, 4, -8)), None, "a record smaller than one protected stretch");
        assert_eq!(geometry(&[0_u8; 16]), None, "a short read");
    }

    /// A 1 KiB record with an update sequence array at 0x30 of three entries:
    /// the check value and the saved last two bytes of each 512-byte stretch.
    fn protected(check: [u8; 2], saved: [[u8; 2]; 2]) -> Vec<u8> {
        let mut record = vec![0_u8; 1024];
        record[4..6].copy_from_slice(&0x30_u16.to_le_bytes());
        record[6..8].copy_from_slice(&3_u16.to_le_bytes());
        record[0x30..0x32].copy_from_slice(&check);
        record[0x32..0x34].copy_from_slice(&saved[0]);
        record[0x34..0x36].copy_from_slice(&saved[1]);
        record[510..512].copy_from_slice(&check);
        record[1022..1024].copy_from_slice(&check);
        record
    }

    #[test]
    fn a_protected_record_gets_its_real_bytes_back() {
        let mut record = protected([0x07, 0x00], [[0xAA, 0xBB], [0xCC, 0xDD]]);
        assert!(fix_up(&mut record));
        assert_eq!(&record[510..512], &[0xAA, 0xBB]);
        assert_eq!(&record[1022..1024], &[0xCC, 0xDD]);
    }

    #[test]
    fn a_torn_record_is_refused_rather_than_half_read() {
        let mut record = protected([0x07, 0x00], [[0xAA, 0xBB], [0xCC, 0xDD]]);
        // The second stretch was not written: its check value is stale.
        record[1022..1024].copy_from_slice(&[0x06, 0x00]);
        assert!(!fix_up(&mut record));
    }

    #[test]
    fn a_record_with_no_protection_array_is_refused() {
        let mut record = vec![0_u8; 1024];
        assert!(!fix_up(&mut record));
        let mut short = vec![0_u8; 6];
        assert!(!fix_up(&mut short));
    }

    /// Writes records the way NTFS lays them out, for the parser to read back.
    pub(crate) struct Writer {
        record: Vec<u8>,
        at: usize,
    }

    impl Writer {
        /// A record header: `FILE`, the sequence, the flags, the base reference.
        pub(crate) fn new(sequence: u16, flags: u16, base: u64) -> Self {
            let mut record = vec![0_u8; 1024];
            record[0..4].copy_from_slice(b"FILE");
            record[0x10..0x12].copy_from_slice(&sequence.to_le_bytes());
            record[0x14..0x16].copy_from_slice(&0x38_u16.to_le_bytes());
            record[0x16..0x18].copy_from_slice(&flags.to_le_bytes());
            record[0x20..0x28].copy_from_slice(&base.to_le_bytes());
            Self { record, at: 0x38 }
        }

        fn attribute(&mut self, kind: u32, resident: bool, name: &str, flags: u16, body: &[u8]) -> &mut Self {
            let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let header = if resident { 0x18 } else { 0x40 };
            let name_at = header;
            let body_at = (name_at + name.len()).next_multiple_of(8);
            let length = (body_at + body.len()).next_multiple_of(8);
            let at = self.at;
            let attribute = &mut self.record[at..at + length];
            attribute[0..4].copy_from_slice(&kind.to_le_bytes());
            attribute[4..8].copy_from_slice(&u32::try_from(length).expect("small").to_le_bytes());
            attribute[8] = u8::from(!resident);
            attribute[9] = u8::try_from(name.len() / 2).expect("short");
            attribute[10..12].copy_from_slice(&u16::try_from(name_at).expect("small").to_le_bytes());
            attribute[12..14].copy_from_slice(&flags.to_le_bytes());
            attribute[name_at..name_at + name.len()].copy_from_slice(&name);
            if resident {
                attribute[0x10..0x14].copy_from_slice(&u32::try_from(body.len()).expect("small").to_le_bytes());
                attribute[0x14..0x16].copy_from_slice(&u16::try_from(body_at).expect("small").to_le_bytes());
                attribute[body_at..body_at + body.len()].copy_from_slice(body);
            } else {
                // `body` is the non-resident header from 0x10 on, then the map.
                attribute[0x10..0x10 + body.len()].copy_from_slice(body);
            }
            self.at += length;
            self
        }

        pub(crate) fn standard_information(&mut self, modified: u64, attributes: u32) -> &mut Self {
            let mut value = vec![0_u8; 0x48];
            value[0x08..0x10].copy_from_slice(&modified.to_le_bytes());
            value[0x20..0x24].copy_from_slice(&attributes.to_le_bytes());
            self.attribute(STANDARD_INFORMATION, true, "", 0, &value)
        }

        pub(crate) fn file_name(&mut self, parent: u64, name: &str, namespace: u8, attributes: u32, tag: u32) -> &mut Self {
            let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let mut value = vec![0_u8; 0x42];
            value[0..8].copy_from_slice(&parent.to_le_bytes());
            value[0x38..0x3C].copy_from_slice(&attributes.to_le_bytes());
            value[0x3C..0x40].copy_from_slice(&tag.to_le_bytes());
            value[0x40] = u8::try_from(units.len() / 2).expect("short");
            value[0x41] = namespace;
            value.extend_from_slice(&units);
            self.attribute(FILE_NAME, true, "", 0, &value)
        }

        pub(crate) fn resident_data(&mut self, name: &str, bytes: usize) -> &mut Self {
            self.attribute(DATA, true, name, 0, &vec![b'x'; bytes])
        }

        /// A non-resident `$DATA`: sizes, flags, and a map.
        pub(crate) fn data(&mut self, name: &str, flags: u16, size: Size, total: u64, map: &[u8]) -> &mut Self {
            let mut body = vec![0_u8; 0x38];
            // lowest VCN 0 at 0x10; the map right after the header.
            let header_end: u16 = 0x48;
            body[0x10..0x12].copy_from_slice(&header_end.to_le_bytes());
            body[0x18..0x20].copy_from_slice(&size.allocated.to_le_bytes());
            body[0x20..0x28].copy_from_slice(&size.logical.to_le_bytes());
            body[0x30..0x38].copy_from_slice(&total.to_le_bytes());
            body.extend_from_slice(map);
            body.push(0);
            self.attribute(DATA, false, name, flags, &body)
        }

        pub(crate) fn reparse_point(&mut self, tag: u32) -> &mut Self {
            let mut value = tag.to_le_bytes().to_vec();
            value.extend_from_slice(&[0; 4]);
            self.attribute(REPARSE_POINT, true, "", 0, &value)
        }

        /// The finished record, with its end marker and used size.
        pub(crate) fn finish(&mut self) -> Vec<u8> {
            let at = self.at;
            self.record[at..at + 4].copy_from_slice(&END.to_le_bytes());
            let used = u32::try_from(at + 8).expect("small");
            self.record[0x18..0x1C].copy_from_slice(&used.to_le_bytes());
            self.record.clone()
        }
    }

    #[test]
    fn a_record_reads_back_as_written() {
        let record = Writer::new(7, IN_USE, 0)
            .standard_information(132_000_000_000_000_000, 0x20)
            .file_name(0x0005_0000_0000_0005, "PROGRA~1", DOS_NAMESPACE, 0, 0)
            .file_name(0x0005_0000_0000_0005, "Program Files", 1, 0, 0)
            .resident_data("", 100)
            .finish();
        let header = header(&record).expect("a valid header");
        assert!(header.in_use() && !header.is_directory());
        assert_eq!((header.sequence, header.base), (7, None));

        let kinds: Vec<u32> = attributes(&record, &header).map(|attribute| attribute.kind()).collect();
        assert_eq!(kinds, [STANDARD_INFORMATION, FILE_NAME, FILE_NAME, DATA]);

        let mut attributes = attributes(&record, &header);
        let standard = attributes
            .next()
            .and_then(|attribute| attribute.standard_information())
            .expect("standard information");
        assert_eq!(standard.modified, 132_000_000_000_000_000);
        assert_eq!(standard.attributes, 0x20);

        let short = attributes.next().and_then(|attribute| attribute.file_name()).expect("the short name");
        assert!(short.is_dos_alias());
        let long = attributes.next().and_then(|attribute| attribute.file_name()).expect("the long name");
        assert_eq!(long.name, "Program Files");
        assert_eq!(long.parent, 0x0005_0000_0000_0005);
        assert!(!long.is_dos_alias());
        assert_eq!(long.reparse_tag(), None);

        let data = attributes.next().expect("the data");
        assert_eq!(
            data.stream_size(),
            Some(Size { logical: 100, allocated: 104 }),
            "a resident stream rounds to eight"
        );
        assert_eq!(data.name(), None);
    }

    #[test]
    fn a_compressed_stream_occupies_the_clusters_it_holds_not_the_span_it_covers() {
        let span = Size {
            logical: 360_000,
            allocated: 360_448,
        };
        let plain = Writer::new(1, IN_USE, 0).data("", 0, span, 0, &[0x11, 0x58, 0x10]).finish();
        let compressed = Writer::new(1, IN_USE, 0)
            .data("", ATTRIBUTE_COMPRESSED, span, 45_056, &[0x11, 0x58, 0x10])
            .finish();
        let sparse = Writer::new(1, IN_USE, 0).data("", ATTRIBUTE_SPARSE, span, 8192, &[0x11, 0x58, 0x10]).finish();

        let size = |record: &[u8]| {
            let header = header(record).expect("valid");
            attributes(record, &header).next().and_then(|attribute| attribute.stream_size())
        };
        assert_eq!(size(&plain), Some(span));
        assert_eq!(
            size(&compressed),
            Some(Size {
                logical: 360_000,
                allocated: 45_056
            })
        );
        assert_eq!(size(&sparse).map(|size| size.allocated), Some(8192));
    }

    #[test]
    fn a_named_stream_and_a_reparse_point_are_told_apart_from_the_file() {
        let record = Writer::new(1, IN_USE, 0)
            .file_name(5, "link", 1, REPARSE_POINT_BIT, 0xA000_000C)
            .resident_data("WofCompressedData", 300)
            .reparse_point(0x8000_0017)
            .finish();
        let header = header(&record).expect("valid");
        let mut attributes = attributes(&record, &header);
        let name = attributes.next().and_then(|attribute| attribute.file_name()).expect("a name");
        assert_eq!(name.reparse_tag(), Some(0xA000_000C), "the name's copy of the tag");
        let stream = attributes.next().expect("a stream");
        assert_eq!(stream.name().as_deref(), Some("WofCompressedData"));
        assert_eq!(attributes.next().and_then(|attribute| attribute.reparse_tag()), Some(0x8000_0017));
        assert!(attributes.next().is_none(), "the end marker ends the attributes");
    }

    #[test]
    fn an_extension_record_names_its_base() {
        let record = Writer::new(3, IN_USE | IS_DIRECTORY, (2_u64 << 48) | 40).finish();
        let header = header(&record).expect("valid");
        assert!(header.is_directory());
        assert_eq!(header.base, Some((2_u64 << 48) | 40));
    }

    #[test]
    fn a_record_that_does_not_hold_together_has_no_header() {
        let good = Writer::new(1, IN_USE, 0).finish();
        let mut unsigned = good.clone();
        unsigned[0..4].copy_from_slice(b"BAAD");
        assert_eq!(header(&unsigned), None);

        let mut overrun = good.clone();
        overrun[0x18..0x1C].copy_from_slice(&4096_u32.to_le_bytes());
        assert_eq!(header(&overrun), None, "used past the record's end");

        let mut inside = good;
        inside[0x14..0x16].copy_from_slice(&0x10_u16.to_le_bytes());
        assert_eq!(header(&inside), None, "attributes starting inside the header");
    }

    #[test]
    fn an_attribute_with_a_broken_length_ends_the_reading() {
        let mut record = Writer::new(1, IN_USE, 0).resident_data("", 10).resident_data("", 20).finish();
        let header = header(&record).expect("valid");
        // The first attribute claims to be far longer than the record.
        record[0x38 + 4..0x38 + 8].copy_from_slice(&5000_u32.to_le_bytes());
        assert_eq!(attributes(&record, &header).count(), 0);
    }

    fn runs_of(map: &[u8], length: u64) -> Result<(u64, Vec<Run>), &'static str> {
        let record = Writer::new(1, IN_USE, 0)
            .data(
                "",
                0,
                Size {
                    logical: length,
                    allocated: length.next_multiple_of(4096),
                },
                0,
                map,
            )
            .finish();
        let header = header(&record).expect("valid");
        let attribute = attributes(&record, &header).next().expect("an attribute");
        attribute.runs(4096)
    }

    #[test]
    fn a_map_reads_as_runs_relative_to_each_other() {
        // 16 clusters at cluster 0x1000, then 8 clusters 0x100 clusters
        // earlier — a negative offset, sign-extended from two bytes.
        let (length, runs) = runs_of(&[0x21, 0x10, 0x00, 0x10, 0x21, 0x08, 0x00, 0xFF], 24 * 4096).expect("a valid map");
        assert_eq!(length, 24 * 4096);
        assert_eq!(
            runs,
            [
                Run::Data {
                    at: 0x1000 * 4096,
                    bytes: 16 * 4096
                },
                Run::Data {
                    at: 0x0F00 * 4096,
                    bytes: 8 * 4096
                },
            ]
        );
    }

    #[test]
    fn a_run_with_no_offset_is_a_hole() {
        let (_, runs) = runs_of(&[0x11, 0x04, 0x20, 0x01, 0x04], 8 * 4096).expect("a valid map");
        assert_eq!(runs[1], Run::Hole { bytes: 4 * 4096 });
    }

    #[test]
    fn a_map_that_does_not_cover_the_value_is_refused() {
        // What a table whose map continues in another record looks like.
        assert!(runs_of(&[0x11, 0x04, 0x20], 64 * 4096).is_err());
        assert!(runs_of(&[0x10, 0x04], 4 * 4096).is_err(), "a run of no clusters");
        assert!(runs_of(&[0x11, 0x04, 0xFF], 4 * 4096).is_err(), "a run before the volume");
    }
}
