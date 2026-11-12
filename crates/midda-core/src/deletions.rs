//! What was deleted, and how each comes back: the log the window keeps.
//!
//! A deletion of what a tool makes again is safe because the tool makes it
//! again - and a month later nobody remembers which tool, or where to run it.
//! Every entry midda deletes is written down as it goes: when, what the rule
//! called it, how it went (for good, or into the Recycle Bin), what it gave
//! back as measured, and how it comes back - `pnpm install`, run in the folder
//! beside it. The window lists the log; a person can read the file.
//!
//! One JSON object per line, appended and flushed to the volume before the
//! next entry is deleted: a crash loses at most the line of the entry in
//! hand. A line that does not read - a write cut short - is passed over, not
//! taken for the end of the log.

use std::io::{Read as _, Seek as _, Write as _};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::delete::Way;
use crate::rules::{Goes, Risk};

/// One entry deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deletion {
    /// When it went, as milliseconds since the Unix epoch.
    pub at: i64,
    /// The entry the rule found: for a folder that was emptied, the folder.
    pub path: String,
    /// What the rule calls what it found, and which rule.
    pub name: String,
    pub rule: String,
    pub risk: Risk,
    pub way: Way,
    /// Whether the entry went, or what was in it.
    pub goes: Goes,
    /// What it took on disk before, and what of that went - measured, by
    /// reading it again afterwards. Into the Recycle Bin, "went" is moved:
    /// the space comes back when the bin is emptied.
    pub held: u64,
    pub gone: u64,
    /// How many entries stayed: held open, refused, not taken by the bin.
    pub left: u64,
    /// How it comes back: what to run, where - the folder beside it - and in
    /// a sentence.
    pub command: Option<String>,
    pub run_in: Option<String>,
    pub explain: String,
}

/// The log, at a file.
#[derive(Debug, Clone)]
pub struct Log {
    path: PathBuf,
}

impl Log {
    /// The log kept in the file at `path`, made when the first entry is
    /// written.
    #[must_use]
    pub const fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The file the log is kept in.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Writes `deletion` down, and makes sure it is on the volume.
    ///
    /// # Errors
    ///
    /// When the file cannot be made, opened or written.
    pub fn write(&self, deletion: &Deletion) -> std::io::Result<()> {
        if let Some(folder) = self.path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let mut line = serde_json::to_vec(deletion).map_err(std::io::Error::other)?;
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new().create(true).read(true).append(true).open(&self.path)?;
        // A line cut short by a crash would take this one with it: it starts
        // on a line of its own.
        if file.metadata()?.len() > 0 {
            let mut last = [0_u8];
            file.seek(std::io::SeekFrom::End(-1))?;
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                line.insert(0, b'\n');
            }
        }
        // One write per line: an append of a whole line is not interleaved
        // with another's.
        file.write_all(&line)?;
        file.sync_data()
    }

    /// Every entry written down, the latest first. Nothing, when nothing has
    /// been deleted yet.
    ///
    /// # Errors
    ///
    /// When the file is there and cannot be read.
    pub fn read(&self) -> std::io::Result<Vec<Deletion>> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut deletions: Vec<Deletion> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
        deletions.reverse();
        Ok(deletions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deletion(at: i64, path: &str) -> Deletion {
        Deletion {
            at,
            path: path.to_owned(),
            name: "node_modules".to_owned(),
            rule: "node-modules".to_owned(),
            risk: Risk::Safe,
            way: Way::ForGood,
            goes: Goes::Whole,
            held: 4096,
            gone: 4096,
            left: 0,
            command: Some("pnpm install".to_owned()),
            run_in: Some("C:\\work\\app".to_owned()),
            explain: "Installed from the lockfile beside it.".to_owned(),
        }
    }

    #[test]
    fn what_was_written_reads_back_the_latest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = Log::at(dir.path().join("data/deletions.jsonl"));
        assert_eq!(log.read().expect("read"), Vec::new(), "nothing yet");
        log.write(&deletion(1, "C:\\work\\app\\node_modules")).expect("written");
        log.write(&deletion(2, "C:\\work\\site\\node_modules")).expect("written");
        let read = log.read().expect("read");
        assert_eq!(read.iter().map(|deletion| deletion.at).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(read[1], deletion(1, "C:\\work\\app\\node_modules"), "every field, back as it was");
    }

    #[test]
    fn a_line_cut_short_is_passed_over() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("deletions.jsonl");
        let log = Log::at(path.clone());
        log.write(&deletion(1, "a")).expect("written");
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
        file.write_all(b"{\"at\":2,\"path\":\"b\",\"na").expect("a torn line");
        drop(file);
        log.write(&deletion(3, "c")).expect("written after it");
        let read = log.read().expect("read");
        assert_eq!(
            read.iter().map(|deletion| deletion.path.as_str()).collect::<Vec<_>>(),
            vec!["c", "a"],
            "the torn line is passed over, and the one after it is not lost with it"
        );
    }
}
