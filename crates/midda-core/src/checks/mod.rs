//! What only a file can say about itself: where a shortcut points, and what
//! an installer installs.
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

/// What a rule asks a file, as a rule writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Check {
    /// Whether a shortcut's target is there. Only a shortcut to nothing is
    /// the rule's.
    Shortcut,
    /// Whether what an installer installs is installed here.
    Installer,
}

impl Check {
    /// What a rules file calls it.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Shortcut => "shortcut",
            Self::Installer => "installer",
        }
    }

    /// Whether this check can answer `outcome`, so a way back can be taken
    /// by it.
    #[must_use]
    pub const fn says(self, outcome: Outcome) -> bool {
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
}

impl Seen {
    /// Whether the entry is its rule's, and by which way back: `None` for
    /// none of the rule's; `Some(None)` for the rule's by its last way.
    #[must_use]
    pub fn outcome(&self) -> Option<Option<Outcome>> {
        match self {
            Self::Broken { .. } => Some(None),
            Self::Fine => None,
            Self::Installer(verdict) => Some(verdict.outcome),
        }
    }

    /// What the reader is told of it, beside the rule's own sentence.
    #[must_use]
    pub fn about(&self) -> Option<String> {
        match self {
            Self::Broken { target } => Some(format!("It points to {target}, which is not there.")),
            Self::Fine => None,
            Self::Installer(verdict) => Some(verdict.about.clone()),
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

/// Asks the file at `path` what `check` needs to know. Reads it, and for an
/// installer asks Windows Installer; changes nothing.
#[must_use]
pub fn look(path: &Path, check: Check, machine: &Machine) -> Seen {
    match check {
        Check::Shortcut => shortcut::look(path),
        Check::Installer => Seen::Installer(installer::look(path, &machine.programs)),
    }
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
}
