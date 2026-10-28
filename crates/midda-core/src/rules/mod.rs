//! What on a disk a tool makes again: rules that find it, and say how it
//! comes back and what is lost.
//!
//! A picture of a disk says what is large. A rule says what is not worth
//! keeping: this `node_modules` is installed from the `package.json` beside
//! it, and the lockfile beside that pins every version, so `pnpm install`
//! brings back exactly what is there. That sentence is the product (Vision
//! §2): without it a large folder is a question, with it it is a decision.
//!
//! # A rule
//!
//! - **Where**: a path read from its end ([`pattern::Template`]) —
//!   `node_modules` anywhere, `AppData/Local/npm-cache` in any profile.
//! - **Beside** and **inside**: a folder counts only when something that
//!   explains it is next to it or in it. A folder called `target` is a Rust
//!   build only with a `Cargo.toml` beside it; without one it is somebody's
//!   folder called `target`.
//! - **How it comes back**, and how careful to be: each rule has ways back,
//!   tried in order, and the last one is taken when no earlier one applies. A
//!   `node_modules` with a lockfile beside it is safe; without one an install
//!   may pick newer versions, and the rule says so.
//! - **A cleanup**, for a cache whose own tool cleans it better than deleting
//!   the folder would ([`cleanup`]).
//!
//! - **Closed**: a folder whose name alone says what is inside it — a
//!   `node_modules` holds installed packages, whatever is beside it — is
//!   never looked into by another rule, even when nothing beside it lets this
//!   one apply. Otherwise the `dist` of every package in a global npm folder
//!   would be "what the project's build wrote", and deleting it would break
//!   the package.
//!
//! A rule that cannot explain itself cannot be written: a rule with no way
//! back, or whose last way back has a condition, is refused when the set is
//! read. "Can be deleted" with no reason is a guess, and a wrong guess costs
//! the reader their data.
//!
//! # What else a rule can say
//!
//! What Windows and a browser leave behind is not named by what is beside
//! it, and the same format says it (ADR 0016):
//!
//! - **Files** as well as folders (`entries`): a memory dump, an installer.
//! - **Age** (`older`): an entry nothing in which was written for that long —
//!   the age of the layer, [`crate::tree::Node::subtree_modified`]. A time the
//!   volume did not give is not old.
//! - **What goes** (`goes`): the entry, or what is in it. A temporary folder
//!   is emptied, never deleted — a program that finds `%TEMP%` gone does not
//!   make it again — and with an age only what in it is that old goes.
//! - **Empty** (`hollow`): a folder with no file anywhere in it, and nothing
//!   in it the scan could not read.
//! - **Whose** (`within`): only inside what some consumers hold — an empty
//!   folder among the reader's documents, not one a program keeps in
//!   `AppData`. Never a place itself: an empty `Downloads` is not a leftover.
//! - **A check** (`check`): what only the file says — where a shortcut
//!   points, what an installer installs. The survey names the entry; the
//!   check is asked afterwards, away from the tree ([`crate::checks`]), and
//!   says which way back applies (`if`), or that the entry is none of the
//!   rule's — a shortcut that works.
//!
//! # Rules are data
//!
//! The rules midda ships with are files, `rules/dev.toml` and
//! `rules/system.toml`, read by the same code that will read a user's own
//! rules: one format, so whatever the shipped rules can say, a user's can.
//! See `docs/adr/0012-rules-that-explain-themselves.md`.
//!
//! # Places
//!
//! The same files say what things are without saying they can go: a
//! [`Place`] names a folder or a file — Windows, a Steam library, a
//! browser's profile — and the [`Consumer`] its bytes are for; a
//! [`FileKind`] says what a file is by its name where no place says
//! otherwise. `rules/places.toml` holds the shipped ones, and
//! [`crate::consumers`] gives every byte of a tree to one consumer by them.
//! A folder a rule finds or closes is its group's consumer, when the group
//! says one ([`Group::consumer`]). See `docs/adr/0014-what-fills-the-disk.md`.
//!
//! # A finding
//!
//! The first entry down from the root that a rule names. Nothing under it is
//! looked at: a `node_modules` is one thing, however many `dist` folders its
//! packages hold. Each finding carries two numbers, because deleting a folder
//! does not always give back what it holds:
//!
//! - **holds** — what the tree counts for what goes;
//! - **frees** — what deleting it would give back: every file whose names are
//!   all inside it. A file with a second name elsewhere — a package store
//!   links its files into every project's `node_modules` — keeps its bytes
//!   when one of its names goes. A `node_modules` installed by pnpm can hold
//!   a gigabyte and free a few megabytes, and saying the gigabyte would be the
//!   lie this product exists to stop telling.

pub mod cleanup;
pub mod pattern;
mod places;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub use cleanup::Cleanup;
use pattern::{Glob, Outer, Template};
pub use places::{FileKind, Place};

use crate::checks::{Check, Outcome, Seen};
use crate::consumers::{Above, Consumer};
use crate::platform::FileIdentity;
use crate::size::Size;
use crate::tree::{NodeId, ROOT, Tree};
use crate::words;

/// Where a finding is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Group {
    /// Packages installed for a project: `node_modules`.
    Dependencies,
    /// What a build wrote: `target`, `.next`, `dist`, bytecode.
    Build,
    /// What a package manager downloaded once for every project.
    Caches,
    /// What was deleted and waits in the Recycle Bin.
    RecycleBin,
    /// What programs left in a temporary folder.
    Temporary,
    /// The reports and memory dumps Windows keeps of what crashed.
    CrashReports,
    /// What was downloaded: installers, disc images, unfinished and old
    /// downloads.
    Downloads,
    /// Empty folders and shortcuts to nothing.
    Leftovers,
}

impl Group {
    /// Every group, in the order a list of findings shows them: what a
    /// project installed, built and shares, then what Windows and the
    /// reader's downloads leave behind.
    pub const ALL: [Self; 8] = [
        Self::Dependencies,
        Self::Build,
        Self::Caches,
        Self::RecycleBin,
        Self::Temporary,
        Self::CrashReports,
        Self::Downloads,
        Self::Leftovers,
    ];

    /// Who the entries of this group are for, when the group says.
    ///
    /// A group belongs to one consumer, so a rule does not say its own: a
    /// rule that could say `group = "caches"` and `consumer = "games"` would
    /// be two answers to one question. A group a game's cache needs is a
    /// group of its own.
    ///
    /// Some groups say nothing: an old download is what it is — a video in
    /// Downloads is media whether or not it is old — and an empty folder or
    /// a shortcut holds next to nothing. Their findings claim no bytes, and
    /// the places and names say whose they are ([`crate::consumers`]).
    #[must_use]
    pub const fn consumer(self) -> Option<Consumer> {
        match self {
            Self::Dependencies | Self::Build | Self::Caches => Some(Consumer::Development),
            Self::RecycleBin | Self::Temporary | Self::CrashReports => Some(Consumer::System),
            Self::Downloads | Self::Leftovers => None,
        }
    }
}

/// How careful to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    /// Nothing anyone needs is lost: it comes back by itself or with one
    /// command, as it was — or nothing uses it any more.
    Safe,
    /// It may not come back as it was, or may hold something nobody can make
    /// again. Look first.
    Caution,
}

/// Which entries a rule names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Entries {
    #[default]
    Folders,
    Files,
    All,
}

impl Entries {
    const fn admits(self, folder: bool) -> bool {
        match self {
            Self::Folders => folder,
            Self::Files => !folder,
            Self::All => true,
        }
    }
}

/// What deleting a rule's entry takes, as a rule writes it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Takes {
    /// The entry itself.
    #[default]
    Whole,
    /// What is in the folder; the folder stays.
    Contents,
}

/// One way an entry comes back, or why nothing needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restore {
    /// Taken when an entry by this name is beside the folder.
    when: Option<Glob>,
    /// Taken when the rule's check says this ([`crate::checks`]). Neither
    /// condition on the last way, which is taken when no other is.
    outcome: Option<Outcome>,
    /// What to run beside the folder to make it again, when one command does.
    pub command: Option<String>,
    pub risk: Risk,
    /// What the entry is, how it comes back and what is lost: one or two
    /// sentences, said to the reader as they are.
    pub explain: String,
}

/// A rule. See the module documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// What the rule is called in a file and in an answer: kebab-case,
    /// unique in its set.
    pub id: String,
    /// What a reader calls the thing it finds: `node_modules`, `Cargo build
    /// output`.
    pub name: String,
    pub group: Group,
    paths: Vec<Template>,
    beside: Vec<Glob>,
    inside: Vec<Glob>,
    /// Nothing inside a folder its paths name is looked at by any rule,
    /// whether or not this one applies to it.
    closed: bool,
    entries: Entries,
    /// How long nothing in an entry has been written, at least.
    older: Option<Duration>,
    takes: Takes,
    /// Only a folder with no file in it.
    hollow: bool,
    /// Only strictly inside what these consumers hold; empty for anywhere.
    within: Vec<Consumer>,
    /// Paths the rule never names, and that never go with a folder it
    /// empties: `desktop.ini` says how the shell shows its folder.
    except: Vec<Template>,
    /// What only the file can say, asked after the survey.
    pub check: Option<Check>,
    restore: Vec<Restore>,
    pub cleanup: Option<Cleanup>,
}

/// What a walk knows of the folder an entry is in.
#[derive(Debug, Clone, Copy)]
struct Here {
    now: SystemTime,
    /// Who the place the entry is in says its bytes are for.
    above: Above,
}

impl Rule {
    /// The ways back, the one taken when nothing else applies last.
    #[must_use]
    pub fn restores(&self) -> &[Restore] {
        &self.restore
    }

    /// The paths it names, as written.
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        self.paths.iter().map(Template::as_string).collect()
    }

    /// Whether `id` is old enough: nothing in it written since `older`
    /// before `now`. Always, when the rule says no age; never, when the
    /// volume gave no time.
    fn old_enough(&self, tree: &Tree, id: NodeId, now: SystemTime) -> bool {
        self.older.is_none_or(|older| {
            tree.node(id)
                .subtree_modified
                .is_some_and(|written| now.duration_since(written).is_ok_and(|age| age >= older))
        })
    }

    /// Whether `id` goes when the rule empties the folder it is in: not one
    /// the rule excepts, and old enough.
    fn goes_with(&self, tree: &Tree, id: NodeId, outer: &Outer, now: SystemTime) -> bool {
        !self.excepts(tree, id, outer) && self.old_enough(tree, id, now)
    }

    fn excepts(&self, tree: &Tree, id: NodeId, outer: &Outer) -> bool {
        self.except.iter().any(|path| path.matches(tree, id, outer))
    }

    /// Whether the rule names `id`, whichever way back then applies: its
    /// paths, what is beside and in it, its age, whose it is, and — for a
    /// hollow rule — that nothing is in it. `place` says whether a place
    /// names the entry itself, asked only when the rule needs to know.
    fn names(&self, tree: &Tree, id: NodeId, outer: &Outer, here: Here, place: impl FnOnce() -> bool, hollow: impl FnOnce() -> bool) -> bool {
        let folder = tree.node(id).is_directory();
        self.entries.admits(folder)
            && !self.excepts(tree, id, outer)
            && self.paths.iter().any(|path| path.matches(tree, id, outer))
            && explained(tree, id, &self.beside, &self.inside)
            && (self.takes == Takes::Contents || self.old_enough(tree, id, here.now))
            && (self.within.is_empty() || (self.within.contains(&here.above.consumer()) && !place()))
            && (!self.hollow || hollow())
    }

    /// Which way back applies to `id` by what is beside it. A rule with a
    /// check has none yet: its check says.
    fn way_beside(&self, tree: &Tree, id: NodeId) -> usize {
        let siblings = &tree.node(tree.node(id).parent).children;
        self.restore
            .iter()
            .position(|way| way.when.as_ref().is_none_or(|glob| has(tree, siblings, glob)))
            .unwrap_or(self.restore.len() - 1)
    }

    /// Which way back a check's answer takes.
    fn way_seen(&self, outcome: Option<Outcome>) -> usize {
        self.restore
            .iter()
            .position(|way| way.outcome.is_none() || way.outcome == outcome)
            .unwrap_or(self.restore.len() - 1)
    }
}

/// Whether something `beside` names is next to `id`, and something `inside`
/// names is in it — each only when asked for.
fn explained(tree: &Tree, id: NodeId, beside: &[Glob], inside: &[Glob]) -> bool {
    let node = tree.node(id);
    (beside.is_empty() || beside.iter().any(|glob| has(tree, &tree.node(node.parent).children, glob)))
        && (inside.is_empty() || inside.iter().any(|glob| has(tree, &node.children, glob)))
}

/// Whether an entry among `among` is one `glob` names.
fn has(tree: &Tree, among: &[NodeId], glob: &Glob) -> bool {
    among.iter().any(|&other| glob.matches(&tree.node(other).name))
}

/// A set of rules, places and kinds of file, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSet {
    rules: Vec<Rule>,
    places: Vec<Place>,
    files: Vec<FileKind>,
    /// The places by the last name of their paths, so a volume's folders are
    /// not each asked of every place.
    index: places::Index,
    /// The rules that name files, the same way.
    file_rules: FileRules,
}

/// The rules that name files, by the last name of their paths: a volume's
/// million files are each asked of the few rules that could name them —
/// `*.lnk` of a shortcut, every rule for a name no index can sort.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FileRules {
    /// Plain last names, folded to lower case.
    named: HashMap<String, Vec<usize>>,
    /// Last names of the form `*.ext`, by `ext` folded to lower case.
    extended: HashMap<String, Vec<usize>>,
    /// Every other last name.
    wild: Vec<usize>,
}

impl FileRules {
    fn of(rules: &[Rule]) -> Self {
        let mut index = Self::default();
        for (at, rule) in rules.iter().enumerate().filter(|(_, rule)| rule.entries != Entries::Folders) {
            for path in &rule.paths {
                let last = path.last().as_str();
                let slot = if !last.contains(['*', '?']) {
                    index.named.entry(last.to_ascii_lowercase()).or_default()
                } else if let Some(extension) = places::extension_of(last) {
                    index.extended.entry(extension).or_default()
                } else {
                    &mut index.wild
                };
                if slot.last() != Some(&at) {
                    slot.push(at);
                }
            }
        }
        index
    }

    /// The rules that may name a file called `name`, in the set's order.
    fn candidates(&self, name: &str, scratch: &mut String, into: &mut Vec<usize>) {
        into.clear();
        places::fold(name, scratch);
        if let Some(rules) = self.named.get(scratch.as_str()) {
            into.extend_from_slice(rules);
        }
        if let Some(rules) = places::extension_in(name, scratch).and_then(|extension| self.extended.get(extension)) {
            into.extend_from_slice(rules);
        }
        into.extend_from_slice(&self.wild);
        into.sort_unstable();
        into.dedup();
    }
}

/// The files the shipped rules and places are in.
const DEV: &str = include_str!("../../rules/dev.toml");
const SYSTEM: &str = include_str!("../../rules/system.toml");
const PLACES: &str = include_str!("../../rules/places.toml");

impl RuleSet {
    /// The rules and places midda ships with.
    ///
    /// # Panics
    ///
    /// If a shipped file does not read — which a test of this crate rules
    /// out before anything is shipped.
    #[must_use]
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<RuleSet> = OnceLock::new();
        SHIPPED.get_or_init(|| Self::parse_all(&[DEV, SYSTEM, PLACES]).expect("the shipped rules read"))
    }

    /// Reads a set written as `rules/dev.toml` and `rules/places.toml` are.
    ///
    /// # Errors
    ///
    /// When the text is not TOML of that shape, or a rule in it cannot
    /// explain itself: no way back, a last way back with a condition, an
    /// empty sentence, a path that is not a path. The message names the rule
    /// or the place.
    pub fn parse(text: &str) -> Result<Self, String> {
        Self::parse_all(&[text])
    }

    /// Reads several files as one set: an id is unique across all of them,
    /// and a rule or place in an earlier file is tried first.
    ///
    /// # Errors
    ///
    /// As [`RuleSet::parse`], for any of them.
    pub fn parse_all(texts: &[&str]) -> Result<Self, String> {
        let mut seen = HashSet::new();
        let mut once = |id: &str| {
            if seen.insert(id.to_owned()) {
                Ok(())
            } else {
                Err(format!("`{id}` is written twice"))
            }
        };
        let (mut rules, mut places, mut files) = (Vec::new(), Vec::new(), Vec::new());
        for text in texts {
            let file: RawFile = toml::from_str(text).map_err(|error| error.to_string())?;
            for raw in file.rule {
                once(&raw.id)?;
                let id = raw.id.clone();
                rules.push(raw.check().map_err(|why| format!("rule `{id}`: {why}"))?);
            }
            for raw in file.place {
                once(&raw.id)?;
                let id = raw.id.clone();
                places.push(raw.check().map_err(|why| format!("place `{id}`: {why}"))?);
            }
            for raw in file.file {
                files.push(raw.check()?);
            }
        }
        let index = places::Index::of(&places, &files);
        let file_rules = FileRules::of(&rules);
        Ok(Self {
            rules,
            places,
            files,
            index,
            file_rules,
        })
    }

    /// Every rule, in the order they are tried.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Every place, in the order they are tried.
    #[must_use]
    pub fn places(&self) -> &[Place] {
        &self.places
    }

    /// What files are by their names.
    #[must_use]
    pub fn file_kinds(&self) -> &[FileKind] {
        &self.files
    }

    /// The place that names `id`, by its index in [`RuleSet::places`]: the
    /// first in the set's order. `scratch` is room to fold a name's case in,
    /// kept by the caller so a walk of a volume does not allocate per entry.
    #[must_use]
    pub fn place_of(&self, tree: &Tree, id: NodeId, outer: &Outer, scratch: &mut String) -> Option<usize> {
        self.index
            .candidates(&tree.node(id).name, scratch)
            .filter(|&(place, path)| self.places[place].names_by(path, tree, id, outer))
            .map(|(place, _)| place)
            .min()
    }

    /// What a file called `name` is by its name alone, when any kind says.
    #[must_use]
    pub fn kind_of(&self, name: &str, scratch: &mut String) -> Option<Consumer> {
        self.index.kind_of(name, &self.files, scratch)
    }

    /// The rule a finding names.
    ///
    /// # Panics
    ///
    /// If `finding` came from another set.
    #[must_use]
    pub fn rule_of(&self, finding: &Finding) -> &Rule {
        &self.rules[finding.rule]
    }

    /// The way back a finding takes.
    ///
    /// # Panics
    ///
    /// If `finding` came from another set.
    #[must_use]
    pub fn restore_of(&self, finding: &Finding) -> &Restore {
        &self.rules[finding.rule].restore[finding.restore]
    }

    /// The rule that names `id` and what goes with it, when one does: the
    /// first rule in the set's order, among `among` when only those can.
    /// `place` says whether a place names `id` itself.
    fn naming(&self, tree: &Tree, id: NodeId, walk: &Walk<'_>, here: Here, place: &dyn Fn() -> bool, among: Option<&[usize]>) -> Option<(usize, Vec<NodeId>)> {
        let named = |index: usize| {
            let rule = &self.rules[index];
            if !rule.names(tree, id, &walk.outer, here, place, || walk.hollow(tree, id)) {
                return None;
            }
            let going = match rule.takes {
                Takes::Whole => vec![id],
                Takes::Contents => {
                    let going: Vec<NodeId> = tree
                        .node(id)
                        .children
                        .iter()
                        .copied()
                        .filter(|&child| rule.goes_with(tree, child, &walk.outer, here.now))
                        .collect();
                    // An emptied folder with nothing old enough in it is not
                    // this rule's, and another may still name what is in it.
                    if going.is_empty() {
                        return None;
                    }
                    going
                }
            };
            Some((index, going))
        };
        match among {
            Some(among) => among.iter().copied().find_map(named),
            None => (0..self.rules.len()).find_map(named),
        }
    }

    /// The closed rule whose path names `id`, when one does: nothing in it is
    /// anybody's.
    fn closing(&self, tree: &Tree, id: NodeId, outer: &Outer) -> Option<usize> {
        self.rules
            .iter()
            .position(|rule| rule.closed && rule.paths.iter().any(|path| path.matches(tree, id, outer)))
    }

    /// What the rules say of `tree` now ([`RuleSet::survey_at`]).
    #[must_use]
    pub fn survey(&self, tree: &Tree) -> Survey {
        self.survey_at(tree, SystemTime::now())
    }

    /// What the rules say of `tree` at `now`: the entries they find, the
    /// most freed first; the closed folders that are not findings; and the
    /// entries a rule names whose check is still to be asked.
    ///
    /// The root is not a finding: it is what was opened, and a scan of a
    /// `node_modules` is a look inside one.
    #[must_use]
    pub fn survey_at(&self, tree: &Tree, now: SystemTime) -> Survey {
        let walk = Walk::of(self, tree);
        let mut survey = Survey::default();
        let mut scratch = String::new();
        let outside = if walk.places { self.above_root(&walk.outer) } else { Above::default() };
        let root_above = self.above(tree, ROOT, &walk, outside, &mut scratch);
        let mut stack: Vec<(NodeId, Above)> = Vec::new();
        let mut among = Vec::new();
        self.look_in(tree, ROOT, &walk, Here { now, above: root_above }, &mut survey, &mut stack, &mut among);
        while let Some((id, above)) = stack.pop() {
            let here = Here { now, above };
            // Asked once of each folder: whether it is a place itself, and
            // what it says of what is in it.
            let place = if walk.places {
                self.place_of(tree, id, &walk.outer, &mut scratch)
            } else {
                None
            };
            match self.naming(tree, id, &walk, here, &|| place.is_some(), None) {
                Some((rule, going)) => self.found(tree, id, rule, &going, &mut survey),
                None => match self.closing(tree, id, &walk.outer) {
                    Some(rule) => survey.closed.push(Closed {
                        id,
                        rule,
                        group: self.rules[rule].group,
                    }),
                    None => {
                        let inner = place.map_or(above, |place| Above::of(&self.places[place]));
                        self.look_in(tree, id, &walk, Here { now, above: inner }, &mut survey, &mut stack, &mut among);
                    }
                },
            }
        }
        survey.findings.sort_by(|a, b| {
            b.frees
                .allocated
                .cmp(&a.frees.allocated)
                .then_with(|| b.holds.allocated.cmp(&a.holds.allocated))
                .then_with(|| tree.scan_order(a.id, b.id))
        });
        survey
    }

    /// What the folders above the root of a scan say of it: a scan of a
    /// folder in `AppData` is a program's, though `AppData` is not in the
    /// tree. From the top of the volume down, the deepest place wins.
    fn above_root(&self, outer: &Outer) -> Above {
        (1..=outer.above_root()).rev().fold(Above::default(), |above, up| {
            self.places.iter().find(|place| place.names_above(outer, up)).map_or(above, Above::of)
        })
    }

    /// What a walk carries into `id` from `above`: the place that names it,
    /// when the set needs to know.
    fn above(&self, tree: &Tree, id: NodeId, walk: &Walk<'_>, above: Above, scratch: &mut String) -> Above {
        if !walk.places {
            return above;
        }
        self.place_of(tree, id, &walk.outer, scratch)
            .map_or(above, |place| Above::of(&self.places[place]))
    }

    /// The files directly in `id` a rule names, as findings; its folders onto
    /// `stack`, each with what the walk knows of where it is. `among` is
    /// room for the rules a file's name lets name it.
    #[allow(clippy::too_many_arguments, reason = "one step of one walk, and the room it reuses")]
    fn look_in(&self, tree: &Tree, id: NodeId, walk: &Walk<'_>, here: Here, survey: &mut Survey, stack: &mut Vec<(NodeId, Above)>, among: &mut Vec<usize>) {
        let mut scratch = String::new();
        for &child in &tree.node(id).children {
            if tree.node(child).is_directory() {
                stack.push((child, here.above));
            } else if walk.files {
                self.file_rules.candidates(&tree.node(child).name, &mut scratch, among);
                if among.is_empty() {
                    continue;
                }
                let place = || walk.places && self.place_of(tree, child, &walk.outer, &mut String::new()).is_some();
                if let Some((rule, going)) = self.naming(tree, child, walk, here, &place, Some(among)) {
                    self.found(tree, child, rule, &going, survey);
                }
            }
        }
    }

    /// Records that `rule` names `id`, with `going` what deleting it takes:
    /// a finding, or an entry whose check is still to be asked.
    fn found(&self, tree: &Tree, id: NodeId, rule: usize, going: &[NodeId], survey: &mut Survey) {
        let spec = &self.rules[rule];
        let mut holds = Size::zero();
        let mut entries = 0;
        let mut newest: Option<SystemTime> = None;
        for &entry in going {
            let node = tree.node(entry);
            holds.add(node.size);
            entries += node.entries;
            newest = newest.max(node.subtree_modified);
        }
        let goes = match spec.takes {
            Takes::Whole => Goes::Whole,
            Takes::Contents => Goes::Contents {
                taken: going.len() as u64,
                of: tree.node(id).children.len() as u64,
            },
        };
        let finding = Finding {
            id,
            rule,
            restore: spec.way_beside(tree, id),
            group: spec.group,
            holds,
            frees: freed_by(tree, going),
            entries,
            newest,
            goes,
        };
        match spec.check {
            None => survey.findings.push(finding),
            Some(check) => survey.waiting.push(Waiting {
                path: tree.path_of(id),
                check,
                finding,
            }),
        }
    }

    /// What a check's answer makes of an entry that waited for it: a finding
    /// with the way back the answer takes, or nothing — a shortcut that works
    /// is none of the broken-shortcut rule's.
    ///
    /// # Panics
    ///
    /// If `waiting` came from another set.
    #[must_use]
    pub fn settle(&self, waiting: &Waiting, seen: &Seen) -> Option<Finding> {
        let rule = &self.rules[waiting.finding.rule];
        let outcome = seen.outcome()?;
        Some(Finding {
            restore: rule.way_seen(outcome),
            ..waiting.finding
        })
    }

    /// What a reader is told of a finding beside its rule's sentence: what
    /// its check saw, or how much of a folder that is emptied goes when only
    /// the old part of it does.
    ///
    /// # Panics
    ///
    /// If `finding` came from another set.
    #[must_use]
    pub fn detail(&self, finding: &Finding, seen: Option<&Seen>) -> Option<String> {
        if let Some(seen) = seen {
            return seen.about();
        }
        match finding.goes {
            Goes::Contents { taken, of } if self.rules[finding.rule].older.is_some() => Some(if taken == of {
                format!("All {} entries in it.", words::count(of))
            } else {
                format!("{} of the {} entries in it.", words::count(taken), words::count(of))
            }),
            _ => None,
        }
    }

    /// What deleting `finding` takes: the entry itself, or what goes from in
    /// it.
    ///
    /// # Panics
    ///
    /// If `finding` came from another set or another tree.
    #[must_use]
    pub fn going(&self, tree: &Tree, finding: &Finding, now: SystemTime) -> Vec<NodeId> {
        let rule = &self.rules[finding.rule];
        match rule.takes {
            Takes::Whole => vec![finding.id],
            Takes::Contents => {
                let outer = Outer::of(tree.root_path());
                tree.node(finding.id)
                    .children
                    .iter()
                    .copied()
                    .filter(|&child| rule.goes_with(tree, child, &outer, now))
                    .collect()
            }
        }
    }

    /// The folders of `tree` that are one thing: every finding, and every
    /// closed folder that is not one — a global npm `node_modules` is no
    /// project's, and is still one thing that grew. What a tree of changes
    /// names whole ([`units_in`]).
    #[must_use]
    pub fn units(&self, tree: &Tree) -> HashSet<NodeId> {
        self.survey(tree).units()
    }

    /// Every folder of `tree` a rule names, the most freed first.
    #[must_use]
    pub fn find(&self, tree: &Tree) -> Vec<Finding> {
        self.survey(tree).findings
    }

    /// Who what a rule claims is for: the consumer of the rule's group.
    /// [`Survey::claims`] holds only what has one.
    #[must_use]
    pub fn consumer_of(&self, claim: Claim) -> Consumer {
        let group = match claim {
            Claim::Found(finding) => finding.group,
            Claim::Closed(closed) => closed.group,
        };
        group.consumer().expect("a claim's group says whose it is")
    }
}

/// What a walk of one tree knows before it starts.
struct Walk<'a> {
    outer: Outer,
    /// Whether any rule names files: a set that names none looks at folders
    /// only, as a dev set always did.
    files: bool,
    /// Whether any rule asks whose an entry is, which takes the place of
    /// every folder on the way down.
    places: bool,
    /// What the scan could not read, for a hollow rule: a folder it could
    /// not list looks empty and is not.
    skipped: Vec<&'a std::path::Path>,
}

impl<'a> Walk<'a> {
    fn of(rules: &RuleSet, tree: &'a Tree) -> Self {
        let hollow = rules.rules.iter().any(|rule| rule.hollow);
        Self {
            outer: Outer::of(tree.root_path()),
            files: rules.rules.iter().any(|rule| rule.entries != Entries::Folders),
            places: rules.rules.iter().any(|rule| !rule.within.is_empty()),
            skipped: if hollow {
                tree.skipped().iter().map(|skipped| skipped.path.as_path()).collect()
            } else {
                Vec::new()
            },
        }
    }

    /// Whether `id` is a folder with no file anywhere in it, and nothing in
    /// it the scan could not read. A link is a file here, as the scanners
    /// record it: a folder holding a junction holds something.
    fn hollow(&self, tree: &Tree, id: NodeId) -> bool {
        let node = tree.node(id);
        if !node.is_directory() || node.size.logical > 0 {
            return false;
        }
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            for &child in &tree.node(at).children {
                if !tree.node(child).is_directory() {
                    return false;
                }
                stack.push(child);
            }
        }
        if self.skipped.is_empty() {
            return true;
        }
        let path = tree.path_of(id);
        !self.skipped.iter().any(|skipped| skipped.starts_with(&path))
    }
}

/// What the rules say of a tree ([`RuleSet::survey`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Survey {
    /// The entries a rule names, the most freed first.
    pub findings: Vec<Finding>,
    /// The closed folders no rule names: one thing each, and nobody's to
    /// delete.
    pub closed: Vec<Closed>,
    /// The entries a rule names that its check has still to settle
    /// ([`RuleSet::settle`]), in the order the walk met them.
    pub waiting: Vec<Waiting>,
}

impl Survey {
    /// The folders that are one thing a tool makes again: what the groups of
    /// development claim. The picture colours them as a tool's, and who is
    /// eating the disk names each as one line; the Recycle Bin and an
    /// emptied temporary folder are neither, so their folders are told as
    /// any other.
    #[must_use]
    pub fn units(&self) -> HashSet<NodeId> {
        self.claims()
            .filter(|&claim| {
                let group = match claim {
                    Claim::Found(finding) => finding.group,
                    Claim::Closed(closed) => closed.group,
                };
                group.consumer() == Some(Consumer::Development)
            })
            .map(Claim::id)
            .collect()
    }

    /// Everything a rule claims whole for its group's consumer: the findings
    /// of a group that says whose they are, and the closed folders. A finding
    /// whose group says nothing — an old download — claims no bytes.
    pub fn claims(&self) -> impl Iterator<Item = Claim> + '_ {
        self.findings
            .iter()
            .copied()
            .filter(|finding| finding.group.consumer().is_some())
            .map(Claim::Found)
            .chain(
                self.closed
                    .iter()
                    .copied()
                    .filter(|closed| closed.group.consumer().is_some())
                    .map(Claim::Closed),
            )
    }
}

/// A closed folder no rule names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed {
    pub id: NodeId,
    /// The rule that closes it, by its place in the set.
    rule: usize,
    group: Group,
}

/// What a rule claims whole: a finding, or a closed folder that is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    Found(Finding),
    Closed(Closed),
}

impl Claim {
    /// The entry.
    #[must_use]
    pub const fn id(self) -> NodeId {
        match self {
            Self::Found(finding) => finding.id,
            Self::Closed(closed) => closed.id,
        }
    }
}

/// The folders directly in `id`.
fn folders_in(tree: &Tree, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    tree.node(id).children.iter().copied().filter(|&child| tree.node(child).is_directory())
}

/// What deleting a finding takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Goes {
    /// The entry itself.
    Whole,
    /// What is in the folder, which stays: `taken` of the `of` entries
    /// directly in it — every one, unless the rule says only the old.
    Contents { taken: u64, of: u64 },
}

/// One entry a rule names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finding {
    /// The entry, in the tree that was asked about: for a folder that is
    /// emptied, the folder.
    pub id: NodeId,
    /// Which rule names it, by its place in the set.
    rule: usize,
    /// Which of the rule's ways back applies, by its place in the rule.
    restore: usize,
    pub group: Group,
    /// What the tree counts for what goes.
    pub holds: Size,
    /// What deleting it would give back: the files whose every name is in
    /// what goes. Never more than [`Finding::holds`].
    pub frees: Size,
    /// How many entries go, and when the newest of them was written.
    pub entries: u64,
    pub newest: Option<SystemTime>,
    pub goes: Goes,
}

/// An entry a rule names, waiting for its check: whether it is the rule's at
/// all, and which way back applies, is what only the file can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    /// Where the file is, to be read away from the tree.
    pub path: PathBuf,
    pub check: Check,
    /// The finding it is, but for the way back.
    pub finding: Finding,
}

/// What deleting `going` would give back: every file under them whose names
/// are all under them.
///
/// A file with one name is freed with it. A file with several is freed only
/// when every one of them is under `going` — and then once, through the name
/// the tree counts its bytes under, which is one of them. A file whose names
/// the platform counted but whose identity it would not give cannot be
/// matched with its other names, and is not counted as freed: saying it
/// would be is the error in the direction this product must never err in.
fn freed_by(tree: &Tree, going: &[NodeId]) -> Size {
    let mut freed = Size::zero();
    // Per file with several names: how many are under `going`, how many it
    // has, and the bytes the tree counts for the ones under `going`.
    let mut shared: HashMap<FileIdentity, (u32, u32, Size)> = HashMap::new();
    let mut stack = going.to_vec();
    while let Some(at) = stack.pop() {
        let node = tree.node(at);
        if node.is_directory() {
            stack.extend_from_slice(&node.children);
            continue;
        }
        match (node.links, node.identity) {
            (Some(links), Some(identity)) if links > 1 => {
                let (here, total, bytes) = shared.entry(identity).or_insert((0, links, Size::zero()));
                *here += 1;
                *total = links;
                bytes.add(node.size);
            }
            (Some(links), None) if links > 1 => {}
            _ => freed.add(node.size),
        }
    }
    for (here, total, bytes) in shared.into_values() {
        if here >= total {
            freed.add(bytes);
        }
    }
    freed
}

/// The folders of `tree` that are among `found` — [`RuleSet::units`] of
/// `context`, a tree of the same folder at another moment or as a whole.
///
/// What a comparison or the view of what was written lately needs to name a
/// `node_modules` as one line rather than three ([`crate::growth`]): those
/// trees hold only what changed, so the `package.json` beside a folder is not
/// in them to be asked about. The folder as it is knows; this matches the two
/// trees name by name from the root, and a folder the other tree does not
/// hold is not one.
#[must_use]
pub fn units_in(tree: &Tree, context: &Tree, found: &HashSet<NodeId>) -> HashSet<NodeId> {
    let mut units = HashSet::new();
    let mut stack = vec![(ROOT, ROOT)];
    while let Some((here, there)) = stack.pop() {
        for child in folders_in(tree, here) {
            let Some(other) = context.child(there, &tree.node(child).name) else {
                continue;
            };
            if found.contains(&other) {
                units.insert(child);
            } else if context.node(other).is_directory() {
                stack.push((child, other));
            }
        }
    }
    units
}

/// The shape of a rules file, as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    rule: Vec<RawRule>,
    #[serde(default)]
    place: Vec<places::RawPlace>,
    #[serde(default)]
    file: Vec<places::RawFileKind>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    name: String,
    group: Group,
    paths: Vec<String>,
    #[serde(default)]
    beside: Vec<String>,
    #[serde(default)]
    inside: Vec<String>,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    entries: Entries,
    older: Option<String>,
    #[serde(default)]
    goes: Takes,
    #[serde(default)]
    hollow: bool,
    #[serde(default)]
    within: Vec<Consumer>,
    #[serde(default)]
    except: Vec<String>,
    check: Option<Check>,
    restore: Vec<RawRestore>,
    cleanup: Option<cleanup::Raw>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRestore {
    when: Option<String>,
    #[serde(rename = "if")]
    outcome: Option<Outcome>,
    command: Option<String>,
    risk: Risk,
    explain: String,
}

/// An age as a rule writes it: `7 days`, `2 weeks`, `3 months`, `1 year` — a
/// month thirty days, a year 365.
fn read_age(text: &str) -> Result<Duration, String> {
    let refused = || format!("`{text}` is not an age: a number and days, weeks, months or years");
    let (count, unit) = text.trim().split_once(' ').ok_or_else(refused)?;
    let count: u64 = count.parse().map_err(|_| refused())?;
    let days = match unit.trim() {
        "day" | "days" => 1,
        "week" | "weeks" => 7,
        "month" | "months" => 30,
        "year" | "years" => 365,
        _ => return Err(refused()),
    };
    if count == 0 {
        return Err("an age of nothing is every entry: leave `older` out".to_owned());
    }
    Ok(Duration::from_secs(count * days * 24 * 60 * 60))
}

/// Whether `id` is kebab-case: lowercase letters, digits and dashes, a dash
/// at neither end.
fn kebab(id: &str) -> Result<(), String> {
    let kebab = !id.is_empty() && !id.starts_with('-') && !id.ends_with('-') && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if kebab {
        Ok(())
    } else {
        Err("an id is lowercase letters, digits and dashes".to_owned())
    }
}

/// The paths and names a rule or a place is written with, read.
fn read_paths(paths: &[String]) -> Result<Vec<Template>, String> {
    if paths.is_empty() {
        return Err("at least one path is needed".to_owned());
    }
    paths.iter().map(|path| Template::new(path)).collect()
}

fn read_globs(names: &[String]) -> Result<Vec<Glob>, String> {
    names.iter().map(|name| Glob::new(name)).collect()
}

impl RawRule {
    /// The rule, when it can explain itself.
    fn check(self) -> Result<Rule, String> {
        kebab(&self.id)?;
        if self.name.trim().is_empty() {
            return Err("a rule needs a name a reader calls the thing by".to_owned());
        }
        let paths = read_paths(&self.paths)?;
        let beside = read_globs(&self.beside)?;
        let inside = read_globs(&self.inside)?;
        let older = self.older.as_deref().map(read_age).transpose()?;
        self.shape()?;

        let Some((last, earlier)) = self.restore.split_last() else {
            return Err("a rule needs a way back: what brings the entry back, and what is lost".to_owned());
        };
        if last.when.is_some() || last.outcome.is_some() {
            return Err("the last way back is the one taken when no other applies, so it has no `when` and no `if`".to_owned());
        }
        if earlier.iter().any(|way| way.when.is_none() && way.outcome.is_none()) {
            return Err("only the last way back goes without a condition: a way after one without would never be taken".to_owned());
        }
        for way in &self.restore {
            match (self.check, way.when.is_some(), way.outcome) {
                (Some(_), true, _) => return Err("a rule with a check takes its ways back by what the check says: `if`, not `when`".to_owned()),
                (None, _, Some(_)) => return Err("`if` is what a check says, and this rule has no check".to_owned()),
                (Some(check), _, Some(outcome)) if !check.says(outcome) => {
                    return Err(format!("the {} check never says `{}`", check.key(), outcome.key()));
                }
                _ => {}
            }
        }
        let restore = self
            .restore
            .into_iter()
            .map(|way| {
                if way.explain.trim().is_empty() {
                    return Err("every way back explains itself".to_owned());
                }
                if way.command.as_deref().is_some_and(|command| command.trim().is_empty()) {
                    return Err("a command cannot be empty; leave it out when there is none".to_owned());
                }
                Ok(Restore {
                    when: way.when.as_deref().map(Glob::new).transpose()?,
                    outcome: way.outcome,
                    command: way.command,
                    risk: way.risk,
                    explain: way.explain,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let cleanup = self.cleanup.map(cleanup::Raw::check).transpose()?;

        Ok(Rule {
            id: self.id,
            name: self.name,
            group: self.group,
            paths,
            beside,
            inside,
            closed: self.closed,
            entries: self.entries,
            older,
            takes: self.goes,
            hollow: self.hollow,
            within: self.within,
            except: self.except.iter().map(|path| Template::new(path)).collect::<Result<_, _>>()?,
            check: self.check,
            restore,
            cleanup,
        })
    }

    /// Whether what the rule says of its entries fits together: each of
    /// these is a sentence about a folder, or about a file.
    fn shape(&self) -> Result<(), String> {
        let folders = self.entries != Entries::Files;
        let files = self.entries != Entries::Folders;
        if self.goes == Takes::Contents && files {
            return Err("only a folder is emptied: `goes = \"contents\"` names folders".to_owned());
        }
        if self.hollow && files {
            return Err("only a folder is empty: `hollow` names folders".to_owned());
        }
        if !self.inside.is_empty() && !folders {
            return Err("nothing is inside a file: `inside` names folders".to_owned());
        }
        if self.closed && files {
            return Err("only a folder is closed".to_owned());
        }
        if self.check.is_some() && folders {
            return Err("a check reads a file: `entries = \"files\"`".to_owned());
        }
        if self.cleanup.is_some() && self.check.is_some() {
            return Err("a cleanup is for a cache's folder, not for what a check reads".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod system_tests;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tree::{Kind, Node};

    /// A tree at `root` from paths: a trailing `/` is a folder, anything
    /// else a file of `bytes`.
    fn tree_of(root: &str, entries: &[(&str, u64)]) -> Tree {
        let mut tree = Tree::new(PathBuf::from(root), Some(4096));
        let mut sorted: Vec<(&str, u64)> = entries.to_vec();
        sorted.sort_by_key(|(path, _)| path.trim_end_matches('/').matches('/').count());
        for (path, bytes) in sorted {
            let folder = path.ends_with('/');
            let path = path.trim_end_matches('/');
            let (above, name) = path.rsplit_once('/').unwrap_or(("", path));
            let parent = above
                .split('/')
                .filter(|part| !part.is_empty())
                .fold(ROOT, |at, part| tree.child(at, part).expect("the folder above comes first"));
            let node = if folder {
                Node::new(name.into(), parent, Kind::Directory)
            } else {
                Node {
                    size: Size::flat(bytes),
                    ..Node::new(name.into(), parent, Kind::File)
                }
            };
            tree.insert(parent, node);
        }
        tree.roll_up();
        tree
    }

    fn named(tree: &Tree, findings: &[Finding]) -> Vec<String> {
        let root = tree.root_path().to_string_lossy().replace('\\', "/");
        findings
            .iter()
            .map(|finding| {
                let path = tree.path_of(finding.id).to_string_lossy().replace('\\', "/");
                path.strip_prefix(&root).unwrap_or(&path).trim_start_matches('/').to_owned()
            })
            .collect()
    }

    #[test]
    fn the_shipped_rules_read_and_each_explains_itself() {
        let rules = RuleSet::shipped();
        assert!(rules.rules().len() >= 10, "the dev set is there");
        for rule in rules.rules() {
            let last = rule.restores().last().expect("a way back");
            assert!(last.when.is_none(), "{}: the last way back applies to everything left", rule.id);
            for way in rule.restores() {
                assert!(way.explain.ends_with('.'), "{}: a sentence, said whole: {:?}", rule.id, way.explain);
            }
        }
    }

    #[test]
    fn a_folder_counts_only_with_what_explains_it_beside_it() {
        let tree = tree_of(
            "C:/work",
            &[
                ("app/", 0),
                ("app/package.json", 1),
                ("app/node_modules/", 0),
                ("app/node_modules/a.js", 4000),
                ("notes/", 0),
                ("notes/node_modules/", 0),
                ("notes/node_modules/mine.txt", 9000),
                ("rust/", 0),
                ("rust/Cargo.toml", 1),
                ("rust/target/", 0),
                ("rust/target/x.rlib", 7000),
                ("photos/", 0),
                ("photos/target/", 0),
                ("photos/target/shot.jpg", 8000),
            ],
        );
        let findings = RuleSet::shipped().find(&tree);
        assert_eq!(named(&tree, &findings), ["rust/target", "app/node_modules"]);
    }

    #[test]
    fn the_way_back_follows_the_lockfile_and_without_one_it_says_to_look() {
        let tree = tree_of(
            "C:/work",
            &[
                ("pinned/", 0),
                ("pinned/package.json", 1),
                ("pinned/pnpm-lock.yaml", 1),
                ("pinned/node_modules/", 0),
                ("pinned/node_modules/a.js", 5000),
                ("loose/", 0),
                ("loose/package.json", 1),
                ("loose/node_modules/", 0),
                ("loose/node_modules/a.js", 4000),
            ],
        );
        let rules = RuleSet::shipped();
        let findings = rules.find(&tree);
        assert_eq!(named(&tree, &findings), ["pinned/node_modules", "loose/node_modules"]);
        let pinned = rules.restore_of(&findings[0]);
        assert_eq!((pinned.command.as_deref(), pinned.risk), (Some("pnpm install"), Risk::Safe));
        let loose = rules.restore_of(&findings[1]);
        assert_eq!(loose.risk, Risk::Caution, "no lockfile: an install may bring other versions");
    }

    #[test]
    fn a_finding_is_one_thing_and_nothing_inside_it_is_another() {
        // A package's own build output is part of the node_modules it is in.
        let tree = tree_of(
            "C:/work",
            &[
                ("app/", 0),
                ("app/package.json", 1),
                ("app/node_modules/", 0),
                ("app/node_modules/lib/", 0),
                ("app/node_modules/lib/package.json", 1),
                ("app/node_modules/lib/dist/", 0),
                ("app/node_modules/lib/dist/index.js", 3000),
            ],
        );
        assert_eq!(named(&tree, &RuleSet::shipped().find(&tree)), ["app/node_modules"]);
    }

    #[test]
    fn a_cache_is_found_by_where_it_is_wherever_the_scan_began() {
        let entries = [
            ("AppData/", 0),
            ("AppData/Local/", 0),
            ("AppData/Local/npm-cache/", 0),
            ("AppData/Local/npm-cache/blob", 6000),
        ];
        let tree = tree_of("C:/Users/someone", &entries);
        assert_eq!(named(&tree, &RuleSet::shipped().find(&tree)), ["AppData/Local/npm-cache"]);

        let inner = tree_of("C:/Users/someone/AppData/Local", &[("npm-cache/", 0), ("npm-cache/blob", 6000)]);
        assert_eq!(named(&inner, &RuleSet::shipped().find(&inner)), ["npm-cache"]);

        let elsewhere = tree_of("C:/Projects", &[("npm-cache/", 0), ("npm-cache/blob", 6000)]);
        assert!(RuleSet::shipped().find(&elsewhere).is_empty(), "a folder called npm-cache that is not npm's");
    }

    #[test]
    fn nothing_inside_a_node_modules_is_a_projects_even_with_nothing_beside_it() {
        // A global npm folder: installed tools, no package.json beside it.
        // The packages in it have their own package.json beside their own
        // dist, and deleting that dist would break the package.
        let tree = tree_of(
            "C:/Users/someone",
            &[
                ("AppData/", 0),
                ("AppData/Roaming/", 0),
                ("AppData/Roaming/npm/", 0),
                ("AppData/Roaming/npm/node_modules/", 0),
                ("AppData/Roaming/npm/node_modules/tool/", 0),
                ("AppData/Roaming/npm/node_modules/tool/package.json", 1),
                ("AppData/Roaming/npm/node_modules/tool/dist/", 0),
                ("AppData/Roaming/npm/node_modules/tool/dist/cli.js", 5000),
            ],
        );
        let rules = RuleSet::shipped();
        assert!(rules.find(&tree).is_empty(), "{:?}", named(&tree, &rules.find(&tree)));
        let roaming = tree.child(tree.child(ROOT, "AppData").expect("AppData"), "Roaming").expect("Roaming");
        let modules = tree.child(tree.child(roaming, "npm").expect("npm"), "node_modules").expect("node_modules");
        assert_eq!(rules.units(&tree), HashSet::from([modules]), "still one thing that grows");
    }

    #[test]
    fn a_workspace_without_a_package_json_is_still_installed_from_its_lockfile() {
        let tree = tree_of(
            "C:/work",
            &[
                ("mono/", 0),
                ("mono/pnpm-workspace.yaml", 1),
                ("mono/pnpm-lock.yaml", 1),
                ("mono/node_modules/", 0),
                ("mono/node_modules/a.js", 100),
            ],
        );
        let rules = RuleSet::shipped();
        let findings = rules.find(&tree);
        assert_eq!(named(&tree, &findings), ["mono/node_modules"]);
        assert_eq!(rules.restore_of(&findings[0]).command.as_deref(), Some("pnpm install"));
    }

    #[test]
    fn bytecode_counts_only_when_there_is_bytecode_in_it() {
        let tree = tree_of(
            "C:/work",
            &[
                ("tool/", 0),
                ("tool/__pycache__/", 0),
                ("tool/__pycache__/main.cpython-312.pyc", 2000),
                ("odd/", 0),
                ("odd/__pycache__/", 0),
                ("odd/__pycache__/notes.txt", 2000),
            ],
        );
        assert_eq!(named(&tree, &RuleSet::shipped().find(&tree)), ["tool/__pycache__"]);
    }

    /// A file of `bytes` with `links` names, identified by `file`.
    fn linked(tree: &mut Tree, parent: NodeId, name: &str, bytes: u64, links: u32, file: u128) {
        tree.insert(
            parent,
            Node {
                size: Size::flat(bytes),
                links: Some(links),
                identity: Some(FileIdentity { volume: 1, file }),
                ..Node::new(name.into(), parent, Kind::File)
            },
        );
    }

    #[test]
    fn a_folder_frees_only_the_files_whose_every_name_is_in_it() {
        // app/node_modules links two files from a store elsewhere on the
        // volume, and holds one of its own; a file linked twice within it
        // goes with it.
        let mut tree = tree_of(
            "C:/work",
            &[
                ("app/", 0),
                ("app/package.json", 1),
                ("app/node_modules/", 0),
                ("app/node_modules/own.js", 1000),
            ],
        );
        let app = tree.child(ROOT, "app").expect("app");
        let modules = tree.child(app, "node_modules").expect("node_modules");
        linked(&mut tree, modules, "from-store-1.js", 50_000, 2, 1);
        linked(&mut tree, modules, "from-store-2.js", 70_000, 3, 2);
        linked(&mut tree, modules, "twice-a.js", 4000, 2, 3);
        linked(&mut tree, modules, "twice-b.js", 4000, 2, 3);
        crate::links::count_shared_once(&mut tree);
        tree.refresh([modules]);

        let findings = RuleSet::shipped().find(&tree);
        let finding = findings.first().expect("node_modules is found");
        assert_eq!(finding.holds.allocated, 1000 + 50_000 + 70_000 + 4000, "what the tree counts: each file once");
        assert_eq!(finding.frees.allocated, 1000 + 4000, "its own file, and the one whose two names are both here");
    }

    #[test]
    fn a_file_whose_other_names_cannot_be_matched_is_not_called_freed() {
        let mut tree = tree_of("C:/work", &[("app/", 0), ("app/package.json", 1), ("app/node_modules/", 0)]);
        let app = tree.child(ROOT, "app").expect("app");
        let modules = tree.child(app, "node_modules").expect("node_modules");
        tree.insert(
            modules,
            Node {
                size: Size::flat(9000),
                links: Some(2),
                identity: None,
                ..Node::new("unknown.js".into(), modules, Kind::File)
            },
        );
        tree.refresh([modules]);
        let finding = RuleSet::shipped().find(&tree)[0];
        assert_eq!((finding.holds.allocated, finding.frees.allocated), (9000, 0));
    }

    #[test]
    fn the_most_freed_comes_first() {
        let tree = tree_of(
            "C:/work",
            &[
                ("a/", 0),
                ("a/Cargo.toml", 1),
                ("a/target/", 0),
                ("a/target/x", 1000),
                ("b/", 0),
                ("b/Cargo.toml", 1),
                ("b/target/", 0),
                ("b/target/x", 9000),
            ],
        );
        assert_eq!(named(&tree, &RuleSet::shipped().find(&tree)), ["b/target", "a/target"]);
    }

    #[test]
    fn the_root_is_what_was_opened_not_a_finding() {
        let tree = tree_of("C:/work/app/node_modules", &[("lib/", 0), ("lib/index.js", 100)]);
        assert!(RuleSet::shipped().find(&tree).is_empty());
    }

    #[test]
    fn a_rule_that_cannot_explain_itself_is_refused() {
        let base = |restore: &str| format!("[[rule]]\nid = \"x\"\nname = \"x\"\ngroup = \"build\"\npaths = [\"x\"]\n{restore}");
        let no_way = base("restore = []\n");
        assert!(RuleSet::parse(&no_way).expect_err("refused").contains("way back"));

        let conditional_last = base("[[rule.restore]]\nwhen = \"a\"\nrisk = \"safe\"\nexplain = \"A.\"\n");
        assert!(RuleSet::parse(&conditional_last).expect_err("refused").contains("no `when`"));

        let unreachable = base("[[rule.restore]]\nrisk = \"safe\"\nexplain = \"A.\"\n[[rule.restore]]\nrisk = \"safe\"\nexplain = \"B.\"\n");
        assert!(RuleSet::parse(&unreachable).expect_err("refused").contains("never be taken"));

        let silent = base("[[rule.restore]]\nrisk = \"safe\"\nexplain = \"  \"\n");
        assert!(RuleSet::parse(&silent).expect_err("refused").contains("explains itself"));

        let fine = base("[[rule.restore]]\nrisk = \"safe\"\nexplain = \"A.\"\n");
        assert!(RuleSet::parse(&fine).is_ok());
        let twice = format!("{fine}{fine}");
        assert!(RuleSet::parse(&twice).expect_err("refused").contains("twice"));
        let unknown = fine.replace("risk = \"safe\"", "risk = \"safe\"\ncolour = \"red\"");
        assert!(RuleSet::parse(&unknown).is_err(), "a field nobody reads is a typo");
    }

    #[test]
    fn a_unit_in_a_tree_of_changes_is_a_finding_in_the_folder_as_it_is() {
        let now = tree_of(
            "C:/work",
            &[
                ("rust/", 0),
                ("rust/Cargo.toml", 1),
                ("rust/target/", 0),
                ("rust/target/debug/", 0),
                ("rust/target/debug/x", 600),
            ],
        );
        // What changed holds the build folders, and not the Cargo.toml beside
        // them, which did not change.
        let changed = tree_of(
            "C:/work",
            &[
                ("rust/", 0),
                ("rust/target/", 0),
                ("rust/target/debug/", 0),
                ("rust/target/debug/x", 600),
                ("gone/", 0),
            ],
        );
        let units = units_in(&changed, &now, &RuleSet::shipped().units(&now));
        let rust = changed.child(ROOT, "rust").expect("rust");
        let target = changed.child(rust, "target").expect("target");
        assert_eq!(units, HashSet::from([target]));
    }
}
