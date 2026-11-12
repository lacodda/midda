//! What the Recycle Bin says of what it holds: when each thing was deleted,
//! from where, and whose bin it is.
//!
//! The bin is a folder per account on every volume, `$Recycle.Bin\<SID>`.
//! Each thing in it is two entries: `$R…`, the thing itself under a made-up
//! name, and `$I…`, a small record the shell wrote when it was deleted - the
//! size, the moment, and the path it came from. Version 2 (Windows 10 on) is a
//! version, the size, the moment as a `FILETIME`, the length of the path in
//! UTF-16 units and the path; version 1 (Vista to 8.1) is the same with the
//! path in a fixed 260 units and no length. A record that is neither is not
//! counted, and nor is a `$R` without one: what cannot be read is not said.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// What one `$I` record says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// What the thing read as when it was deleted.
    pub bytes: u64,
    /// When it was deleted, when the record gives a moment.
    pub deleted: Option<SystemTime>,
    /// Where it was.
    pub from: String,
}

/// What a bin folder holds, by its records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bin {
    /// Whose bin it is, by the account's name, when the machine can say.
    pub account: Option<String>,
    /// The records that read, the earliest deleted first.
    pub records: Vec<Record>,
}

impl Bin {
    /// The earliest and the latest moment anything in it was deleted.
    #[must_use]
    pub fn span(&self) -> Option<(SystemTime, SystemTime)> {
        let mut moments = self.records.iter().filter_map(|record| record.deleted);
        let first = moments.next()?;
        Some(moments.fold((first, first), |(early, late), at| (early.min(at), late.max(at))))
    }

    /// What a reader is told of it beside the rule's sentence: how many
    /// things, whose, and from when to when - "12 things deleted by ada, from
    /// 3 days to 5 months ago." `None` when no record read.
    #[must_use]
    pub fn about(&self, now: SystemTime) -> Option<String> {
        let count = self.records.len();
        if count == 0 {
            return None;
        }
        let things = if count == 1 {
            "1 thing".to_owned()
        } else {
            format!("{} things", crate::words::count(count as u64))
        };
        let by = self.account.as_ref().map(|name| format!(" by {name}")).unwrap_or_default();
        let when = match self.span() {
            None => String::new(),
            Some((early, late)) => {
                let (newest, oldest) = (ago(now, late), ago(now, early));
                if count == 1 || newest == oldest {
                    format!(", {oldest}")
                } else {
                    format!(", from {} to {oldest}", newest.trim_end_matches(" ago"))
                }
            }
        };
        Some(format!("{things} deleted{by}{when}."))
    }
}

/// How long before `now` a moment was, in the largest whole unit a reader
/// would say: "today", "3 days ago", "5 months ago", "2 years ago".
#[must_use]
pub fn ago(now: SystemTime, then: SystemTime) -> String {
    const DAY: u64 = 24 * 60 * 60;
    let days = now.duration_since(then).unwrap_or(Duration::ZERO).as_secs() / DAY;
    let (count, unit) = match days {
        0 => return "today".to_owned(),
        1..=13 => (days, "day"),
        14..=59 => (days / 7, "week"),
        60..=729 => (days / 30, "month"),
        _ => (days / 365, "year"),
    };
    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}

/// Reads one `$I` record, as either version writes it.
#[must_use]
pub fn record(bytes: &[u8]) -> Option<Record> {
    let word = |at: usize| bytes.get(at..at + 8).and_then(|slice| slice.try_into().ok()).map(u64::from_le_bytes);
    let version = word(0)?;
    let size = word(8)?;
    let moment = word(16)?;
    let units: &[u8] = match version {
        1 => bytes.get(24..24 + 520)?,
        2 => {
            let length = bytes.get(24..28).and_then(|slice| slice.try_into().ok()).map(u32::from_le_bytes)? as usize;
            bytes.get(28..28 + length.checked_mul(2)?)?
        }
        _ => return None,
    };
    let path: Vec<u16> = units
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .take_while(|&unit| unit != 0)
        .collect();
    if path.is_empty() {
        return None;
    }
    Some(Record {
        bytes: size,
        deleted: from_filetime(moment),
        from: String::from_utf16_lossy(&path),
    })
}

/// A `FILETIME` - hundreds of nanoseconds since 1601 - as a moment, when it
/// is one: zero is "not given".
fn from_filetime(ticks: u64) -> Option<SystemTime> {
    const TO_UNIX: u64 = 116_444_736_000_000_000;
    let since_unix = ticks.checked_sub(TO_UNIX)?;
    (ticks != 0).then(|| SystemTime::UNIX_EPOCH + Duration::from_nanos(since_unix.saturating_mul(100)))
}

/// What the bin folder at `folder` holds, by its records, and whose it is by
/// the SID its name is. Reads only the small `$I` files, and leaves the time
/// each was last opened as it was.
#[must_use]
pub fn read(folder: &Path) -> Bin {
    let account = folder.file_name().and_then(|name| name.to_str()).and_then(crate::shell::account_name);
    let mut records: Vec<Record> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.len() > 2 && name[..2].eq_ignore_ascii_case("$I"))
        })
        .filter_map(|entry| {
            let path = entry.path();
            let _kept = crate::platform::keeping_access_time(&path);
            std::fs::read(&path).ok().as_deref().and_then(record)
        })
        .collect();
    records.sort_by_key(|record| record.deleted);
    Bin { account, records }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    /// A record as Windows 10 writes it.
    fn second(size: u64, deleted: SystemTime, from: &str) -> Vec<u8> {
        let ticks = deleted.duration_since(SystemTime::UNIX_EPOCH).expect("after 1970").as_nanos() / 100 + 116_444_736_000_000_000;
        let path: Vec<u16> = from.encode_utf16().chain(std::iter::once(0)).collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_u64.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&u64::try_from(ticks).expect("a moment").to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(path.len()).expect("a path").to_le_bytes());
        bytes.extend(path.iter().flat_map(|unit| unit.to_le_bytes()));
        bytes
    }

    #[test]
    fn a_record_of_either_version_says_what_when_and_from_where() {
        let moment = SystemTime::UNIX_EPOCH + DAY * 20_000;
        let read = record(&second(5000, moment, r"C:\Users\ada\Downloads\setup.exe")).expect("reads");
        assert_eq!(read.bytes, 5000);
        assert_eq!(read.deleted, Some(moment));
        assert_eq!(read.from, r"C:\Users\ada\Downloads\setup.exe");

        // Version 1: the same, with the path in 260 units and no length.
        let mut first = second(7, moment, "");
        first.truncate(24);
        first[0] = 1;
        let mut path: Vec<u8> = r"D:\old.txt".encode_utf16().flat_map(u16::to_le_bytes).collect();
        path.resize(520, 0);
        first.extend(path);
        assert_eq!(record(&first).map(|read| read.from).as_deref(), Some(r"D:\old.txt"));

        assert_eq!(record(&[3, 0, 0, 0, 0, 0, 0, 0]), None, "a version nobody wrote");
        let mut short = second(1, moment, r"C:\x");
        short.truncate(30);
        assert_eq!(record(&short), None, "a path longer than the record");
    }

    #[test]
    fn a_bin_says_how_many_things_whose_and_from_when() {
        let now = SystemTime::UNIX_EPOCH + DAY * 20_000;
        let at = |days: u32, from: &str| Record {
            bytes: 1,
            deleted: Some(now - DAY * days),
            from: from.to_owned(),
        };
        let bin = Bin {
            account: Some("ada".to_owned()),
            records: vec![at(150, "a"), at(3, "b"), at(40, "c")],
        };
        assert_eq!(bin.about(now).as_deref(), Some("3 things deleted by ada, from 3 days to 5 months ago."));
        let one = Bin {
            account: None,
            records: vec![at(0, "a")],
        };
        assert_eq!(one.about(now).as_deref(), Some("1 thing deleted, today."));
        assert_eq!(Bin::default().about(now), None, "nothing read is nothing said");
        assert_eq!(ago(now, now - DAY * 800), "2 years ago");
        assert_eq!(ago(now, now - DAY * 20), "2 weeks ago");
    }

    #[test]
    fn a_folder_of_records_is_read_and_what_is_not_a_record_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = SystemTime::now();
        std::fs::write(dir.path().join("$IABC123.txt"), second(10, now - DAY * 2, r"C:\a.txt")).expect("write");
        std::fs::write(dir.path().join("$RABC123.txt"), b"the thing itself").expect("write");
        std::fs::write(dir.path().join("$IBROKEN"), b"short").expect("write");
        std::fs::write(dir.path().join("desktop.ini"), b"[.ShellClassInfo]").expect("write");
        let bin = read(dir.path());
        assert_eq!(bin.records.len(), 1);
        assert_eq!(bin.records[0].from, r"C:\a.txt");
    }
}
