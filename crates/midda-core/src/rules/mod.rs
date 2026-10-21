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
//! # Rules are data
//!
//! The rules midda ships with are a file, `rules/dev.toml`, read by the same
//! code that will read a user's own rules: one format, so whatever the
//! shipped rules can say, a user's can. See
//! `docs/adr/0012-rules-that-explain-themselves.md`.
//!
//! # Places
//!
//! The same files say what things are without saying they can go: a
//! [`Place`] names a folder or a file — Windows, a Steam library, a
//! browser's profile — and the [`Consumer`] its bytes are for; a
//! [`FileKind`] says what a file is by its name where no place says
//! otherwise. `rules/places.toml` holds the shipped ones, and
//! [`crate::consumers`] gives every byte of a tree to one consumer by them.
//! A folder a rule finds or closes is its group's consumer
//! ([`Group::consumer`]). See `docs/adr/0014-what-fills-the-disk.md`.
//!
//! # A finding
//!
//! The first folder down from the root that a rule names. Nothing under it is
//! looked at: a `node_modules` is one thing, however many `dist` folders its
//! packages hold. Each finding carries two numbers, because deleting a folder
//! does not always give back what it holds:
//!
//! - **holds** — what the tree counts for it;
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
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub use cleanup::Cleanup;
use pattern::{Glob, Outer, Template};
pub use places::{FileKind, Place};

use crate::consumers::Consumer;
use crate::platform::FileIdentity;
use crate::size::Size;
use crate::tree::{NodeId, ROOT, Tree};

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
}

impl Group {
    /// Who the folders of this group are for.
    ///
    /// A group belongs to one consumer, so a rule does not say its own: a
    /// rule that could say `group = "caches"` and `consumer = "games"` would
    /// be two answers to one question. A group a game's cache needs is a
    /// group of its own.
    #[must_use]
    pub const fn consumer(self) -> Consumer {
        match self {
            Self::Dependencies | Self::Build | Self::Caches => Consumer::Development,
        }
    }
}

/// How careful to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    /// Comes back by itself or with one command, as it was.
    Safe,
    /// Comes back, but not necessarily as it was — or the folder may hold
    /// something the tool did not make. Look first.
    Caution,
}

/// One way a folder comes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restore {
    /// Taken when an entry by this name is beside the folder. `None` on the
    /// last way, which is taken when no other is.
    when: Option<Glob>,
    /// What to run beside the folder to make it again, when one command does.
    pub command: Option<String>,
    pub risk: Risk,
    /// What the folder is, how it comes back and what is lost: one or two
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
    restore: Vec<Restore>,
    pub cleanup: Option<Cleanup>,
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

    /// Which way back applies to `id`, when the rule names it.
    fn applies(&self, tree: &Tree, id: NodeId, outer: &Outer) -> Option<usize> {
        if !self.paths.iter().any(|path| path.matches(tree, id, outer)) || !explained(tree, id, &self.beside, &self.inside) {
            return None;
        }
        let siblings = &tree.node(tree.node(id).parent).children;
        self.restore
            .iter()
            .position(|way| way.when.as_ref().is_none_or(|glob| has(tree, siblings, glob)))
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
}

/// The files the shipped rules and places are in.
const DEV: &str = include_str!("../../rules/dev.toml");
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
        SHIPPED.get_or_init(|| Self::parse_all(&[DEV, PLACES]).expect("the shipped rules read"))
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
        Ok(Self { rules, places, files, index })
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

    /// The rule and way back that name `id`, when one does: the first rule in
    /// the set's order.
    fn naming(&self, tree: &Tree, id: NodeId, outer: &Outer) -> Option<(usize, usize)> {
        self.rules
            .iter()
            .enumerate()
            .find_map(|(index, rule)| rule.applies(tree, id, outer).map(|way| (index, way)))
    }

    /// The closed rule whose path names `id`, when one does: nothing in it is
    /// anybody's.
    fn closing(&self, tree: &Tree, id: NodeId, outer: &Outer) -> Option<usize> {
        self.rules
            .iter()
            .position(|rule| rule.closed && rule.paths.iter().any(|path| path.matches(tree, id, outer)))
    }

    /// What the rules say of `tree`: the folders they find, the most freed
    /// first, and the closed folders that are not findings.
    ///
    /// The root is not a finding: it is what was opened, and a scan of a
    /// `node_modules` is a look inside one.
    #[must_use]
    pub fn survey(&self, tree: &Tree) -> Survey {
        let outer = Outer::of(tree.root_path());
        let mut findings = Vec::new();
        let mut closed = Vec::new();
        let mut stack: Vec<NodeId> = folders_in(tree, ROOT).collect();
        while let Some(id) = stack.pop() {
            match self.naming(tree, id, &outer) {
                Some((rule, restore)) => findings.push(Finding {
                    id,
                    rule,
                    restore,
                    holds: tree.node(id).size,
                    frees: freed_by(tree, id),
                }),
                None => match self.closing(tree, id, &outer) {
                    Some(rule) => closed.push(Closed { id, rule }),
                    None => stack.extend(folders_in(tree, id)),
                },
            }
        }
        findings.sort_by(|a, b| {
            b.frees
                .allocated
                .cmp(&a.frees.allocated)
                .then_with(|| b.holds.allocated.cmp(&a.holds.allocated))
                .then_with(|| tree.scan_order(a.id, b.id))
        });
        Survey { findings, closed }
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

    /// Who the folder a rule claims is for: the consumer of the rule's group.
    ///
    /// # Panics
    ///
    /// If `claim` came from another set.
    #[must_use]
    pub fn consumer_of(&self, claim: Claim) -> Consumer {
        let rule = match claim {
            Claim::Found(finding) => finding.rule,
            Claim::Closed(closed) => closed.rule,
        };
        self.rules[rule].group.consumer()
    }
}

/// What the rules say of a tree ([`RuleSet::survey`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Survey {
    /// The folders a rule names, the most freed first.
    pub findings: Vec<Finding>,
    /// The closed folders no rule names: one thing each, and nobody's to
    /// delete.
    pub closed: Vec<Closed>,
}

impl Survey {
    /// The folders that are one thing: the findings and the closed folders.
    #[must_use]
    pub fn units(&self) -> HashSet<NodeId> {
        self.claims().map(|claim| claim.id()).collect()
    }

    /// Every folder a rule claims whole.
    pub fn claims(&self) -> impl Iterator<Item = Claim> + '_ {
        self.findings
            .iter()
            .copied()
            .map(Claim::Found)
            .chain(self.closed.iter().copied().map(Claim::Closed))
    }
}

/// A closed folder no rule names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed {
    pub id: NodeId,
    /// The rule that closes it, by its place in the set.
    rule: usize,
}

/// A folder a rule claims whole: a finding, or a closed folder that is not
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    Found(Finding),
    Closed(Closed),
}

impl Claim {
    /// The folder.
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

/// One folder a rule names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finding {
    /// The folder, in the tree that was asked about.
    pub id: NodeId,
    /// Which rule names it, by its place in the set.
    rule: usize,
    /// Which of the rule's ways back applies, by its place in the rule.
    restore: usize,
    /// What the tree counts for it.
    pub holds: Size,
    /// What deleting it would give back: the files whose every name is in
    /// it. Never more than [`Finding::holds`].
    pub frees: Size,
}

/// What deleting `id` would give back: every file under it whose names are
/// all under it.
///
/// A file with one name is freed with it. A file with several is freed only
/// when every one of them is under `id` — and then once, through the name
/// the tree counts its bytes under, which is one of them. A file whose names
/// the platform counted but whose identity it would not give cannot be
/// matched with its other names, and is not counted as freed: saying it
/// would be is the error in the direction this product must never err in.
fn freed_by(tree: &Tree, id: NodeId) -> Size {
    let mut freed = Size::zero();
    // Per file with several names: how many are under `id`, how many it has,
    // and the bytes the tree counts for the ones under `id`.
    let mut shared: HashMap<FileIdentity, (u32, u32, Size)> = HashMap::new();
    let mut stack = vec![id];
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
    restore: Vec<RawRestore>,
    cleanup: Option<cleanup::Raw>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRestore {
    when: Option<String>,
    command: Option<String>,
    risk: Risk,
    explain: String,
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

        let Some((last, earlier)) = self.restore.split_last() else {
            return Err("a rule needs a way back: what brings the folder back, and what is lost".to_owned());
        };
        if last.when.is_some() {
            return Err("the last way back is the one taken when no other applies, so it has no `when`".to_owned());
        }
        if earlier.iter().any(|way| way.when.is_none()) {
            return Err("only the last way back goes without a `when`: a way after one without would never be taken".to_owned());
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
            restore,
            cleanup,
        })
    }
}

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
