//! The records of a volume's change journal, read from bytes.
//!
//! NTFS writes a record for every change to every file — created, written,
//! renamed, deleted — with the file, the folder it was in, the name it had and
//! when. The journal is read on Windows ([`super::journal`]); what its bytes
//! say is read here, and builds and is tested everywhere.

#![cfg_attr(not(windows), allow(dead_code, reason = "only the journal reader, which is Windows-only, reads records"))]

/// One change, as the journal recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Where in the journal the record is. Increases with every record.
    pub usn: i64,
    /// When the change happened, as a `FILETIME`.
    pub time: u64,
    /// The file the change is to, as a 64-bit file reference.
    pub file: u64,
    /// The folder the file was in when it changed.
    pub parent: u64,
    /// What changed: `USN_REASON_*` bits.
    pub reason: u32,
    /// The file's `FILE_ATTRIBUTE_*` bits when it changed — after a delete,
    /// the only thing that says whether a folder or a file went.
    pub attributes: u32,
    /// The file's name, without its folder. For a rename, the record of the
    /// old name carries the old one.
    pub name: String,
}

/// The data was overwritten.
pub const DATA_OVERWRITE: u32 = 0x0000_0001;
/// The data was extended.
pub const DATA_EXTEND: u32 = 0x0000_0002;
/// The data was truncated.
pub const DATA_TRUNCATION: u32 = 0x0000_0004;
/// A named stream was overwritten, extended or truncated: where the Windows
/// Overlay Filter keeps a compressed file's bytes.
const NAMED_DATA: u32 = 0x0000_0070;
/// The file was created.
pub const FILE_CREATE: u32 = 0x0000_0100;
/// The file was deleted.
pub const FILE_DELETE: u32 = 0x0000_0200;
/// The file was renamed away from the name in the record.
pub const RENAME_OLD_NAME: u32 = 0x0000_1000;
/// The file was renamed to the name in the record.
pub const RENAME_NEW_NAME: u32 = 0x0000_2000;
/// Timestamps or attribute bits changed.
const BASIC_INFO_CHANGE: u32 = 0x0000_8000;
/// A hard link was added or removed.
const HARD_LINK_CHANGE: u32 = 0x0001_0000;
/// The file was compressed or decompressed.
const COMPRESSION_CHANGE: u32 = 0x0002_0000;
/// A reparse point was added, changed or removed.
const REPARSE_POINT_CHANGE: u32 = 0x0010_0000;
/// A named stream was added, removed or renamed.
const STREAM_CHANGE: u32 = 0x0020_0000;

/// Every reason that can change what the tree holds: a name, a size, a time,
/// a trait. Security, extended attributes, object ids and the content index
/// change nothing midda shows, and are not asked for.
pub const WATCHED: u32 = DATA_OVERWRITE
    | DATA_EXTEND
    | DATA_TRUNCATION
    | NAMED_DATA
    | FILE_CREATE
    | FILE_DELETE
    | RENAME_OLD_NAME
    | RENAME_NEW_NAME
    | BASIC_INFO_CHANGE
    | HARD_LINK_CHANGE
    | COMPRESSION_CHANGE
    | REPARSE_POINT_CHANGE
    | STREAM_CHANGE;

/// `FILE_ATTRIBUTE_DIRECTORY`.
pub const DIRECTORY: u32 = 0x0000_0010;

/// The records in the output of one journal read: the next USN to ask for,
/// and the records after it.
///
/// Version 2 records carry 64-bit file references and version 3 records
/// 128-bit ids; an NTFS id fits the low 64 bits of either. A record that does
/// not hold together ends the reading — the rest of the buffer is not trusted.
#[must_use]
pub fn parse(buffer: &[u8]) -> (Option<i64>, Vec<Record>) {
    let next = buffer.get(0..8).and_then(|bytes| bytes.try_into().ok()).map(i64::from_le_bytes);
    let mut records = Vec::new();
    let mut at = 8_usize;
    while let Some(record) = buffer.get(at..) {
        let Some(length) = u32_at(record, 0).and_then(|length| usize::try_from(length).ok()) else {
            break;
        };
        // The smallest record is the version 2 header with an empty name.
        if length < 0x3C || length > record.len() {
            break;
        }
        let Some(parsed) = one(&record[..length]) else {
            break;
        };
        records.push(parsed);
        at += length;
    }
    (next, records)
}

fn one(record: &[u8]) -> Option<Record> {
    let major = u16_at(record, 4)?;
    let (file, parent, rest) = match major {
        2 => (u64_at(record, 0x08)?, u64_at(record, 0x10)?, 0x18),
        // A 128-bit id: an NTFS file reference is its low half.
        3 => (u64_at(record, 0x08)?, u64_at(record, 0x18)?, 0x28),
        _ => return None,
    };
    let name_length = usize::from(u16_at(record, rest + 0x20)?);
    let name_at = usize::from(u16_at(record, rest + 0x22)?);
    let name = record.get(name_at..name_at.checked_add(name_length)?)?;
    let units: Vec<u16> = name.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    Some(Record {
        usn: i64::from_le_bytes(record.get(rest..rest + 8)?.try_into().ok()?),
        time: u64_at(record, rest + 0x08)?,
        file,
        parent,
        reason: u32_at(record, rest + 0x10)?,
        attributes: u32_at(record, rest + 0x1C)?,
        name: String::from_utf16_lossy(&units),
    })
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    bytes.get(at..at + 2)?.try_into().ok().map(u16::from_le_bytes)
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at + 4)?.try_into().ok().map(u32::from_le_bytes)
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    bytes.get(at..at + 8)?.try_into().ok().map(u64::from_le_bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A version 2 record, laid out as NTFS writes one.
    pub(crate) fn version_2(record: &Record) -> Vec<u8> {
        let name: Vec<u8> = record.name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let length = (0x3C + name.len()).next_multiple_of(8);
        let mut bytes = vec![0_u8; length];
        bytes[0..4].copy_from_slice(&u32::try_from(length).expect("short").to_le_bytes());
        bytes[4..6].copy_from_slice(&2_u16.to_le_bytes());
        bytes[0x08..0x10].copy_from_slice(&record.file.to_le_bytes());
        bytes[0x10..0x18].copy_from_slice(&record.parent.to_le_bytes());
        bytes[0x18..0x20].copy_from_slice(&record.usn.to_le_bytes());
        bytes[0x20..0x28].copy_from_slice(&record.time.to_le_bytes());
        bytes[0x28..0x2C].copy_from_slice(&record.reason.to_le_bytes());
        bytes[0x34..0x38].copy_from_slice(&record.attributes.to_le_bytes());
        bytes[0x38..0x3A].copy_from_slice(&u16::try_from(name.len()).expect("short").to_le_bytes());
        bytes[0x3A..0x3C].copy_from_slice(&0x3C_u16.to_le_bytes());
        bytes[0x3C..0x3C + name.len()].copy_from_slice(&name);
        bytes
    }

    fn version_3(record: &Record) -> Vec<u8> {
        let name: Vec<u8> = record.name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let length = (0x4C + name.len()).next_multiple_of(8);
        let mut bytes = vec![0_u8; length];
        bytes[0..4].copy_from_slice(&u32::try_from(length).expect("short").to_le_bytes());
        bytes[4..6].copy_from_slice(&3_u16.to_le_bytes());
        bytes[0x08..0x10].copy_from_slice(&record.file.to_le_bytes());
        bytes[0x18..0x20].copy_from_slice(&record.parent.to_le_bytes());
        bytes[0x28..0x30].copy_from_slice(&record.usn.to_le_bytes());
        bytes[0x30..0x38].copy_from_slice(&record.time.to_le_bytes());
        bytes[0x38..0x3C].copy_from_slice(&record.reason.to_le_bytes());
        bytes[0x44..0x48].copy_from_slice(&record.attributes.to_le_bytes());
        bytes[0x48..0x4A].copy_from_slice(&u16::try_from(name.len()).expect("short").to_le_bytes());
        bytes[0x4A..0x4C].copy_from_slice(&0x4C_u16.to_le_bytes());
        bytes[0x4C..0x4C + name.len()].copy_from_slice(&name);
        bytes
    }

    fn sample(usn: i64, name: &str) -> Record {
        Record {
            usn,
            time: 133_000_000_000_000_000 + u64::try_from(usn).expect("positive"),
            file: (3_u64 << 48) | 1234,
            parent: (1_u64 << 48) | 5,
            reason: FILE_CREATE | DATA_EXTEND,
            attributes: 0x20,
            name: name.to_owned(),
        }
    }

    #[test]
    fn a_read_comes_back_as_its_next_usn_and_its_records() {
        let mut buffer = 9_000_i64.to_le_bytes().to_vec();
        buffer.extend(version_2(&sample(8_000, "report.pdf")));
        buffer.extend(version_3(&sample(8_100, "Résumé 報告.docx")));
        let (next, records) = parse(&buffer);
        assert_eq!(next, Some(9_000));
        assert_eq!(records, [sample(8_000, "report.pdf"), sample(8_100, "Résumé 報告.docx")]);
    }

    #[test]
    fn a_read_that_found_nothing_is_only_the_next_usn() {
        let (next, records) = parse(&42_i64.to_le_bytes());
        assert_eq!(next, Some(42));
        assert!(records.is_empty());
        assert_eq!(parse(&[]), (None, Vec::new()));
    }

    #[test]
    fn a_record_that_does_not_hold_together_ends_the_reading() {
        let mut buffer = 1_i64.to_le_bytes().to_vec();
        buffer.extend(version_2(&sample(10, "kept")));
        let mut broken = version_2(&sample(20, "broken"));
        broken[0x38..0x3A].copy_from_slice(&900_u16.to_le_bytes());
        buffer.extend(broken);
        buffer.extend(version_2(&sample(30, "after")));
        let (_, records) = parse(&buffer);
        assert_eq!(records.len(), 1, "nothing past a broken record is trusted");

        let mut unknown = version_2(&sample(40, "v4"));
        unknown[4..6].copy_from_slice(&4_u16.to_le_bytes());
        let mut buffer = 1_i64.to_le_bytes().to_vec();
        buffer.extend(unknown);
        assert!(parse(&buffer).1.is_empty(), "a version this does not know is not guessed at");
    }
}
