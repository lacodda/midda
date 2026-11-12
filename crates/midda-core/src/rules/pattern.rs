//! Names and paths as a rule writes them: `node_modules`, `build.gradle*`,
//! `AppData/Local/pnpm/store/v*`.
//!
//! A name is matched whole, ignoring case — Windows does, and a rule that
//! missed `Node_Modules` would be a rule about spelling. `*` stands for any
//! run of characters and `?` for one; nothing else is special, so a name with
//! brackets or braces in it means what it says.
//!
//! A path is a list of names read from its end: `pnpm/store/v*` is a folder
//! called `v`-something in a folder `store` in a folder `pnpm`, wherever that
//! is. The end is what a rule knows — a user's profile folder is called
//! something different on every machine, and a drive letter is the reader's
//! business — so a path is not anchored to a drive.
//!
//! It can be anchored to the top of one, though: a path written with a
//! leading `/` starts there. `/Windows` is the system's folder, not every
//! folder called `Windows` — `AppData\Local\Microsoft\Windows` is a profile's
//! — and `/Users/*/AppData` is every profile's, on whichever volume. A path
//! that starts with a drive, `D:/Archive`, is that volume's and no other's:
//! what a reader writes about their own disk, and where a folder the shell
//! moved is.
//!
//! And it can start where the shell keeps one of the reader's folders:
//! `{downloads}/*` is what is in Downloads, wherever that is — in the
//! profile, moved to `D:\Downloads`, or into OneDrive. A known folder is
//! read when the set is ([`Known`]), and stands for every place it is: the
//! profile's own folder of every account, and where this account's shell
//! keeps it now.

use std::path::{Component, Path, Prefix};

use crate::tree::{NodeId, ROOT, Tree};

/// One name, with `*` and `?` in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glob {
    pattern: String,
    /// Whether the pattern holds `*` or `?`. A plain name is compared at
    /// once, and it is the overwhelming majority of what a rule names.
    wild: bool,
}

impl Glob {
    /// A pattern for one name.
    ///
    /// # Errors
    ///
    /// When it is empty, holds a path separator — a name is one name — or is
    /// `.` or `..`, which name a place relative to another, not an entry.
    pub fn new(pattern: &str) -> Result<Self, String> {
        if pattern.is_empty() {
            return Err("a name cannot be empty".to_owned());
        }
        if pattern.contains(['/', '\\']) {
            return Err(format!("`{pattern}` is a path, not a name"));
        }
        if pattern == "." || pattern == ".." {
            return Err(format!("`{pattern}` is not the name of an entry"));
        }
        Ok(Self {
            pattern: pattern.to_owned(),
            wild: pattern.contains(['*', '?']),
        })
    }

    /// The pattern as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    /// Whether `name` is one this pattern names, ignoring case.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        if self.wild {
            wildcard(self.pattern.as_bytes(), name.as_bytes())
        } else {
            self.pattern.eq_ignore_ascii_case(name)
        }
    }
}

/// `*` and `?` against a name, ignoring ASCII case.
///
/// The classic greedy match with one point to return to: a `*` first takes
/// nothing, and each time what follows fails it takes one byte more. Linear
/// in practice, and no allocation — this runs against every folder of a
/// volume. Bytes rather than characters: a pattern's wildcards and letters
/// are ASCII, and a `?` that met the middle of a multibyte character would
/// only ever be asked about by a pattern that cannot name it anyway.
fn wildcard(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut back: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some(b'*') => {
                back = Some((p, n));
                p += 1;
            }
            Some(&b) if b == b'?' || b.eq_ignore_ascii_case(&name[n]) => {
                p += 1;
                n += 1;
            }
            _ => match back {
                Some((star, taken)) => {
                    p = star + 1;
                    n = taken + 1;
                    back = Some((star, taken + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&b| b == b'*')
}

/// One step of a path: a name, or `**` — any number of folders, none
/// included.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Name(Glob),
    Folders,
}

/// Where the first name of a path sits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Anchor {
    /// Anywhere: the path is read from its end only.
    Nowhere,
    /// At the top of any volume.
    Top,
    /// At the top of this volume, by its drive as `C:` writes it, upper case.
    Drive(String),
}

/// A path read from its end: the last name is the entry itself, the ones
/// before it the folders it sits in.
///
/// `**` between two names stands for any number of folders, none included:
/// `Start Menu/**/*.lnk` is a shortcut in the Start Menu or anywhere under
/// it, as a menu nests its programs in folders of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// Outermost first, as written.
    steps: Vec<Step>,
    /// Where the first name sits.
    anchor: Anchor,
    /// Whether any step is `**`: a path without one is matched name for name,
    /// with nothing collected — the overwhelming majority.
    deep: bool,
}

/// The known folder a path starts with — `{downloads}/*` — and what follows
/// it. A name in braces is a known folder when it is a lowercase word; any
/// other — `{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}` — is a folder's own name,
/// and means what it says.
fn known_at_start(path: &str) -> Option<(&str, &str)> {
    let (name, after) = path.strip_prefix('{')?.split_once('}')?;
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase())).then_some((name, after))
}

/// The drive a path starts with — `D:/Archive` — and the rest of it.
fn drive_of(path: &str) -> Option<(String, &str)> {
    let bytes = path.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/').then(|| (path[..2].to_ascii_uppercase(), &path[3..]))
}

impl Template {
    /// A path of names separated by `/`; a leading `/` anchors it to the top
    /// of a volume, a leading drive to the top of that one.
    ///
    /// # Errors
    ///
    /// When it is empty or any of its names is not a name; when it ends in
    /// `**`, which would name no entry, or says `**` twice in a row; when it
    /// starts at a known folder, which only [`Template::read`] can place.
    pub fn new(path: &str) -> Result<Self, String> {
        if known_at_start(path).is_some() {
            return Err(format!("`{path}` starts at a known folder, which is read with the set"));
        }
        let (anchor, path) = match (drive_of(path), path.strip_prefix('/')) {
            (Some((drive, rest)), _) => (Anchor::Drive(drive), rest),
            (None, Some(rest)) => (Anchor::Top, rest),
            (None, None) => (Anchor::Nowhere, path),
        };
        let steps = path
            .split('/')
            .map(|name| if name == "**" { Ok(Step::Folders) } else { Glob::new(name).map(Step::Name) })
            .collect::<Result<Vec<_>, _>>()?;
        match steps.last() {
            None => return Err("a path cannot be empty".to_owned()),
            Some(Step::Folders) => return Err(format!("`{path}` ends in `**`: a path ends in the name of what it names")),
            Some(Step::Name(_)) => {}
        }
        if steps.windows(2).any(|pair| pair.iter().all(|step| *step == Step::Folders)) {
            return Err(format!("`{path}` says `**` twice in a row"));
        }
        let deep = steps.contains(&Step::Folders);
        Ok(Self { steps, anchor, deep })
    }

    /// A path as a rules file writes it: as [`Template::new`] reads one, or
    /// starting at a known folder — `{downloads}/*` — which is a path for
    /// every place `known` says the folder is.
    ///
    /// # Errors
    ///
    /// As [`Template::new`]; and when a known folder is one midda does not
    /// know, or is not where the path starts.
    pub fn read(path: &str, known: &Known) -> Result<Vec<Self>, String> {
        let Some((name, after)) = known_at_start(path) else {
            return Ok(vec![Self::new(path)?]);
        };
        let folder = KnownFolder::from_key(name).ok_or_else(|| {
            format!(
                "`{{{name}}}` is not a folder midda knows: {}",
                KnownFolder::ALL.map(|folder| format!("{{{}}}", folder.key())).join(", ")
            )
        })?;
        let after = match after {
            "" => "",
            _ => after
                .strip_prefix('/')
                .ok_or_else(|| format!("`{path}`: a known folder is followed by `/` and a path"))?,
        };
        known
            .places(folder)
            .iter()
            .map(|place| Self::new(&if after.is_empty() { place.clone() } else { format!("{place}/{after}") }))
            .collect()
    }

    /// The path as written.
    #[must_use]
    pub fn as_string(&self) -> String {
        let path = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Name(glob) => glob.as_str(),
                Step::Folders => "**",
            })
            .collect::<Vec<_>>()
            .join("/");
        match &self.anchor {
            Anchor::Nowhere => path,
            Anchor::Top => format!("/{path}"),
            Anchor::Drive(drive) => format!("{drive}/{path}"),
        }
    }

    /// The last name: what the entry itself is called.
    #[must_use]
    pub fn last(&self) -> &Glob {
        // `new` refuses a path with no names, and one that ends in `**`.
        match &self.steps[self.steps.len() - 1] {
            Step::Name(glob) => glob,
            Step::Folders => unreachable!("a path ends in a name"),
        }
    }

    /// Whether `id` is the entry this path names: its own name is the last,
    /// and each folder above it is the one before — and, when the path is
    /// anchored, nothing is above the first.
    ///
    /// Above the root of the tree the names are the root's own path, as
    /// `outer` holds it — so `AppData/Local/npm-cache` is found by a scan of
    /// `C:\Users\me\AppData\Local` as well as by a scan of `C:\`.
    #[must_use]
    pub fn matches(&self, tree: &Tree, id: NodeId, outer: &Outer) -> bool {
        self.on(outer) && self.matches_names(upward(tree, id, outer))
    }

    /// Whether this path names a folder above the root of a scan: the one
    /// `up` folders above it, which is not in the tree but in its path.
    #[must_use]
    pub fn matches_above(&self, outer: &Outer, up: usize) -> bool {
        up > 0 && up < outer.names.len() && self.on(outer) && self.matches_names(outer.names[up..].iter().map(String::as_str))
    }

    /// Whether the volume a scan is on is one this path can be on: any, but
    /// for a path that names its drive.
    fn on(&self, outer: &Outer) -> bool {
        match &self.anchor {
            Anchor::Drive(drive) => outer.drive.as_deref().is_some_and(|on| on.eq_ignore_ascii_case(drive)),
            Anchor::Nowhere | Anchor::Top => true,
        }
    }

    /// Whether this path names the entry whose name and whose folders' names
    /// are `names`, its own first.
    fn matches_names<'a>(&self, mut names: impl Iterator<Item = &'a str>) -> bool {
        let anchored = self.anchor != Anchor::Nowhere;
        if self.deep {
            let names: Vec<&str> = names.collect();
            let steps: Vec<&Step> = self.steps.iter().rev().collect();
            return deep_match(&steps, &names, anchored);
        }
        self.steps
            .iter()
            .rev()
            .all(|step| matches!(step, Step::Name(glob) if names.next().is_some_and(|name| glob.matches(name))))
            && (!anchored || names.next().is_none())
    }
}

/// A folder the shell keeps for the reader, and moves when told to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownFolder {
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
}

impl KnownFolder {
    pub const ALL: [Self; 6] = [Self::Desktop, Self::Documents, Self::Downloads, Self::Music, Self::Pictures, Self::Videos];

    /// What a path calls it, between braces: `{downloads}`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Documents => "documents",
            Self::Downloads => "downloads",
            Self::Music => "music",
            Self::Pictures => "pictures",
            Self::Videos => "videos",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|folder| folder.key() == key)
    }

    /// Where every account's profile has it until the shell is told to move
    /// it: the folder's name in the profile.
    #[must_use]
    pub const fn in_profile(self) -> &'static str {
        match self {
            Self::Desktop => "Desktop",
            Self::Documents => "Documents",
            Self::Downloads => "Downloads",
            Self::Music => "Music",
            Self::Pictures => "Pictures",
            Self::Videos => "Videos",
        }
    }
}

/// Where the known folders are, as paths a rule can start with.
///
/// Every account's profile has each one under its own name — that is where
/// another account's folders are looked for, since only this account's
/// shell can be asked — and this account's shell may keep it elsewhere: a
/// policy or the reader moved Downloads to `D:\Downloads`, OneDrive took the
/// Desktop. Both are the folder; a rule about Downloads is about either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    places: Vec<(KnownFolder, Vec<String>)>,
}

impl Known {
    /// Each folder in every profile, and nowhere else: what a set is read
    /// with when there is no shell to ask, and in a test.
    #[must_use]
    pub fn in_profiles() -> Self {
        Self {
            places: KnownFolder::ALL
                .into_iter()
                .map(|folder| (folder, vec![format!("/Users/*/{}", folder.in_profile())]))
                .collect(),
        }
    }

    /// Each folder in every profile, and where this account's shell keeps it
    /// now ([`crate::shell::known_folders`]).
    #[must_use]
    pub fn of_this_account() -> Self {
        let mut known = Self::in_profiles();
        for (folder, path) in crate::shell::known_folders() {
            known.also(folder, &path);
        }
        known
    }

    /// Says `folder` is also at `path`: an absolute path, with its drive. A
    /// path that is not one, or not a path a rule can write, says nothing.
    pub fn also(&mut self, folder: KnownFolder, path: &Path) {
        let Some(written) = written_as_rule(path) else {
            return;
        };
        if let Some((_, places)) = self.places.iter_mut().find(|(known, _)| *known == folder)
            && !places.iter().any(|place| place.eq_ignore_ascii_case(&written))
        {
            places.push(written);
        }
    }

    /// Every place `folder` is, as paths.
    #[must_use]
    pub fn places(&self, folder: KnownFolder) -> &[String] {
        self.places
            .iter()
            .find(|(known, _)| *known == folder)
            .map_or(&[][..], |(_, places)| places.as_slice())
    }
}

/// An absolute path as a rule writes it - `D:/Downloads` - when it has a
/// drive and every name in it is a name a rule can hold: none with `*` or
/// `?`, which a rule reads as wildcards.
#[must_use]
pub fn written_as_rule(path: &Path) -> Option<String> {
    let mut components = path.components();
    let drive = match components.next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => format!("{}:", char::from(letter).to_ascii_uppercase()),
            _ => return None,
        },
        _ => return None,
    };
    let mut written = drive;
    for component in components {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let name = name.to_str()?;
                if name.contains(['*', '?']) {
                    return None;
                }
                written.push('/');
                written.push_str(name);
            }
            _ => return None,
        }
    }
    (written.len() > 2).then_some(written)
}

/// `steps` against `names`, both innermost first: a name takes one, `**`
/// any number — tried from none up, as few as the rest allows.
fn deep_match(steps: &[&Step], names: &[&str], anchored: bool) -> bool {
    match steps.split_first() {
        None => !anchored || names.is_empty(),
        Some((Step::Name(glob), rest)) => names.first().is_some_and(|name| glob.matches(name)) && deep_match(rest, &names[1..], anchored),
        Some((Step::Folders, rest)) => (0..=names.len()).any(|taken| deep_match(rest, &names[taken..], anchored)),
    }
}

/// The names of the root's own path, the root's first and the drive's last —
/// what is above a tree, for a path that reaches past its root.
#[derive(Debug, Clone, Default)]
pub struct Outer {
    names: Vec<String>,
    /// The drive the root is on, as `C:`: what a path that names its drive
    /// is matched against. None for a share, or where there are no drives.
    drive: Option<String>,
}

impl Outer {
    /// The names of `root`'s path, innermost first. A drive or a share is not
    /// a name: a path that reaches it has reached past every folder.
    #[must_use]
    pub fn of(root: &Path) -> Self {
        let mut drive = None;
        let mut names: Vec<String> = root
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                Component::Prefix(prefix) => {
                    if let Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) = prefix.kind() {
                        drive = Some(format!("{}:", char::from(letter).to_ascii_uppercase()));
                    }
                    None
                }
                _ => None,
            })
            .collect();
        names.reverse();
        Self { names, drive }
    }

    /// How many folders are above the root of the scan, the root's own name
    /// not counted.
    #[must_use]
    pub fn above_root(&self) -> usize {
        self.names.len().saturating_sub(1)
    }
}

/// The names from `id` up: its own, its folder's, and so on to the root's —
/// and past it, the root's own path.
///
/// The root's own name is the first of `outer`, not the root node's: the
/// node of a volume root is called `C:\`, which is a drive, not a folder.
fn upward<'a>(tree: &'a Tree, id: NodeId, outer: &'a Outer) -> impl Iterator<Item = &'a str> {
    let mut at = Some(id).filter(|&id| id != ROOT);
    let inside = std::iter::from_fn(move || {
        let here = at?;
        let node = tree.node(here);
        at = Some(node.parent).filter(|&parent| parent != ROOT);
        Some(node.name.as_str())
    });
    inside.chain(outer.names.iter().map(String::as_str))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tree::{Kind, Node};

    fn glob(pattern: &str) -> Glob {
        Glob::new(pattern).expect("a valid pattern")
    }

    #[test]
    fn a_plain_name_matches_itself_in_any_case_and_nothing_longer() {
        let modules = glob("node_modules");
        assert!(modules.matches("node_modules"));
        assert!(modules.matches("Node_Modules"));
        assert!(!modules.matches("node_modules2"));
        assert!(!modules.matches("node_module"));
    }

    #[test]
    fn a_star_takes_any_run_and_a_question_mark_one_character() {
        assert!(glob("v*").matches("v10"));
        assert!(glob("v*").matches("V"));
        assert!(!glob("v*").matches("store"));
        assert!(glob("build.gradle*").matches("build.gradle.kts"));
        assert!(glob("*.pyc").matches("module.cpython-312.pyc"));
        assert!(!glob("*.pyc").matches("module.py"));
        assert!(glob("a*b*c").matches("aXXbYYbZc"), "a star gives back what the next one needs");
        assert!(!glob("a*b*c").matches("aXXbYYbZ"));
        assert!(glob("?.txt").matches("a.txt"));
        assert!(!glob("?.txt").matches("ab.txt"));
    }

    #[test]
    fn a_name_is_not_a_path_nor_a_place() {
        for bad in ["", "a/b", "a\\b", ".", ".."] {
            assert!(Glob::new(bad).is_err(), "{bad:?} is refused");
        }
        assert!(Template::new("AppData//npm-cache").is_err(), "an empty name in a path is refused");
    }

    /// The top of a volume as the platform writes it: `C:/` here, `/` on
    /// Linux - where `C:` is a folder like any other, and a test that called
    /// it a drive would be asserting Windows, not the rule.
    const VOLUME: &str = if cfg!(windows) { "C:/" } else { "/" };

    /// root (Users/me on the volume) / AppData / Local / pnpm / store / v10
    fn profile() -> (Tree, NodeId) {
        let mut tree = Tree::new(PathBuf::from(format!("{VOLUME}Users/me")), None);
        let mut at = ROOT;
        for name in ["AppData", "Local", "pnpm", "store", "v10"] {
            at = tree.push(at, Node::new(name.into(), at, Kind::Directory));
        }
        (tree, at)
    }

    #[test]
    fn a_path_is_matched_from_its_end_and_reaches_past_the_root() {
        let (tree, v10) = profile();
        let outer = Outer::of(tree.root_path());
        let path = |text: &str| Template::new(text).expect("a valid path");

        assert!(path("pnpm/store/v*").matches(&tree, v10, &outer));
        assert!(path("AppData/Local/pnpm/store/v*").matches(&tree, v10, &outer));
        assert!(path("me/AppData/Local/pnpm/store/v*").matches(&tree, v10, &outer), "the root's own name");
        assert!(
            path("Users/me/AppData/Local/pnpm/store/v10").matches(&tree, v10, &outer),
            "and the path above it"
        );
        assert!(!path("Roaming/pnpm/store/v*").matches(&tree, v10, &outer));
        if cfg!(windows) {
            assert!(
                path("C:/Users/me/AppData/Local/pnpm/store/v10").matches(&tree, v10, &outer),
                "a drive anchors a path to its volume"
            );
            assert!(!path("D:/Users/me/AppData/Local/pnpm/store/v10").matches(&tree, v10, &outer), "and to no other");
            assert!(!path("C:/me/AppData/Local/pnpm/store/v10").matches(&tree, v10, &outer), "from its top");
        }
        assert!(
            !path("x/Users/me/AppData/Local/pnpm/store/v10").matches(&tree, v10, &outer),
            "nothing is above the drive"
        );
    }

    #[test]
    fn a_scan_of_the_folder_itself_still_knows_where_it_is() {
        // Opened at the store: v10 is a child of the root, and the rest of
        // the path is the root's.
        let mut tree = Tree::new(PathBuf::from(format!("{VOLUME}Users/me/AppData/Local/pnpm/store")), None);
        let v10 = tree.push(ROOT, Node::new("v10".into(), ROOT, Kind::Directory));
        let outer = Outer::of(tree.root_path());
        assert!(Template::new("AppData/Local/pnpm/store/v*").expect("a path").matches(&tree, v10, &outer));
    }

    #[test]
    fn an_anchored_path_starts_at_the_top_of_the_volume() {
        let (tree, v10) = profile();
        let outer = Outer::of(tree.root_path());
        let path = |text: &str| Template::new(text).expect("a valid path");

        assert!(path("/Users/*/AppData/Local/pnpm/store/v*").matches(&tree, v10, &outer));
        assert!(!path("/AppData/Local/pnpm/store/v*").matches(&tree, v10, &outer), "AppData is not at the top");
        assert!(
            !path("/me/AppData/Local/pnpm/store/v*").matches(&tree, v10, &outer),
            "nor is the root's own name"
        );
        assert_eq!(path("/Users/*/AppData").as_string(), "/Users/*/AppData");
        assert!(Template::new("/").is_err() && Template::new("//Users").is_err(), "a slash is not a name");

        // From inside the volume's top folder as well: the root's own path
        // says where the top is.
        let mut inner = Tree::new(PathBuf::from(format!("{VOLUME}Windows")), None);
        let temp = inner.push(ROOT, Node::new("Temp".into(), ROOT, Kind::Directory));
        let outer = Outer::of(inner.root_path());
        assert!(path("/Windows/Temp").matches(&inner, temp, &outer));
        assert!(!path("/Temp").matches(&inner, temp, &outer));
    }

    #[test]
    fn a_volume_root_has_nothing_above_it() {
        let mut tree = Tree::new(PathBuf::from(VOLUME), None);
        let modules = tree.push(ROOT, Node::new("node_modules".into(), ROOT, Kind::Directory));
        let outer = Outer::of(tree.root_path());
        assert!(Template::new("node_modules").expect("a path").matches(&tree, modules, &outer));
        assert!(
            Template::new("/node_modules").expect("a path").matches(&tree, modules, &outer),
            "and is the top"
        );
        assert!(
            !Template::new("*/node_modules").expect("a path").matches(&tree, modules, &outer),
            "the drive is not a folder"
        );
    }

    /// root (the volume) / Users / me / Start Menu / Programs / Tool / x.lnk,
    /// and the shortcut at each depth from Start Menu down.
    fn menu() -> (Tree, [NodeId; 3]) {
        let mut tree = Tree::new(PathBuf::from(VOLUME), None);
        let mut at = ROOT;
        let mut shortcuts = Vec::new();
        for name in ["Users", "me", "Start Menu", "Programs", "Tool"] {
            at = tree.push(at, Node::new(name.into(), at, Kind::Directory));
            if tree.node(at).name == "Start Menu" || tree.node(at).name == "Programs" || tree.node(at).name == "Tool" {
                shortcuts.push(tree.push(at, Node::new("x.lnk".into(), at, Kind::File)));
            }
        }
        (tree, shortcuts.try_into().expect("three shortcuts"))
    }

    #[test]
    fn two_stars_stand_for_any_number_of_folders_none_included() {
        let (tree, [top, programs, deep]) = menu();
        let outer = Outer::of(tree.root_path());
        let path = |text: &str| Template::new(text).expect("a valid path");

        let anywhere = path("/Users/*/Start Menu/**/*.lnk");
        for shortcut in [top, programs, deep] {
            assert!(anywhere.matches(&tree, shortcut, &outer), "{}", tree.path_of(shortcut).display());
        }
        assert!(
            !path("/Users/*/Start Menu/**/Programs/*.lnk").matches(&tree, top, &outer),
            "a name after ** is still a name"
        );
        assert!(path("/Users/*/Start Menu/**/Programs/*.lnk").matches(&tree, programs, &outer));
        assert!(path("/Users/*/Start Menu/**/Programs/**/*.lnk").matches(&tree, deep, &outer), "two of them");
        assert!(!path("/me/**/*.lnk").matches(&tree, deep, &outer), "anchored still means the top");
        assert!(path("/**/Tool/*.lnk").matches(&tree, deep, &outer));
        assert!(path("Start Menu/**/x.lnk").matches(&tree, deep, &outer), "and unanchored, from anywhere");
        assert_eq!(anywhere.as_string(), "/Users/*/Start Menu/**/*.lnk");

        assert!(Template::new("Start Menu/**").is_err(), "a path ends in what it names");
        assert!(Template::new("a/**/**/b").is_err());
    }
}
