//! What fills a disk, and who for: every byte of a tree given to one
//! consumer — Windows, programs, games, development, browsers, virtual
//! disks, media, documents, or what midda does not recognise — and the few
//! places in each that hold most of it.
//!
//! A picture of a volume says what is large; "C: is full" asks *why*, and
//! the answer a person can act on is a handful of reasons — games take a
//! third of it, development another third, Windows a tenth — not a million
//! entries. See `docs/adr/0014-what-fills-the-disk.md`.
//!
//! # Where a byte goes
//!
//! The rules files say what things are ([`crate::rules::Place`],
//! [`crate::rules::FileKind`]). A walk from the root gives each file one
//! consumer, the first of these that has an answer:
//!
//! 1. A folder a rule finds or closes — a `node_modules`, a Cargo `target` —
//!    is its group's consumer whole, whatever place it is in: what is inside
//!    it is not looked at.
//! 2. The deepest place that names the file or a folder above it — Steam's
//!    library inside Program Files is games, Docker's disk inside Docker's
//!    folder a virtual disk.
//! 3. What the file is by its own name, a video or a source file — but only
//!    where no place above it says otherwise: a video inside a game is the
//!    game's.
//! 4. A soft place above it: a file in Documents whose name says nothing is a
//!    document.
//! 5. Nothing: midda does not recognise it, and says so rather than guess.
//!
//! Every byte the tree counts is given once, so the consumers add up to the
//! root ([`Classified::total`]).
//!
//! # Where in a consumer
//!
//! "Games take 92 GB" wants "the Steam library is 90 GB of it". The lines of
//! a consumer are the folders that hold most of it, each named as deep as it
//! is one sizeable thing and counted once — the same rule that names who is
//! eating the disk ([`crate::growth`]), asked of one consumer's bytes rather
//! than of growth. A folder a rule claims is named whole.

pub mod story;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::growth;
use crate::rules::pattern::Outer;
use crate::rules::{Claim, RuleSet, Survey};
use crate::size::{Size, SizeBasis};
use crate::tree::{NodeId, ROOT, Tree};

/// Who the bytes are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Consumer {
    /// Windows and what it keeps beside it: the page and hibernation files,
    /// restore points, updates, temporary files.
    System,
    /// Installed programs and the data they keep.
    Programs,
    /// Game libraries, saves and shader caches.
    Games,
    /// Toolchains, packages, build output and repositories.
    Development,
    /// Browser profiles and caches.
    Browsers,
    /// WSL, Docker and virtual machines: each one file with a whole disk
    /// inside ([`crate::vdisk`]).
    VirtualDisks,
    /// Video, audio, images and archives.
    Media,
    /// Documents, text and code.
    Documents,
    /// What nothing names.
    Unrecognised,
}

impl Consumer {
    /// Every consumer, in the order a tie between two is broken.
    pub const ALL: [Self; 9] = [
        Self::System,
        Self::Programs,
        Self::Games,
        Self::Development,
        Self::Browsers,
        Self::VirtualDisks,
        Self::Media,
        Self::Documents,
        Self::Unrecognised,
    ];

    /// How a rules file writes it.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Programs => "programs",
            Self::Games => "games",
            Self::Development => "development",
            Self::Browsers => "browsers",
            Self::VirtualDisks => "virtual-disks",
            Self::Media => "media",
            Self::Documents => "documents",
            Self::Unrecognised => "unrecognised",
        }
    }

    /// What a reader calls it, at the start of a sentence.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "Windows",
            Self::Programs => "Programs",
            Self::Games => "Games",
            Self::Development => "Development",
            Self::Browsers => "Browsers",
            Self::VirtualDisks => "Virtual disks",
            Self::Media => "Media and archives",
            Self::Documents => "Documents and code",
            Self::Unrecognised => "Files midda does not recognise",
        }
    }

    /// What is in it, in a line.
    #[must_use]
    pub const fn about(self) -> &'static str {
        match self {
            Self::System => "The system, its page and hibernation files, restore points, updates and temporary files.",
            Self::Programs => "Installed programs and the data they keep for you.",
            Self::Games => "Game libraries, saves and shader caches.",
            Self::Development => "Toolchains, packages, build output and repositories.",
            Self::Browsers => "Browser profiles and their caches.",
            Self::VirtualDisks => {
                "WSL, Docker and virtual machines: each one file that grows as the machine inside writes, and does not shrink when it deletes."
            }
            Self::Media => "Video, audio, images and archives.",
            Self::Documents => "Documents, text and code.",
            Self::Unrecognised => "What no place or name midda knows says anything about.",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }

    /// The consumer at `index` in [`Consumer::ALL`].
    fn at(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }
}

/// A folder's bytes are not all one consumer's.
const MIXED: u8 = u8::MAX;

/// A line is a place when it and the place hold the same bytes to within one
/// part in this many: a tenth.
const NEARLY: u64 = 10;

/// What a walk carries down: who the place above says the bytes are for.
#[derive(Debug, Clone, Copy, Default)]
struct Above {
    /// A place that decides.
    firm: Option<Consumer>,
    /// A soft place: what a file's own name does not say.
    soft: Option<Consumer>,
}

impl Above {
    fn of(place: &crate::rules::Place) -> Self {
        if place.yields {
            Self {
                firm: None,
                soft: Some(place.consumer),
            }
        } else {
            Self {
                firm: Some(place.consumer),
                soft: None,
            }
        }
    }
}

/// A tree's bytes, each given to one consumer.
#[derive(Debug, Clone)]
pub struct Classified {
    /// Per node: the consumer of a file, and of a folder a rule claims
    /// whole; [`MIXED`] for any other folder.
    of: Vec<u8>,
    /// The folders, each before what is in it, nothing under a folder a rule
    /// claims: walked once here, and asked of every consumer.
    folders: growth::Folders,
    /// Where in `folders` each folder is, by its id; `u32::MAX` for anything
    /// else.
    position: Vec<u32>,
    /// The folders a rule claims whole.
    units: HashSet<NodeId>,
    /// The entries a place names: the place's index in the set, and whom it
    /// says they are for.
    named: HashMap<NodeId, (usize, Consumer)>,
    /// What each consumer holds, by [`Consumer::index`].
    totals: [Size; 9],
    /// What deleting the findings of each consumer would free.
    frees: [Size; 9],
}

/// Gives every byte of `tree` to one consumer, by `rules` and what they
/// found in it.
#[must_use]
pub fn classify(rules: &RuleSet, tree: &Tree, survey: &Survey) -> Classified {
    let slots = tree.nodes().map(|(id, _)| id as usize + 1).max().unwrap_or(1);
    let mut classified = Classified {
        of: vec![MIXED; slots],
        folders: growth::Folders::default(),
        position: Vec::new(),
        units: HashSet::new(),
        named: HashMap::new(),
        totals: [Size::zero(); 9],
        frees: [Size::zero(); 9],
    };
    let claims: HashMap<NodeId, Claim> = survey.claims().map(|claim| (claim.id(), claim)).collect();
    let outer = Outer::of(tree.root_path());
    let mut scratch = String::new();

    // The root is what was opened; a place may still name it - a scan of
    // `C:\Windows` is Windows throughout.
    let root_place = rules.place_of(tree, ROOT, &outer, &mut scratch);
    if let Some(place) = root_place {
        classified.named.insert(ROOT, (place, rules.places()[place].consumer));
    }
    let mut stack = vec![(ROOT, root_place.map(|place| Above::of(&rules.places()[place])).unwrap_or_default())];
    while let Some((id, above)) = stack.pop() {
        for &child in &tree.node(id).children {
            let node = tree.node(child);
            if let Some(&claim) = claims.get(&child) {
                let consumer = rules.consumer_of(claim);
                classified.give(child, consumer, node.size);
                classified.units.insert(child);
                if let Claim::Found(finding) = claim {
                    classified.frees[consumer.index()].add(finding.frees);
                }
                continue;
            }
            let here = match rules.place_of(tree, child, &outer, &mut scratch) {
                Some(place) => {
                    classified.named.insert(child, (place, rules.places()[place].consumer));
                    Above::of(&rules.places()[place])
                }
                None => above,
            };
            if node.is_directory() {
                stack.push((child, here));
            } else {
                let consumer = here
                    .firm
                    .or_else(|| rules.kind_of(&node.name, &mut scratch))
                    .or(here.soft)
                    .unwrap_or(Consumer::Unrecognised);
                classified.give(child, consumer, node.size);
            }
        }
    }
    classified.folders = growth::Folders::of(tree, |id| classified.units.contains(&id));
    classified.position = vec![u32::MAX; slots];
    for (at, &id) in classified.folders.folders().iter().enumerate() {
        classified.position[id as usize] = u32::try_from(at).expect("fewer folders than entries");
    }
    classified
}

impl Classified {
    fn give(&mut self, id: NodeId, consumer: Consumer, size: Size) {
        #[allow(clippy::cast_possible_truncation, reason = "nine consumers")]
        let index = consumer.index() as u8;
        self.of[id as usize] = index;
        self.totals[consumer.index()].add(size);
    }

    /// The consumer of a file, or of a folder a rule claims whole; `None` for
    /// a folder whose bytes are several consumers', and for an entry the walk
    /// did not reach.
    #[must_use]
    pub fn of(&self, id: NodeId) -> Option<Consumer> {
        self.of.get(id as usize).copied().and_then(Consumer::at)
    }

    /// What `consumer` holds.
    #[must_use]
    pub fn total(&self, consumer: Consumer) -> Size {
        self.totals[consumer.index()]
    }

    /// What deleting the findings that are `consumer`'s would free.
    #[must_use]
    pub fn frees(&self, consumer: Consumer) -> Size {
        self.frees[consumer.index()]
    }

    /// The place that names `id`, by its index in the set's places.
    #[must_use]
    pub fn place(&self, id: NodeId) -> Option<usize> {
        self.named.get(&id).map(|&(place, _)| place)
    }

    /// Every entry a place names, with the place's index in the set.
    pub fn places(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.named.iter().map(|(&id, &(place, _))| (id, place))
    }

    /// The place that names `id` as `consumer`'s.
    fn place_for(&self, id: NodeId, consumer: Consumer) -> Option<usize> {
        self.named.get(&id).filter(|&&(_, whose)| whose == consumer).map(|&(place, _)| place)
    }

    /// The place a line is, when one is: the nearest place of its consumer
    /// at or above it that the line holds nearly all of - Chrome's profile
    /// folder over the one profile in it - or, below it, the place its bytes
    /// nearly all are - Docker's disk in the folder that holds only it.
    fn named_line(&self, tree: &Tree, line: NodeId, consumer: Consumer, held: &impl Fn(NodeId) -> u64) -> Option<Named> {
        let line_holds = held(line);
        let nearly = |part: u64, whole: u64| part.saturating_mul(NEARLY) >= whole.saturating_mul(NEARLY - 1);
        let mut at = line;
        loop {
            if let Some(place) = self.place_for(at, consumer) {
                if nearly(line_holds, held(at)) {
                    return Some(Named { id: at, place });
                }
                break;
            }
            if at == ROOT {
                break;
            }
            at = tree.node(at).parent;
        }
        let mut at = line;
        while !self.units.contains(&at) {
            let biggest = tree.node(at).children.iter().copied().max_by_key(|&child| held(child))?;
            if !nearly(held(biggest), line_holds) {
                return None;
            }
            at = biggest;
            if let Some(place) = self.place_for(at, consumer) {
                return Some(Named { id: at, place });
            }
        }
        None
    }

    /// The folders a rule claims whole.
    #[must_use]
    pub const fn units(&self) -> &HashSet<NodeId> {
        &self.units
    }

    /// What a reader calls a line of `consumer`: the place it is, when it is
    /// one; for the root's own files, the places that hold a notable part of
    /// them when a place names every one, the largest first - "the page file
    /// and the hibernation file", and not the twelve kilobytes of a crash
    /// log beside them. `None` when no place says, and the path is its name.
    #[must_use]
    pub fn name_of(&self, rules: &RuleSet, tree: &Tree, consumer: Consumer, line: &Line) -> Option<String> {
        if !line.loose {
            return line.named.map(|named| rules.places()[named.place].name.clone());
        }
        let mut files: Vec<(u64, usize)> = Vec::new();
        let mut loose = 0_u64;
        for &child in &tree.root().children {
            let node = tree.node(child);
            if node.is_directory() || self.of(child) != Some(consumer) {
                continue;
            }
            let place = self.place_for(child, consumer)?;
            loose += node.size.allocated;
            match files.iter_mut().find(|(_, held)| *held == place) {
                Some((bytes, _)) => *bytes += node.size.allocated,
                None => files.push((node.size.allocated, place)),
            }
        }
        files.retain(|&(bytes, _)| bytes.saturating_mul(NEARLY) >= loose);
        files.sort_by_key(|&(bytes, _)| std::cmp::Reverse(bytes));
        let names: Vec<&str> = files.iter().map(|&(_, place)| rules.places()[place].name.as_str()).collect();
        match names.as_slice() {
            [] => None,
            [one] => Some((*one).to_owned()),
            [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
        }
    }

    /// The entry a line stands for: the place it is, the root for the root's
    /// own files, or its folder.
    #[must_use]
    pub fn entry_of(line: &Line) -> NodeId {
        match (line.loose, line.named) {
            (true, _) => ROOT,
            (false, Some(named)) => named.id,
            (false, None) => line.id,
        }
    }

    /// Each consumer that holds anything, the most on `basis` first, with
    /// the places that hold most of it.
    #[must_use]
    pub fn summary(&self, tree: &Tree, basis: SizeBasis) -> Summary {
        // What of each consumer is under each folder, in one walk: a folder
        // a rule claims holds its own, any other the files directly in it,
        // and each hands what it holds on to the folder above, the deepest
        // first.
        let folders = self.folders.folders();
        let mut sums = vec![[0_u64; 9]; folders.len()];
        for (at, &id) in folders.iter().enumerate() {
            if self.folders.units()[at] {
                if let Some(consumer) = self.of(id) {
                    sums[at][consumer.index()] = basis.of(tree.node(id).size);
                }
                continue;
            }
            for &child in &tree.node(id).children {
                let node = tree.node(child);
                if let Some(consumer) = self.of(child).filter(|_| !node.is_directory()) {
                    sums[at][consumer.index()] += basis.of(node.size);
                }
            }
        }
        for at in (1..folders.len()).rev() {
            let here = sums[at];
            let parent = &mut sums[self.folders.parents()[at] as usize];
            for (into, from) in parent.iter_mut().zip(here) {
                *into = into.saturating_add(from);
            }
        }
        let mut shares: Vec<Share> = Consumer::ALL
            .into_iter()
            .filter(|&consumer| basis.of(self.total(consumer)) > 0)
            .map(|consumer| self.share(tree, consumer, basis, &sums))
            .collect();
        shares.sort_by(|a, b| basis.of(b.size).cmp(&basis.of(a.size)).then(a.consumer.cmp(&b.consumer)));
        Summary {
            basis,
            total: tree.root().size,
            shares,
        }
    }

    /// One consumer, and the lines of it: the folders that hold most of it.
    fn share(&self, tree: &Tree, consumer: Consumer, basis: SizeBasis, sums: &[[u64; 9]]) -> Share {
        let index = consumer.index();
        // What of `consumer` an entry holds: a folder's from the sums, a
        // file's its own when it is the consumer's.
        let held = |id: NodeId| match self.position.get(id as usize).copied().filter(|&at| at != u32::MAX) {
            Some(at) => sums[at as usize][index],
            None if self.of(id) == Some(consumer) => basis.of(tree.node(id).size),
            None => 0,
        };
        let signed = |bytes: u64| i64::try_from(bytes).unwrap_or(i64::MAX);
        let found = self
            .folders
            .growers(basis.of(self.total(consumer)), |at| signed(sums[at][index]), |file| signed(held(file)));
        let mut lines: Vec<Line> = found
            .growers
            .into_iter()
            .map(|line| Line {
                id: line.id,
                own: line.own.unsigned_abs(),
                whole: line.growth.unsigned_abs(),
                inside: line.inside,
                loose: line.loose,
                named: if line.loose { None } else { self.named_line(tree, line.id, consumer, &held) },
            })
            .collect();
        // A place is said once: a line inside a folder that is a line itself
        // keeps its own path, rather than the name the folder above carries.
        let ids: HashSet<NodeId> = lines.iter().filter(|line| !line.loose).map(|line| line.id).collect();
        for line in &mut lines {
            if line.named.is_some_and(|named| named.id != line.id && ids.contains(&named.id)) {
                line.named = None;
            }
        }
        // What each holds, the most first: a folder and a line inside it are
        // both true, and the path says which is inside which.
        lines.sort_by(|a, b| {
            b.whole
                .cmp(&a.whole)
                .then_with(|| a.loose.cmp(&b.loose))
                .then_with(|| tree.scan_order(a.id, b.id))
        });
        Share {
            consumer,
            size: self.total(consumer),
            frees: self.frees(consumer),
            lines,
            rest: found.rest.unsigned_abs(),
            threshold: found.threshold,
        }
    }
}

/// What fills a tree: each consumer that holds anything, the most first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// The size the shares are ordered and their places measured on.
    pub basis: SizeBasis,
    /// What the root holds: what the shares add up to.
    pub total: Size,
    pub shares: Vec<Share>,
}

/// One consumer of a tree, and where its bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub consumer: Consumer,
    /// What it holds.
    pub size: Size,
    /// What deleting the findings that are its would free: what of it a
    /// tool makes again.
    pub frees: Size,
    /// The folders that hold most of it, the most first by what each holds
    /// of it - a line inside another included in the other's.
    pub lines: Vec<Line>,
    /// What no line names: pieces each under the threshold.
    pub rest: u64,
    /// What a folder had to hold on its own to be a line.
    pub threshold: u64,
}

/// A folder that holds a sizeable part of one consumer's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub id: NodeId,
    /// What of the consumer it holds apart from the lines inside it: what
    /// the lines add up by.
    pub own: u64,
    /// What of the consumer it holds, the lines inside included.
    pub whole: u64,
    /// How many lines are inside it.
    pub inside: u32,
    /// The line is the files directly in the root, not a folder.
    pub loose: bool,
    /// The place the line is, when one is.
    pub named: Option<Named>,
}

/// A place a line is: the entry it names, which can be the line's folder,
/// one above it or one inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Named {
    pub id: NodeId,
    /// The place, by its index in the set's places.
    pub place: usize,
}

#[cfg(test)]
mod tests;
