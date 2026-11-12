//! A reader's own rules files: the folder they are kept in, and the one the
//! window writes when the reader keeps something.
//!
//! Every `.toml` in the folder is a file of rules, places and keeps in the
//! format the shipped ones are written in, read in the order of its name and
//! tried before midda's ([`super::RuleSet::with_own`]). One of them is the
//! window's: [`KEPT`], where "Keep" on a finding writes a `[[keep]]`. The
//! window writes nothing else there, and refuses to rewrite it once it holds
//! anything but keeps - a rule the reader put in by hand is theirs.

use std::path::Path;

use serde::Deserialize;

/// The file the window writes what the reader keeps into.
pub const KEPT: &str = "kept.toml";

/// What the top of the window's file says to whoever opens it.
const KEPT_HEADER: &str = "# What midda keeps: written by the window each time Keep is pressed on a
# finding. Nothing a keep names is a finding, nor anything in it, and a folder
# that holds something kept is never taken whole. Edit it as any rules file;
# see docs/rules.md in midda's repository for the format.
";

/// Every rules file in `folder`, by name, in the order they are tried: the
/// order of their names. A folder that is not there holds none; a file that
/// cannot be read as text is left out, as the error beside its name says.
#[must_use]
pub fn read_folder(folder: &Path) -> Vec<(String, Result<String, String>)> {
    let mut files: Vec<(String, Result<String, String>)> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let ends = name.len() > 5 && name[name.len() - 5..].eq_ignore_ascii_case(".toml");
            ends.then(|| {
                let text = std::fs::read_to_string(entry.path()).map_err(|error| format!("it does not read as text: {error}"));
                (name, text)
            })
        })
        .collect();
    files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
    files
}

/// One keep as the window writes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kept {
    pub paths: Vec<String>,
    #[serde(default)]
    pub why: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeptFile {
    #[serde(default)]
    keep: Vec<Kept>,
}

/// The keeps of the window's file, as written.
///
/// # Errors
///
/// When it is not TOML, or holds anything but keeps: the window does not
/// rewrite a file with the reader's own rules in it.
pub fn kept_in(text: &str) -> Result<Vec<Kept>, String> {
    toml::from_str::<KeptFile>(text)
        .map(|file| file.keep)
        .map_err(|error| format!("{KEPT} holds more than keeps, or does not read: {error}"))
}

/// The window's file holding `kept`, in order.
#[must_use]
pub fn kept_text(kept: &[Kept]) -> String {
    let mut text = KEPT_HEADER.to_owned();
    for keep in kept {
        text.push_str("\n[[keep]]\npaths = [");
        text.push_str(&keep.paths.iter().map(|path| quoted(path)).collect::<Vec<_>>().join(", "));
        text.push_str("]\n");
        if let Some(why) = keep.why.as_deref().map(str::trim).filter(|why| !why.is_empty()) {
            text.push_str("why = ");
            text.push_str(&quoted(why));
            text.push('\n');
        }
    }
    text
}

/// `text` as a TOML basic string.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::RuleSet;

    #[test]
    fn what_the_window_writes_is_read_back_as_it_was_and_by_the_set() {
        let kept = vec![
            Kept {
                paths: vec!["D:/Projects/app/target".to_owned()],
                why: Some("the \"release\" build\\nightly".to_owned()),
            },
            Kept {
                paths: vec!["C:/Users/ada/Downloads/keeper.iso".to_owned()],
                why: None,
            },
        ];
        let text = kept_text(&kept);
        assert_eq!(kept_in(&text), Ok(kept));
        let set = RuleSet::parse(&text).expect("a rules file like any other");
        assert_eq!(set.keeps().len(), 2);
        assert_eq!(set.keeps()[0].why.as_deref(), Some("the \"release\" build\\nightly"));
    }

    #[test]
    fn a_file_with_the_readers_rules_in_it_is_not_the_windows_to_rewrite() {
        let mine = "[[keep]]\npaths = [\"x\"]\n\n[[rule]]\nid = \"mine\"\n";
        assert!(kept_in(mine).is_err());
        assert_eq!(kept_in(""), Ok(Vec::new()));
    }

    #[test]
    fn a_folder_is_read_in_the_order_of_its_names_and_only_its_toml_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("b.toml"), "").expect("write");
        std::fs::write(dir.path().join("A.TOML"), "").expect("write");
        std::fs::write(dir.path().join("notes.txt"), "").expect("write");
        std::fs::create_dir(dir.path().join("c.toml")).expect("mkdir");
        let names: Vec<String> = read_folder(dir.path()).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["A.TOML", "b.toml"]);
        assert!(read_folder(&dir.path().join("nowhere")).is_empty());
    }
}
