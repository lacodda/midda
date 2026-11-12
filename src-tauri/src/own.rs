//! The reader's own rules: the files in midda's rules folder, read into the
//! set the open folder is surveyed with, and what the window writes there
//! when the reader keeps something.
//!
//! The folder is `rules` in the application's configuration - the reader's
//! roaming `AppData`, since rules are what the reader wants, not a picture of
//! this machine's disk. Every `.toml` in it is tried before midda's own
//! (ADR 0019): a file that does not read is listed with why, and costs only
//! itself. Adding a file copies it in after it has been read with the rest;
//! saving one copies it out; removing one sends it to the Recycle Bin. A keep
//! goes into `kept.toml`, the window's own file there.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use midda_core::rules::own::{self, KEPT, Kept};
use midda_core::rules::pattern::{Known, written_as_rule};
use midda_core::rules::{File, RuleSet};
use serde::{Deserialize, Serialize};

use crate::session::Session;

/// The set the open folder is surveyed with.
#[derive(Debug, Clone)]
pub struct Rules(pub Arc<RuleSet>);

impl Default for Rules {
    fn default() -> Self {
        Self(Arc::new(RuleSet::shipped().clone()))
    }
}

/// One of the reader's files, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesFileRow {
    pub name: String,
    pub rules: u64,
    pub places: u64,
    pub keeps: u64,
    /// Why it was not taken into the set, when it was not.
    pub refused: Option<String>,
}

/// One keep, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepRow {
    pub paths: Vec<String>,
    pub why: Option<String>,
    pub file: String,
    /// Whether the window wrote it, and can let it go: a keep in a file of
    /// the reader's own is theirs to edit.
    pub mine: bool,
}

/// The reader's own rules, as the window shows them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnRules {
    /// Where they are kept.
    pub folder: String,
    pub files: Vec<RulesFileRow>,
    pub keeps: Vec<KeepRow>,
}

/// Reads the files in `folder` into a set with midda's: the set, and each
/// file with how it read.
#[must_use]
pub fn load(folder: &Path) -> (Arc<RuleSet>, Vec<RulesFileRow>) {
    let known = Known::of_this_account();
    let read = own::read_folder(folder);
    let texts: Vec<(String, String)> = read
        .iter()
        .filter_map(|(name, text)| text.as_ref().ok().map(|text| (name.clone(), text.clone())))
        .collect();
    let (set, refused) = RuleSet::with_own(&texts, &known);
    let rows = read
        .into_iter()
        .map(|(name, text)| {
            let why = match &text {
                Err(why) => Some(why.clone()),
                Ok(_) => refused.iter().find(|refused| refused.name == name).map(|refused| refused.why.clone()),
            };
            let (rules, places, keeps) = match (&why, &text) {
                (None, Ok(text)) => File::read(&name, text, &known).map(|file| file.counts()).unwrap_or_default(),
                _ => (0, 0, 0),
            };
            RulesFileRow {
                name,
                rules: rules as u64,
                places: places as u64,
                keeps: keeps as u64,
                refused: why,
            }
        })
        .collect();
    (Arc::new(set), rows)
}

/// What the window shows of the reader's rules now.
fn shown(session: &Session) -> Result<OwnRules, String> {
    let (rules, files) = session.own_rules()?;
    let keeps = rules
        .keeps()
        .iter()
        .map(|keep| KeepRow {
            paths: keep.paths(),
            why: keep.why.clone(),
            mine: keep.file == KEPT,
            file: keep.file.clone(),
        })
        .collect();
    Ok(OwnRules {
        folder: session.rules_folder().display().to_string(),
        files,
        keeps,
    })
}

/// Reads the folder again and puts what it says in effect.
fn reload(session: &Session) -> Result<OwnRules, String> {
    let (set, files) = load(session.rules_folder());
    session.put_rules(set, files)?;
    shown(session)
}

/// The reader's own rules files and what they keep.
///
/// # Errors
///
/// When the session's state cannot be read.
#[tauri::command]
pub fn own_rules(session: tauri::State<'_, Session>) -> Result<OwnRules, String> {
    shown(&session)
}

/// Reads the rules folder again: what the reader changed in it by hand.
///
/// # Errors
///
/// As [`own_rules`].
#[tauri::command]
pub fn read_rules_again(session: tauri::State<'_, Session>) -> Result<OwnRules, String> {
    reload(&session)
}

/// Adds the rules file at `path` to the folder, once it reads with the rest;
/// one of the same name is replaced, as a newer copy of it.
///
/// # Errors
///
/// When it is not a `.toml`, is named as the window's own file, cannot be
/// read, or would not be taken into the set - and then why.
#[tauri::command]
pub fn add_rules_file(path: String, session: tauri::State<'_, Session>) -> Result<OwnRules, String> {
    let path = PathBuf::from(path);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.to_ascii_lowercase().ends_with(".toml"))
        .ok_or_else(|| "a rules file is a .toml file".to_owned())?
        .to_owned();
    if name.eq_ignore_ascii_case(KEPT) {
        return Err(format!("{KEPT} is the window's own file of keeps: give yours another name"));
    }
    let text = std::fs::read_to_string(&path).map_err(|error| format!("{name} does not read as text: {error}"))?;
    let folder = session.rules_folder();
    let mut files: Vec<(String, String)> = own::read_folder(folder)
        .into_iter()
        .filter(|(other, _)| !other.eq_ignore_ascii_case(&name))
        .filter_map(|(other, text)| text.ok().map(|text| (other, text)))
        .collect();
    files.push((name.clone(), text.clone()));
    files.sort_by_key(|(name, _)| name.to_lowercase());
    let (_, refused) = RuleSet::with_own(&files, &Known::of_this_account());
    if let Some(refused) = refused.iter().find(|refused| refused.name == name) {
        return Err(format!("{name} was not added: {}", refused.why));
    }
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    std::fs::write(folder.join(&name), text).map_err(|error| format!("{name} could not be written: {error}"))?;
    reload(&session)
}

/// Saves a copy of the reader's file `name` at `to`: a preset to hand on.
///
/// # Errors
///
/// When there is no such file, or the copy cannot be written.
#[tauri::command]
pub fn save_rules_file(name: String, to: String, session: tauri::State<'_, Session>) -> Result<(), String> {
    let from = in_folder(&session, &name)?;
    std::fs::copy(&from, &to)
        .map(|_| ())
        .map_err(|error| format!("{name} could not be saved: {error}"))
}

/// Sends the reader's file `name` to the Recycle Bin, and reads the folder
/// again without it.
///
/// # Errors
///
/// When there is no such file, or the bin does not take it.
#[tauri::command]
pub fn remove_rules_file(name: String, session: tauri::State<'_, Session>) -> Result<OwnRules, String> {
    let path = in_folder(&session, &name)?;
    midda_core::delete::recycle(&path).map_err(|(_, said)| format!("{name} stayed where it is: {said}"))?;
    reload(&session)
}

/// Opens the rules folder in Explorer, making it first.
///
/// # Errors
///
/// When it cannot be made, or Explorer cannot be started.
#[tauri::command]
pub fn open_rules_folder(session: tauri::State<'_, Session>) -> Result<(), String> {
    let folder = session.rules_folder();
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(folder)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Keeps the entry `id` of the tree on screen: never a finding again, nor
/// anything in it. Written into the window's own file.
///
/// # Errors
///
/// When the entry is not in the tree on screen, its path cannot be written
/// as a rule's, or the window's file holds the reader's own rules.
#[tauri::command]
pub fn keep_entry(arena: u64, id: u32, why: Option<String>, session: tauri::State<'_, Session>) -> Result<OwnRules, String> {
    let path = session.entry_path(arena, id)?;
    let written = written_as_rule(&path).ok_or_else(|| format!("{} cannot be written as a path a rule reads", path.display()))?;
    change_kept(&session, |kept| {
        if !kept.iter().any(|keep| keep.paths.iter().any(|kept| kept.eq_ignore_ascii_case(&written))) {
            kept.push(Kept {
                paths: vec![written],
                why: why.map(|why| why.trim().to_owned()).filter(|why| !why.is_empty()),
            });
        }
    })
}

/// Stops keeping `path`: the window's keep of it goes from its file.
///
/// # Errors
///
/// As [`keep_entry`] for the window's file.
#[tauri::command]
pub fn let_go(path: String, session: tauri::State<'_, Session>) -> Result<OwnRules, String> {
    change_kept(&session, |kept| {
        for keep in kept.iter_mut() {
            keep.paths.retain(|kept| !kept.eq_ignore_ascii_case(&path));
        }
        kept.retain(|keep| !keep.paths.is_empty());
    })
}

/// Rewrites the window's file of keeps by `change`, and reads the folder
/// again.
fn change_kept(session: &Session, change: impl FnOnce(&mut Vec<Kept>)) -> Result<OwnRules, String> {
    let folder = session.rules_folder();
    let file = folder.join(KEPT);
    let mut kept = match std::fs::read_to_string(&file) {
        Ok(text) => own::kept_in(&text)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("{KEPT} does not read: {error}")),
    };
    change(&mut kept);
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    std::fs::write(&file, own::kept_text(&kept)).map_err(|error| format!("{KEPT} could not be written: {error}"))?;
    reload(session)
}

/// The path of the reader's file `name`, when it is one in the folder.
fn in_folder(session: &Session, name: &str) -> Result<PathBuf, String> {
    if name.contains(['/', '\\']) || name == ".." {
        return Err(format!("{name} is not a file of the rules folder"));
    }
    let path = session.rules_folder().join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("there is no {name} in the rules folder"))
    }
}
