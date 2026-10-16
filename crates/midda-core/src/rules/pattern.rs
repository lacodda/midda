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
//! business — so a path is never anchored to a root.

use std::path::{Component, Path};

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

/// A path read from its end: the last name is the folder itself, the ones
/// before it the folders it sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// Outermost first, as written.
    names: Vec<Glob>,
}

impl Template {
    /// A path of names separated by `/`.
    ///
    /// # Errors
    ///
    /// When it is empty or any of its names is not a name.
    pub fn new(path: &str) -> Result<Self, String> {
        let names = path.split('/').map(Glob::new).collect::<Result<Vec<_>, _>>()?;
        if names.is_empty() {
            return Err("a path cannot be empty".to_owned());
        }
        Ok(Self { names })
    }

    /// The path as written.
    #[must_use]
    pub fn as_string(&self) -> String {
        self.names.iter().map(Glob::as_str).collect::<Vec<_>>().join("/")
    }

    /// Whether `id` is the folder this path names: its own name is the last,
    /// and each folder above it is the one before.
    ///
    /// Above the root of the tree the names are the root's own path, as
    /// `outer` holds it — so `AppData/Local/npm-cache` is found by a scan of
    /// `C:\Users\me\AppData\Local` as well as by a scan of `C:\`.
    #[must_use]
    pub fn matches(&self, tree: &Tree, id: NodeId, outer: &Outer) -> bool {
        let mut names = upward(tree, id, outer);
        self.names.iter().rev().all(|glob| names.next().is_some_and(|name| glob.matches(name)))
    }
}

/// The names of the root's own path, the root's first and the drive's last —
/// what is above a tree, for a path that reaches past its root.
#[derive(Debug, Clone, Default)]
pub struct Outer {
    names: Vec<String>,
}

impl Outer {
    /// The names of `root`'s path, innermost first. A drive or a share is not
    /// a name: a path that reaches it has reached past every folder.
    #[must_use]
    pub fn of(root: &Path) -> Self {
        let mut names: Vec<String> = root
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        names.reverse();
        Self { names }
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

    /// root (C:/Users/me) / AppData / Local / pnpm / store / v10
    fn profile() -> (Tree, NodeId) {
        let mut tree = Tree::new(PathBuf::from("C:/Users/me"), None);
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
        assert!(
            !path("C:/Users/me/AppData/Local/pnpm/store/v10").matches(&tree, v10, &outer),
            "a drive is not a name"
        );
        assert!(
            !path("x/Users/me/AppData/Local/pnpm/store/v10").matches(&tree, v10, &outer),
            "nothing is above the drive"
        );
    }

    #[test]
    fn a_scan_of_the_folder_itself_still_knows_where_it_is() {
        // Opened at the store: v10 is a child of the root, and the rest of
        // the path is the root's.
        let mut tree = Tree::new(PathBuf::from("C:/Users/me/AppData/Local/pnpm/store"), None);
        let v10 = tree.push(ROOT, Node::new("v10".into(), ROOT, Kind::Directory));
        let outer = Outer::of(tree.root_path());
        assert!(Template::new("AppData/Local/pnpm/store/v*").expect("a path").matches(&tree, v10, &outer));
    }

    #[test]
    fn a_volume_root_has_nothing_above_it() {
        let mut tree = Tree::new(PathBuf::from("C:/"), None);
        let modules = tree.push(ROOT, Node::new("node_modules".into(), ROOT, Kind::Directory));
        let outer = Outer::of(tree.root_path());
        assert!(Template::new("node_modules").expect("a path").matches(&tree, modules, &outer));
        assert!(
            !Template::new("*/node_modules").expect("a path").matches(&tree, modules, &outer),
            "the drive is not a folder"
        );
    }
}
