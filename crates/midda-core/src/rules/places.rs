//! What things are, without a word on whether they can go: places, and kinds
//! of file by name.
//!
//! A rule's job is to say a folder comes back; a place's is only to say what
//! it is and who it is for — `C:\Windows` is Windows, `steamapps` is a game
//! library — so a volume can be told as a handful of consumers
//! ([`crate::consumers`]). The format is the rules' own: paths read from
//! their end, `beside` and `inside`, and a leading `/` for the top of a
//! volume.

use std::collections::HashMap;

use serde::Deserialize;

use super::pattern::{Glob, Outer, Template};
use super::{explained, kebab, read_globs, read_paths};
use crate::consumers::Consumer;
use crate::tree::{NodeId, Tree};
use crate::vdisk::Owner;

/// A folder or a file midda knows by where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// Unique among every rule and place of a set.
    pub id: String,
    /// What a reader calls it in the middle of a sentence: `the page file`,
    /// `Chrome`.
    pub name: String,
    pub consumer: Consumer,
    paths: Vec<Template>,
    beside: Vec<Glob>,
    inside: Vec<Glob>,
    /// A soft place: a file in it is what its own name says first, and the
    /// place's only when its name says nothing — a video in Documents is
    /// media, a file called `notes` there is a document.
    pub yields: bool,
    /// Whose machine the virtual disks in it are: what has to stop before
    /// one is compacted ([`crate::vdisk`]).
    pub owner: Option<Owner>,
}

impl Place {
    /// Whether this place's path at `path` is `id`.
    pub(super) fn names_by(&self, path: usize, tree: &Tree, id: NodeId, outer: &Outer) -> bool {
        self.paths[path].matches(tree, id, outer) && explained(tree, id, &self.beside, &self.inside)
    }

    /// The paths it names, as written.
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        self.paths.iter().map(Template::as_string).collect()
    }
}

/// What a file is by its name, where no place says otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileKind {
    pub consumer: Consumer,
    names: Vec<Glob>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawPlace {
    pub(super) id: String,
    name: String,
    consumer: Consumer,
    paths: Vec<String>,
    #[serde(default)]
    beside: Vec<String>,
    #[serde(default)]
    inside: Vec<String>,
    #[serde(default)]
    yields: bool,
    owner: Option<Owner>,
}

impl RawPlace {
    pub(super) fn check(self) -> Result<Place, String> {
        kebab(&self.id)?;
        if self.name.trim().is_empty() {
            return Err("a place needs a name a reader calls it by".to_owned());
        }
        if self.consumer == Consumer::Unrecognised {
            return Err("a place that names something recognises it: `unrecognised` is what no place names".to_owned());
        }
        if self.owner.is_some() && self.consumer != Consumer::VirtualDisks {
            return Err("an owner says whose machine a virtual disk is: only a place of `virtual-disks` has one".to_owned());
        }
        Ok(Place {
            paths: read_paths(&self.paths)?,
            beside: read_globs(&self.beside)?,
            inside: read_globs(&self.inside)?,
            id: self.id,
            name: self.name,
            consumer: self.consumer,
            yields: self.yields,
            owner: self.owner,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawFileKind {
    consumer: Consumer,
    names: Vec<String>,
}

impl RawFileKind {
    pub(super) fn check(self) -> Result<FileKind, String> {
        if self.names.is_empty() {
            return Err(format!("a kind of file for `{}` names no file", self.consumer.key()));
        }
        if self.consumer == Consumer::Unrecognised {
            return Err("a kind of file recognises its files: `unrecognised` is what nothing names".to_owned());
        }
        Ok(FileKind {
            consumer: self.consumer,
            names: read_globs(&self.names)?,
        })
    }
}

/// Places and kinds of file by the last name of what they name.
///
/// A walk of a system volume asks "which place is this" of a few million
/// entries, and nearly every answer is "none". Without the index each entry
/// would be compared with every path of every place; with it, an entry is
/// compared with the paths that end in its own name, the few that end in its
/// extension, and the handful whose last name has a wildcard elsewhere.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Index {
    /// Plain last names, folded to lower case: the place and which of its
    /// paths ends in it.
    named: HashMap<String, Vec<(usize, usize)>>,
    /// Last names of the form `*.ext`, by `ext` folded to lower case.
    extended: HashMap<String, Vec<(usize, usize)>>,
    /// Every other last name.
    wild: Vec<(usize, usize)>,
    /// Kinds of file the same way, by the index of the kind.
    kinds_extended: HashMap<String, usize>,
    kinds_wild: Vec<(Glob, usize)>,
}

/// The extension a `*.ext` pattern names, folded to lower case — `None` for
/// any other pattern.
fn extension_of(pattern: &str) -> Option<String> {
    let extension = pattern.strip_prefix("*.")?;
    (!extension.is_empty() && !extension.contains(['*', '?', '.'])).then(|| extension.to_ascii_lowercase())
}

/// What follows the last dot of `name`, folded into `scratch`.
fn extension_in<'a>(name: &str, scratch: &'a mut String) -> Option<&'a str> {
    let (_, extension) = name.rsplit_once('.')?;
    fold(extension, scratch);
    Some(scratch.as_str())
}

/// `text` in lower case, in `scratch`: the ASCII folding names are matched
/// with.
fn fold(text: &str, scratch: &mut String) {
    scratch.clear();
    scratch.extend(text.chars().map(|c| c.to_ascii_lowercase()));
}

impl Index {
    pub(super) fn of(places: &[Place], kinds: &[FileKind]) -> Self {
        let mut index = Self::default();
        for (at, place) in places.iter().enumerate() {
            for (path, template) in place.paths.iter().enumerate() {
                let last = template.last().as_str();
                if !last.contains(['*', '?']) {
                    index.named.entry(last.to_ascii_lowercase()).or_default().push((at, path));
                } else if let Some(extension) = extension_of(last) {
                    index.extended.entry(extension).or_default().push((at, path));
                } else {
                    index.wild.push((at, path));
                }
            }
        }
        for (at, kind) in kinds.iter().enumerate() {
            for glob in &kind.names {
                match extension_of(glob.as_str()) {
                    // The first kind to name an extension keeps it.
                    Some(extension) => {
                        index.kinds_extended.entry(extension).or_insert(at);
                    }
                    None => index.kinds_wild.push((glob.clone(), at)),
                }
            }
        }
        index
    }

    /// The paths of places that may name an entry called `name`, as the
    /// place and the path: each to be asked.
    pub(super) fn candidates<'a>(&'a self, name: &str, scratch: &mut String) -> impl Iterator<Item = (usize, usize)> + 'a {
        fold(name, scratch);
        let named = self.named.get(scratch.as_str()).map_or(&[][..], Vec::as_slice);
        let extended = extension_in(name, scratch)
            .and_then(|extension| self.extended.get(extension))
            .map_or(&[][..], Vec::as_slice);
        named.iter().chain(extended).chain(&self.wild).copied()
    }

    /// What `name` is by itself, among `kinds`: the first kind in the set
    /// that names it.
    pub(super) fn kind_of(&self, name: &str, kinds: &[FileKind], scratch: &mut String) -> Option<Consumer> {
        let by_extension = extension_in(name, scratch).and_then(|extension| self.kinds_extended.get(extension)).copied();
        let by_glob = self.kinds_wild.iter().filter(|(glob, _)| glob.matches(name)).map(|&(_, at)| at).min();
        let at = match (by_extension, by_glob) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b)?,
        };
        Some(kinds[at].consumer)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::RuleSet;
    use super::*;
    use crate::tree::{Kind, Node, ROOT};

    fn rules(text: &str) -> Result<RuleSet, String> {
        RuleSet::parse(text)
    }

    #[test]
    fn a_place_names_where_it_is_and_who_it_is_for() {
        let set = rules("[[place]]\nid = \"games\"\nname = \"the Steam library\"\nconsumer = \"games\"\npaths = [\"steamapps\"]\n").expect("reads");
        assert_eq!(set.places().len(), 1);
        assert_eq!(set.places()[0].consumer, Consumer::Games);
        assert_eq!(set.places()[0].paths(), ["steamapps"]);
    }

    #[test]
    fn a_place_that_says_nothing_is_refused() {
        let place = |body: &str| format!("[[place]]\nid = \"x\"\n{body}");
        assert!(
            rules(&place("name = \"x\"\nconsumer = \"games\"\npaths = []\n"))
                .expect_err("no path")
                .contains("path")
        );
        assert!(
            rules(&place("name = \" \"\nconsumer = \"games\"\npaths = [\"x\"]\n"))
                .expect_err("no name")
                .contains("name")
        );
        assert!(
            rules(&place("name = \"x\"\nconsumer = \"unrecognised\"\npaths = [\"x\"]\n"))
                .expect_err("unrecognised")
                .contains("unrecognised")
        );
        assert!(
            rules(&place("name = \"x\"\nconsumer = \"weather\"\npaths = [\"x\"]\n")).is_err(),
            "no such consumer"
        );
        let twice = "[[rule]]\nid = \"x\"\nname = \"x\"\ngroup = \"build\"\npaths = [\"x\"]\n[[rule.restore]]\nrisk = \"safe\"\nexplain = \"A.\"\n[[place]]\nid = \"x\"\nname = \"x\"\nconsumer = \"games\"\npaths = [\"x\"]\n";
        assert!(rules(twice).expect_err("one id").contains("twice"), "an id is unique across rules and places");
        assert!(rules("[[file]]\nconsumer = \"media\"\nnames = []\n").is_err());
    }

    #[test]
    fn the_index_finds_a_place_by_its_name_its_extension_or_a_wildcard() {
        let set = rules(
            "[[place]]\nid = \"plain\"\nname = \"p\"\nconsumer = \"system\"\npaths = [\"/Windows\"]\n\
             [[place]]\nid = \"extended\"\nname = \"e\"\nconsumer = \"virtual-disks\"\npaths = [\"*.vhdx\"]\n\
             [[place]]\nid = \"wild\"\nname = \"w\"\nconsumer = \"system\"\npaths = [\"/DumpStack.log*\"]\n",
        )
        .expect("reads");
        let mut tree = Tree::new(PathBuf::from(if cfg!(windows) { "C:/" } else { "/" }), None);
        let windows = tree.push(ROOT, Node::new("WINDOWS".into(), ROOT, Kind::Directory));
        let disk = tree.push(ROOT, Node::new("ext4.VHDX".into(), ROOT, Kind::File));
        let log = tree.push(ROOT, Node::new("DumpStack.log.tmp".into(), ROOT, Kind::File));
        let other = tree.push(ROOT, Node::new("notes.txt".into(), ROOT, Kind::File));
        let outer = Outer::of(tree.root_path());
        let mut scratch = String::new();
        let place = |id| set.place_of(&tree, id, &outer, &mut String::new());
        assert_eq!(place(windows), Some(0), "by name, in any case");
        assert_eq!(place(disk), Some(1), "by extension, in any case");
        assert_eq!(place(log), Some(2), "by a wildcard");
        assert_eq!(place(other), None);
        assert_eq!(set.kind_of("x.txt", &mut scratch), None, "no kind is written");
    }

    #[test]
    fn the_first_place_in_the_set_wins_whatever_the_index_met_first() {
        // Docker's disk is a `.vhdx` too: the place written first applies,
        // though the generic one is met by extension and the other by a path.
        let set = rules(
            "[[place]]\nid = \"docker-disk\"\nname = \"d\"\nconsumer = \"virtual-disks\"\npaths = [\"Docker/wsl/*/*.vhdx\"]\n\
             [[place]]\nid = \"wsl-disk\"\nname = \"w\"\nconsumer = \"virtual-disks\"\npaths = [\"ext4.vhdx\"]\n",
        )
        .expect("reads");
        let mut tree = Tree::new(PathBuf::from("C:/Users/me/AppData/Local"), None);
        let docker = tree.push(ROOT, Node::new("Docker".into(), ROOT, Kind::Directory));
        let wsl = tree.push(docker, Node::new("wsl".into(), docker, Kind::Directory));
        let data = tree.push(wsl, Node::new("data".into(), wsl, Kind::Directory));
        let disk = tree.push(data, Node::new("ext4.vhdx".into(), data, Kind::File));
        let outer = Outer::of(tree.root_path());
        assert_eq!(set.place_of(&tree, disk, &outer, &mut String::new()), Some(0));
    }

    #[test]
    fn a_kind_of_file_is_found_by_its_extension_or_its_pattern() {
        let set = rules("[[file]]\nconsumer = \"media\"\nnames = [\"*.mp4\", \"IMG_*\"]\n[[file]]\nconsumer = \"documents\"\nnames = [\"*.MP4\", \"*.md\"]\n")
            .expect("reads");
        let mut scratch = String::new();
        assert_eq!(
            set.kind_of("holiday.MP4", &mut scratch),
            Some(Consumer::Media),
            "the first kind keeps an extension"
        );
        assert_eq!(set.kind_of("IMG_0001", &mut scratch), Some(Consumer::Media));
        assert_eq!(set.kind_of("README.md", &mut scratch), Some(Consumer::Documents));
        assert_eq!(set.kind_of("Makefile", &mut scratch), None);
        assert_eq!(set.kind_of("archive.tar.gz", &mut scratch), None, "the last extension is the one asked");
    }
}
