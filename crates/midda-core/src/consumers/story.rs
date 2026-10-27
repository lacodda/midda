//! Why a disk is full, said as a paragraph: how full it is, then the few
//! consumers that take most of it, each with the places that hold the most
//! of it.
//!
//! One text for a person and for an assistant asked the same question: the
//! window shows it as it is, and the brief - the same paragraph and every
//! consumer's places, as Markdown - is what goes into a chat, and later what
//! the CLI and the MCP door answer with. Written here rather than in the
//! window so that all of them say the same sentence.
//!
//! On disk, always, whatever size the window shows: "full" is a fact about
//! clusters, and a paragraph that set what files read as against how full the
//! volume is would be two measures in one sentence.

use std::fmt::Write as _;
use std::path::{Component, Path};

use super::{Classified, Consumer, Line, Share, Summary};
use crate::platform::Space;
use crate::rules::RuleSet;
use crate::size::SizeBasis;
use crate::tree::{NodeId, ROOT, Tree};
use crate::vdisk::{self, Looked};
use crate::words;

/// The most consumers the paragraph names: five reasons are an answer, nine
/// are a table.
pub const REASONS: usize = 5;

/// A consumer is a reason when it holds at least one part in this many of
/// the whole: a fiftieth.
const LEAST: u64 = 50;

/// A place is named in a reason when it holds at least one part in this
/// many of the reason.
const NOTABLE: u64 = 10;

/// The most places named in one reason.
const PLACES_SAID: usize = 2;

/// The most places listed under a consumer in the brief.
const PLACES_LISTED: usize = 5;

/// Why a disk is full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Story {
    /// The paragraph, as a person reads it.
    pub text: String,
    /// The paragraph and every consumer with the places that hold most of
    /// it, as Markdown: what an assistant is given.
    pub brief: String,
}

/// Tells why `tree` is as full as it is. `space` is the volume's, when the
/// tree is of a whole volume and its size could be read. Reads inside the
/// virtual disks in it ([`vdisk::look_all`]).
#[must_use]
pub fn tell(rules: &RuleSet, tree: &Tree, classified: &Classified, space: Option<Space>) -> Story {
    let disks = vdisk::look_all(vdisk::find(rules, tree, classified));
    tell_from(rules, tree, classified, &classified.summary(tree, SizeBasis::Allocated), space, &disks)
}

/// [`tell`], from a summary already made and the virtual disks already looked
/// into. A summary on another size than on disk is made again on disk: the
/// paragraph does not mix the two.
#[must_use]
pub fn tell_from(rules: &RuleSet, tree: &Tree, classified: &Classified, summary: &Summary, space: Option<Space>, disks: &[Looked]) -> Story {
    let on_disk;
    let summary = if summary.basis == SizeBasis::Allocated {
        summary
    } else {
        on_disk = classified.summary(tree, SizeBasis::Allocated);
        &on_disk
    };
    let teller = Teller {
        rules,
        tree,
        classified,
        summary,
        space: space.filter(|_| is_volume(tree.root_path())),
        disks,
    };
    Story {
        text: teller.paragraph(false),
        brief: teller.brief(),
    }
}

/// Whether `path` is the top of a volume: nothing in it is a folder's name.
#[must_use]
pub fn is_volume(path: &Path) -> bool {
    !path.components().any(|component| matches!(component, Component::Normal(_)))
}

struct Teller<'a> {
    rules: &'a RuleSet,
    tree: &'a Tree,
    classified: &'a Classified,
    summary: &'a Summary,
    space: Option<Space>,
    disks: &'a [Looked],
}

impl Teller<'_> {
    fn root(&self) -> String {
        self.tree.root_path().display().to_string()
    }

    /// The paragraph; with `marked`, paths as Markdown code, so that a chat
    /// does not read the underscores in `__pycache__` as emphasis.
    fn paragraph(&self, marked: bool) -> String {
        let total = self.summary.total.allocated;
        let mut text = self.opening();
        if total == 0 {
            return text;
        }
        let reasons: Vec<&Share> = self
            .summary
            .shares
            .iter()
            .filter(|share| share.size.allocated.saturating_mul(LEAST) >= total)
            .take(REASONS)
            .collect();
        if reasons.is_empty() {
            text.push_str(" Nothing in it is large enough to be a reason on its own.");
            return text;
        }
        let covered: u64 = reasons.iter().map(|share| share.size.allocated).sum();
        let count = count_in_words(reasons.len());
        let things = if reasons.len() == 1 { "thing" } else { "things" };
        if covered.saturating_mul(2) >= total {
            let _ = write!(text, " Most of it is {count} {things}.");
        } else {
            let _ = write!(text, " {} {things} take the largest part of it.", capitalised(count));
        }
        for share in reasons {
            text.push(' ');
            text.push_str(&self.reason(share, marked));
        }
        text
    }

    /// How full the volume is, or what the folder holds.
    fn opening(&self) -> String {
        let root = self.root();
        let scanned = self.summary.total.allocated;
        let Some(space) = self.space.filter(|space| space.total > 0) else {
            return format!("{root} holds {}.", words::bytes(scanned));
        };
        let used = space.total.saturating_sub(space.free);
        let mut text = format!(
            "{root} is {} full: {} used of {}, {} free.",
            words::share(used, space.total),
            words::bytes(used),
            words::bytes(space.total),
            words::bytes(space.free)
        );
        // What the volume says is used and what a scan can open are not the
        // same: the file table, shadow copies and what was refused are in
        // the first and not the second. Said when it is more than a little.
        if scanned.saturating_mul(20) < used.saturating_mul(19) {
            let _ = write!(text, " The scan counts {} of it; the rest is in what it could not open.", words::bytes(scanned));
        }
        text
    }

    /// One consumer as a sentence or two.
    fn reason(&self, share: &Share, marked: bool) -> String {
        let consumer = share.consumer;
        let size = share.size.allocated;
        let mut text = format!(
            "{} {} {} ({})",
            consumer.label(),
            if matches!(consumer, Consumer::System | Consumer::Development) {
                "takes"
            } else {
                "take"
            },
            words::bytes(size),
            words::share(size, self.summary.total.allocated)
        );
        let most = self.most(share);
        let names: Vec<String> = most.iter().map(|line| self.name(share.consumer, line, marked)).collect();
        // Two places one name could be read as one place said twice: then
        // the paths say which.
        let twice = names.iter().enumerate().any(|(at, name)| names[..at].contains(name));
        let named: Vec<String> = most
            .iter()
            .zip(names)
            .map(|(line, name)| {
                let name = if twice && !line.loose {
                    self.path(Classified::entry_of(line), marked)
                } else {
                    name
                };
                format!("{name} {}", words::bytes(line.whole))
            })
            .collect();
        if !named.is_empty() {
            let _ = write!(text, ": {}", named.join(", "));
        }
        text.push('.');
        if share.frees.allocated > 0 {
            let _ = write!(text, " A tool makes {} of it again.", words::bytes(share.frees.allocated));
        }
        if consumer == Consumer::VirtualDisks {
            text.push_str(&self.compacting(share));
        }
        text
    }

    /// What compacting the virtual disks gives back, as a sentence after
    /// theirs: what was read inside them, or that nothing could be.
    fn compacting(&self, share: &Share) -> String {
        if self.disks.is_empty() {
            return String::new();
        }
        let them = if self.disks.len() == 1 { "it" } else { "them" };
        if self.disks.iter().all(|disk| disk.told.gives_back.is_none()) {
            return format!(" midda cannot see inside {them}.");
        }
        match vdisk::gives_back(self.disks, share.size.allocated) {
            Some(back) => format!(" Compacting {them} gives back about {}.", words::bytes(back)),
            None => format!(" Compacting {them} would give back next to nothing."),
        }
    }

    /// What compacting the virtual disks gives back, when it is worth saying.
    fn compacts(&self, share: &Share) -> Option<u64> {
        (share.consumer == Consumer::VirtualDisks)
            .then(|| vdisk::gives_back(self.disks, share.size.allocated))
            .flatten()
    }

    /// The places a reason names: the largest that hold a notable part of
    /// it, none inside another.
    fn most<'s>(&self, share: &'s Share) -> Vec<&'s Line> {
        let mut said: Vec<&Line> = Vec::new();
        for line in &share.lines {
            if said.len() == PLACES_SAID || line.whole.saturating_mul(NOTABLE) < share.size.allocated {
                break;
            }
            if !said.iter().any(|other| self.nested(line, other)) {
                said.push(line);
            }
        }
        said
    }

    /// Whether one of two lines is inside the other.
    fn nested(&self, a: &Line, b: &Line) -> bool {
        if a.loose || b.loose {
            return false;
        }
        let above = |inner: NodeId, outer: NodeId| {
            let mut at = inner;
            while at != ROOT {
                at = self.tree.node(at).parent;
                if at == outer {
                    return true;
                }
            }
            false
        };
        above(a.id, b.id) || above(b.id, a.id)
    }

    /// What a reader calls a line: the place it is, when it is one; the
    /// places of the root's own files; or the path.
    fn name(&self, consumer: Consumer, line: &Line, marked: bool) -> String {
        match self.classified.name_of(self.rules, self.tree, consumer, line) {
            Some(name) => name,
            None if line.loose => format!("files directly in {}", self.path(ROOT, marked)),
            None => self.path(line.id, marked),
        }
    }

    /// The path of `id`; with `marked`, as Markdown code.
    fn path(&self, id: NodeId, marked: bool) -> String {
        let path = self.tree.path_of(id).display().to_string();
        if marked { format!("`{path}`") } else { path }
    }

    /// The paragraph and every consumer's places, as Markdown.
    fn brief(&self) -> String {
        let total = self.summary.total.allocated;
        let mut brief = format!("**What fills {}**, on disk\n\n{}\n", self.root(), self.paragraph(true));
        for share in &self.summary.shares {
            let size = share.size.allocated;
            let _ = write!(
                brief,
                "\n- **{}** {} ({})",
                share.consumer.label(),
                words::bytes(size),
                words::share(size, total)
            );
            if share.frees.allocated > 0 {
                let _ = write!(brief, "; a tool makes {} of it again", words::bytes(share.frees.allocated));
            }
            if let Some(back) = self.compacts(share) {
                let _ = write!(brief, "; compacting gives back about {}", words::bytes(back));
            }
            for line in share.lines.iter().take(PLACES_LISTED) {
                let path = format!("`{}`", self.tree.path_of(Classified::entry_of(line)).display());
                let what = match (self.classified.name_of(self.rules, self.tree, share.consumer, line), line.loose) {
                    (Some(name), true) => format!("{name} in {path}"),
                    (None, true) => format!("files directly in {path}"),
                    (Some(name), false) => format!("{name} {path}"),
                    (None, false) => path,
                };
                let _ = write!(brief, "\n  - {what} {}", words::bytes(line.whole));
            }
            let more = share.lines.len().saturating_sub(PLACES_LISTED);
            if more > 0 {
                let _ = write!(brief, "\n  - {more} more {}", if more == 1 { "place" } else { "places" });
            }
        }
        brief.push('\n');
        brief
    }
}

fn count_in_words(count: usize) -> &'static str {
    ["no", "one", "two", "three", "four", "five"].get(count).copied().unwrap_or("several")
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}
