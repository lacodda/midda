//! What only a file can say about itself: where a shortcut points, what an
//! installer installs, whether a folder says it is a cache, and what the
//! Recycle Bin's records say it holds.
//!
//! A rule names an entry by where it is and how old it is — that is the
//! tree's to answer. Whether `Foo.lnk` on the Desktop points to anything, or
//! whether the program `setup.exe` in Downloads installs is on this computer,
//! is not in the tree: the file has to be read, and for an installer the
//! computer asked what it has installed. That is done after the survey, away
//! from the tree and the lock that guards it ([`crate::rules::Waiting`]):
//! the survey runs whenever the tree changes, and a check reads files.
//!
//! A check says one of three things: the entry is none of its rule's (a
//! shortcut that works), the entry is the rule's by its last way back, or it
//! is the rule's by the way back that names what the check saw (`if =
//! "installed"`). What it cannot tell it never calls the worse case: a
//! shortcut to a drive that is not connected is not broken, and an installer
//! that does not say what it installs is not "installed".
//!
//! See `docs/adr/0016-what-windows-and-downloads-leave-behind.md`.

pub mod installer;
pub mod shortcut;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::rules::{Finding, RuleSet, Waiting};

/// What a rule asks a file or a folder, as a rule writes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Check {
    /// Whether a shortcut's target is there. Only a shortcut to nothing is
    /// the rule's.
    Shortcut,
    /// Whether what an installer installs is installed here.
    Installer,
    /// Whether a file in the folder starts as the rule says. Written as
    /// `[rule.marker]`, never as `check`: a marker with nothing to look for
    /// cannot be written.
    #[serde(skip)]
    Marker(Marker),
}

/// A file a folder holds to say what it is, and how it begins: a
/// `CACHEDIR.TAG` that starts with the standard's signature says the folder
/// is a tool's cache (bford.info/cachedir).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Marker {
    /// The file's name, as the folder holds it: a name, never a pattern.
    pub file: String,
    /// What the file starts with, byte for byte.
    pub starts: std::sync::Arc<[u8]>,
}

impl Check {
    /// What a rules file calls it.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        match self {
            Self::Shortcut => "shortcut",
            Self::Installer => "installer",
            Self::Marker(_) => "marker",
        }
    }

    /// Whether what this check reads is a folder rather than a file.
    #[must_use]
    pub const fn reads_folders(&self) -> bool {
        matches!(self, Self::Marker(_))
    }

    /// Whether this check can answer `outcome`, so a way back can be taken
    /// by it.
    #[must_use]
    pub const fn says(&self, outcome: Outcome) -> bool {
        matches!((self, outcome), (Self::Installer, Outcome::Installed | Outcome::OlderInstalled))
    }
}

/// What a way back can be taken by (`if`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// What the installer installs is installed, in its version or a newer
    /// one.
    Installed,
    /// An older version than the one the installer installs is: it may be
    /// an update not yet run.
    OlderInstalled,
}

impl Outcome {
    /// What a rules file calls it.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::OlderInstalled => "older-installed",
        }
    }
}

/// What a check saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// A shortcut whose target is not there: the target as the shortcut
    /// names it.
    Broken { target: String },
    /// A shortcut that works — or whose target cannot be told from here: on
    /// a share, on a drive that is not connected, a place of the shell's
    /// rather than a file. None of the rule's.
    Fine,
    /// An installer, and what of it is installed.
    Installer(installer::Verdict),
    /// A folder whose marker file starts as its rule says.
    Marked,
    /// A Recycle Bin folder, and what its records say of it, when any read.
    Bin { about: Option<String> },
}

impl Seen {
    /// Whether the entry is its rule's, and by which way back: `None` for
    /// none of the rule's; `Some(None)` for the rule's by its last way.
    #[must_use]
    pub fn outcome(&self) -> Option<Option<Outcome>> {
        match self {
            Self::Broken { .. } | Self::Marked | Self::Bin { .. } => Some(None),
            Self::Fine => None,
            Self::Installer(verdict) => Some(verdict.outcome),
        }
    }

    /// What the reader is told of it, beside the rule's own sentence.
    #[must_use]
    pub fn about(&self) -> Option<String> {
        match self {
            Self::Broken { target } => Some(format!("It points to {target}, which is not there.")),
            Self::Fine | Self::Marked => None,
            Self::Installer(verdict) => Some(verdict.about.clone()),
            Self::Bin { about } => about.clone(),
        }
    }
}

/// What a batch of checks asks of the computer once, rather than per file:
/// the programs installed.
#[derive(Debug, Clone, Default)]
pub struct Machine {
    programs: Vec<installer::Program>,
}

impl Machine {
    /// This computer, as it is now: none of it on another platform.
    #[must_use]
    pub fn read() -> Self {
        Self {
            programs: installer::installed(),
        }
    }

    /// A computer with these programs installed, and no Windows Installer
    /// to ask: what a test stands in for this one with.
    #[must_use]
    pub const fn with(programs: Vec<installer::Program>) -> Self {
        Self { programs }
    }
}

/// Asks the entry at `path` what `check` needs to know. Reads the file - for
/// a folder, the file in it the check is about - and for an installer asks
/// Windows Installer; changes nothing, the time a file was last opened
/// included, which the read would otherwise make today.
#[must_use]
pub fn look(path: &Path, check: &Check, machine: &Machine) -> Seen {
    match check {
        Check::Shortcut => {
            let _kept = crate::platform::keeping_access_time(path);
            shortcut::look(path)
        }
        Check::Installer => {
            let _kept = crate::platform::keeping_access_time(path);
            Seen::Installer(installer::look(path, &machine.programs))
        }
        Check::Marker(marker) => {
            let file = path.join(&marker.file);
            let _kept = crate::platform::keeping_access_time(&file);
            if starts_with(&file, &marker.starts) { Seen::Marked } else { Seen::Fine }
        }
    }
}

/// What a rule's finding is told more of, read from its files once it is
/// found: it says nothing about whether it is one (`about`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum About {
    /// What the records of a Recycle Bin folder say it holds: how many
    /// things, deleted when, by whom.
    RecycleBin,
}

impl About {
    /// Whether what this reads is a folder rather than a file.
    #[must_use]
    pub const fn reads_folders(self) -> bool {
        matches!(self, Self::RecycleBin)
    }
}

/// Reads what `about` tells of the entry at `path`.
#[must_use]
pub fn tell(path: &Path, about: About) -> Seen {
    match about {
        About::RecycleBin => Seen::Bin {
            about: crate::recycled::read(path).about(SystemTime::now()),
        },
    }
}

/// A finding whose rule has more to tell of it, read away from the tree as
/// a check is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Told {
    pub path: PathBuf,
    pub about: About,
    pub finding: Finding,
}

/// Whether the file at `path` begins with `start`: only that many bytes are
/// read. A file that cannot be read does not.
fn starts_with(path: &Path, start: &[u8]) -> bool {
    use std::io::Read as _;
    let mut head = vec![0; start.len()];
    std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut head)).is_ok() && head == start
}

/// What checks saw of files, kept by path as of the size and the time each
/// had then.
///
/// The survey runs again whenever the tree changes, and nearly every change
/// is to some other file: asking every shortcut and installer again each time
/// would read them all for nothing. A file whose size or time moved is read
/// again; one that is gone is forgotten.
#[derive(Debug, Clone, Default)]
pub struct Kept {
    seen: HashMap<PathBuf, (u64, Option<SystemTime>, Seen)>,
    /// What was told of findings, the same way.
    told: HashMap<PathBuf, (u64, Option<SystemTime>, Seen)>,
}

impl Kept {
    /// The entries of `waiting` that are their rules', each with what its
    /// check saw — kept when the file is as it was, asked of `look` when it
    /// is not.
    pub fn settle(&mut self, rules: &RuleSet, waiting: &[Waiting], look: &mut dyn FnMut(&Waiting) -> Seen) -> Vec<(Finding, Seen)> {
        let mut settled = Vec::new();
        let mut kept = HashMap::with_capacity(waiting.len());
        for entry in waiting {
            let size = entry.finding.holds.logical;
            let written = entry.finding.newest;
            let seen = match self.seen.remove(&entry.path) {
                Some((was, then, seen)) if was == size && then == written => seen,
                _ => look(entry),
            };
            if let Some(finding) = rules.settle(entry, &seen) {
                settled.push((finding, seen.clone()));
            }
            kept.insert(entry.path.clone(), (size, written, seen));
        }
        self.seen = kept;
        settled
    }

    /// What is told of each of `told`, by the entry - kept when it is as it
    /// was, read with `tell` when it is not.
    pub fn tell(&mut self, told: &[Told], tell: &mut dyn FnMut(&Told) -> Seen) -> Vec<(crate::NodeId, Seen)> {
        let mut said = Vec::with_capacity(told.len());
        let mut kept = HashMap::with_capacity(told.len());
        for entry in told {
            let size = entry.finding.holds.logical;
            let written = entry.finding.newest;
            let seen = match self.told.remove(&entry.path) {
                Some((was, then, seen)) if was == size && then == written => seen,
                _ => tell(entry),
            };
            said.push((entry.finding.id, seen.clone()));
            kept.insert(entry.path.clone(), (size, written, seen));
        }
        self.told = kept;
        said
    }
}
