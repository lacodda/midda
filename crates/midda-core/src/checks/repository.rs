//! What a Git working copy holds that its remote does not.
//!
//! A clone made from a working copy's remote brings back every commit the
//! remote has and every file those commits hold. It does not bring back what
//! is only here: a change not committed, a file git does not track, a commit
//! on a branch never pushed, a stash, an operation left halfway - nor what git
//! ignores, which is never committed at all. A working copy where none of that
//! is so, and where what git ignores is something a rule says its tool makes
//! again (`node_modules` beside its lockfile, a Cargo `target`), is one
//! `git clone` away; any other is listed, by what would be lost.
//!
//! Read as git reads it, by `gix`, from this machine alone: nothing is
//! fetched, so "the remote has it" means the remote had it the last time
//! this copy talked to it, and the sentence says when that was. Nothing is
//! written - not even the index's timestamps, which `git status` refreshes.
//!
//! See `docs/adr/0020-working-copies-a-clone-brings-back.md`.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gix::bstr::ByteSlice;

use super::Outcome;

/// How many of a list are named before the rest is counted.
const NAMED: usize = 3;

/// How deep submodules are followed: a submodule's own submodules, and theirs.
const DEPTH: usize = 3;

/// What the check makes of a working copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// [`Outcome::OnRemote`] when a clone brings it all back; `None` when
    /// something is only here.
    pub outcome: Option<Outcome>,
    /// What the reader is told: where the remote is and when this copy last
    /// heard from it, or what is only here.
    pub about: String,
    /// The clone that brings it back, run beside the folder.
    pub command: Option<String>,
}

/// What of a working copy is only here, as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Local {
    /// The remote a clone is made from, by name and address; `None` when
    /// the repository has none.
    pub remote: Option<Remote>,
    /// Commits reachable from `HEAD`, a branch or a tag that no branch of
    /// the remote reaches - what a clone would not bring back.
    pub unpushed: usize,
    /// The branches those commits are on, by name.
    pub branches: Vec<String>,
    /// Tracked paths that differ from `HEAD`, staged or not, and paths a
    /// merge left unresolved.
    pub changed: Vec<String>,
    /// What git does not track and does not ignore: a folder with nothing
    /// tracked in it once, with a `/`.
    pub untracked: Vec<String>,
    /// What git ignores that no rule says comes back: a folder once, with a
    /// `/`.
    pub ignored: Vec<String>,
    pub stashes: usize,
    /// A merge, a rebase or another operation git started and has not
    /// finished.
    pub operation: Option<&'static str>,
    /// The other working copies that share this one's `.git`, which go with
    /// it: where each is.
    pub worktrees: Vec<String>,
    /// The submodules that have no remote of their own, by path: a clone of
    /// the working copy around them has nowhere to bring them back from.
    pub alone: Vec<String>,
    /// What could not be read, when something could not: a working copy
    /// that cannot be read whole is not one a clone is known to bring back.
    pub unread: Option<String>,
}

/// A remote, as a clone would be made from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    /// Where it is, with any password left out - and, for a web address,
    /// the user too, which is where a token is written.
    pub url: String,
    /// The remote is a folder on this computer: a clone of it is a copy on
    /// the same machine, and goes with it.
    pub here: bool,
    /// When this copy last heard from it: the newest of the last fetch and
    /// the last change to a branch of it.
    pub heard: Option<SystemTime>,
}

impl Local {
    /// Whether a clone of the remote brings it all back.
    #[must_use]
    pub fn on_remote(&self) -> bool {
        self.remote.as_ref().is_some_and(|remote| !remote.here)
            && self.unpushed == 0
            && self.changed.is_empty()
            && self.untracked.is_empty()
            && self.ignored.is_empty()
            && self.stashes == 0
            && self.operation.is_none()
            && self.worktrees.is_empty()
            && self.alone.is_empty()
            && self.unread.is_none()
    }

    /// What the reader is told, as of `now`, of a working copy in a folder
    /// called `name`.
    #[must_use]
    pub fn verdict(&self, name: &str, now: SystemTime) -> Verdict {
        if self.on_remote() {
            let remote = self.remote.as_ref().expect("on a remote");
            let heard = remote
                .heard
                .map(|heard| format!(", as of {}", crate::recycled::ago(now, heard)))
                .unwrap_or_default();
            return Verdict {
                outcome: Some(Outcome::OnRemote),
                about: format!("All of it is on {} at {}{heard}.", remote.name, remote.url),
                command: Some(format!("git clone {} {}", argument(&remote.url), argument(name))),
            };
        }
        Verdict {
            outcome: None,
            about: self.only_here(),
            command: None,
        }
    }

    /// What is only here, in one sentence.
    fn only_here(&self) -> String {
        let mut parts = Vec::new();
        let remote = match &self.remote {
            None => {
                parts.push("it has no remote, so nothing in it is anywhere else".to_owned());
                None
            }
            Some(remote) if remote.here => {
                parts.push(format!("its remote {} is a folder on this computer, {}", remote.name, remote.url));
                Some(remote.name.as_str())
            }
            Some(remote) => Some(remote.name.as_str()),
        };
        if self.unpushed > 0 {
            let on = if self.branches.is_empty() {
                String::new()
            } else {
                format!(", on {}", named(&self.branches, self.branches.len()))
            };
            let commits = counted(self.unpushed, "commit", "commits");
            parts.push(match remote {
                Some(name) => format!("{commits} {name} does not have{on}"),
                None => format!("{commits}{}", on.replacen(',', "", 1)),
            });
        }
        if !self.changed.is_empty() {
            parts.push(format!(
                "{} not committed: {}",
                counted(self.changed.len(), "change", "changes"),
                named(&self.changed, self.changed.len())
            ));
        }
        if !self.untracked.is_empty() {
            parts.push(format!("not tracked: {}", named(&self.untracked, self.untracked.len())));
        }
        if !self.ignored.is_empty() {
            parts.push(format!("ignored, and nothing makes it again: {}", named(&self.ignored, self.ignored.len())));
        }
        if self.stashes > 0 {
            parts.push(counted(self.stashes, "stash", "stashes"));
        }
        if let Some(operation) = self.operation {
            parts.push(format!("a {operation} left halfway"));
        }
        if !self.worktrees.is_empty() {
            let copies = if self.worktrees.len() == 1 {
                "another working copy uses its .git"
            } else {
                "other working copies use its .git"
            };
            parts.push(format!("{copies}: {}", named(&self.worktrees, self.worktrees.len())));
        }
        if !self.alone.is_empty() {
            parts.push(format!("no remote of its own: {}", named(&self.alone, self.alone.len())));
        }
        if let Some(unread) = &self.unread {
            parts.push(format!("not all of it could be read ({unread})"));
        }
        let mut sentence = parts.join("; ");
        if let Some(first) = sentence.get(..1) {
            let upper = first.to_uppercase();
            sentence.replace_range(..1, &upper);
        }
        sentence.push('.');
        sentence
    }
}

/// `1 commit`, `3 commits`.
fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{} {}", crate::words::count(count as u64), if count == 1 { one } else { many })
}

/// The first few of `names`, and how many more of `total` there are.
fn named(names: &[String], total: usize) -> String {
    let shown: Vec<&str> = names.iter().take(NAMED).map(String::as_str).collect();
    let rest = total.saturating_sub(shown.len());
    if rest == 0 {
        shown.join(", ")
    } else {
        format!("{} and {} more", shown.join(", "), crate::words::count(rest as u64))
    }
}

/// A word of a command line, quoted when it holds what a shell would split.
fn argument(word: &str) -> String {
    if word.is_empty() || word.contains([' ', '\t', '"', '\'', '&', ';', '|', '(', ')', '<', '>', '^', '%', '!']) {
        format!("\"{}\"", word.replace('"', "\\\""))
    } else {
        word.to_owned()
    }
}

/// Reads the working copy in `folder`: `None` when it is not one midda can
/// open - no `.git` folder in it, or one git would not open either. A
/// working copy whose `.git` is a file - a linked worktree, a submodule's
/// checkout - is another repository's, and is not read as its own.
///
/// `explained` are the paths in the folder, from it with `/`, that a rule
/// says come back as they were: what git ignores is lost unless it is one of
/// them or inside one.
#[must_use]
pub fn read(folder: &Path, explained: &[String]) -> Option<Local> {
    let git = folder.join(".git");
    if !git.is_dir() {
        return None;
    }
    let _kept = held(&git, folder);
    let repo = gix::open(folder).ok()?;
    let explained: Vec<&str> = explained.iter().map(String::as_str).collect();
    let mut local = Local::default();
    read_into(&repo, "", &explained, &mut local, 0);
    Some(local)
}

/// What [`read`] says of `folder`, in the reader's words.
#[must_use]
pub fn look(folder: &Path, explained: &[String]) -> Option<Verdict> {
    let local = read(folder, explained)?;
    let name = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    Some(local.verdict(&name, SystemTime::now()))
}

/// Reads `repo` into `local`, its paths under `prefix` - empty for the
/// working copy itself, `lib/` for a submodule in `lib`.
fn read_into(repo: &gix::Repository, prefix: &str, explained: &[&str], local: &mut Local, depth: usize) {
    let mut problem: Option<String> = None;
    let mut unread = |what: &str, error: &dyn std::fmt::Display| {
        problem.get_or_insert_with(|| format!("{prefix}{what}: {error}"));
    };
    let remote = remote_of(repo);
    if depth == 0 {
        local.remote.clone_from(&remote);
    } else if remote.is_none() {
        local.alone.push(prefix.to_owned());
    }
    match unpushed(repo, remote.as_ref().map(|remote| remote.name.as_str())) {
        Ok((count, branches)) => {
            local.unpushed += count;
            local.branches.extend(branches.into_iter().map(|branch| format!("{prefix}{branch}")));
        }
        Err(error) => unread("its commits", &error),
    }
    match changed(repo) {
        Ok(paths) => local.changed.extend(paths.into_iter().map(|path| format!("{prefix}{path}"))),
        Err(error) => unread("its changes", &error),
    }
    match not_committed(repo, prefix, explained) {
        Ok((untracked, ignored)) => {
            local.untracked.extend(untracked.into_iter().map(|path| format!("{prefix}{path}")));
            local.ignored.extend(ignored.into_iter().map(|path| format!("{prefix}{path}")));
        }
        Err(error) => unread("what it does not track", &error),
    }
    local.stashes += stashes(repo);
    if local.operation.is_none() {
        local.operation = repo.state().map(operation);
    }
    if depth == 0 {
        match repo.worktrees() {
            Ok(worktrees) => local.worktrees.extend(worktrees.iter().map(|proxy| {
                proxy
                    .base()
                    .map_or_else(|_| proxy.git_dir().display().to_string(), |base| base.display().to_string())
            })),
            Err(error) => unread("its worktrees", &error),
        }
    }
    if depth < DEPTH {
        match repo.submodules() {
            Ok(Some(submodules)) => {
                for submodule in submodules {
                    let Ok(path) = submodule.path() else { continue };
                    let inner = format!("{prefix}{}/", path.to_str_lossy());
                    match submodule.open() {
                        Ok(Some(sub)) => read_into(&sub, &inner, explained, local, depth + 1),
                        // Not checked out: the superproject's commit names
                        // it, and a clone fetches it from its own remote.
                        Ok(None) => {}
                        Err(error) => unread(&format!("the submodule {inner}"), &error),
                    }
                }
            }
            Ok(None) => {}
            Err(error) => unread("its submodules", &error),
        }
    }
    if local.unread.is_none() {
        local.unread = problem;
    }
}

/// The remote a clone is made from: the one the current branch follows,
/// else `origin`, else the only one, else the first by name.
fn remote_of(repo: &gix::Repository) -> Option<Remote> {
    let names = repo.remote_names();
    let followed = repo
        .head_name()
        .ok()
        .flatten()
        .and_then(|head| repo.branch_remote_name(head.shorten(), gix::remote::Direction::Fetch))
        .map(|name| name.as_bstr().to_string())
        .filter(|name| names.iter().any(|known| known.as_bytes() == name.as_bytes()));
    let name = followed.or_else(|| {
        if names.iter().any(|known| known.as_bytes() == b"origin") {
            Some("origin".to_owned())
        } else {
            names.iter().next().map(|first| first.to_string())
        }
    })?;
    let found = repo.find_remote(name.as_str()).ok()?;
    let url = found.url(gix::remote::Direction::Fetch)?;
    let here = url.scheme == gix::url::Scheme::File;
    let mut shown = url.clone();
    shown.set_password(None);
    if matches!(shown.scheme, gix::url::Scheme::Http | gix::url::Scheme::Https) {
        shown.set_user(None);
    }
    Some(Remote {
        heard: heard(repo.git_dir(), &name),
        name,
        url: shown.to_bstring().to_string(),
        here,
    })
}

/// When this copy last heard from the remote `name`: the newest of the last
/// fetch and the logs of its branches.
fn heard(git: &Path, name: &str) -> Option<SystemTime> {
    let written = |path: &Path| std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok();
    let mut newest = written(&git.join("FETCH_HEAD"));
    let mut stack = vec![git.join("logs").join("refs").join("remotes").join(name)];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push(path);
            } else {
                newest = newest.max(written(&path));
            }
        }
    }
    newest
}

/// How many commits `HEAD`, the branches and the tags reach that no branch
/// of `remote` does, and the branches they are on. Without a remote, all of
/// them.
fn unpushed(repo: &gix::Repository, remote: Option<&str>) -> Result<(usize, Vec<String>), String> {
    let platform = repo.references().map_err(|error| error.to_string())?;
    let mut hidden = Vec::new();
    if let Some(remote) = remote {
        let prefix = format!("refs/remotes/{remote}/");
        for reference in platform.prefixed(prefix.as_str()).map_err(|error| error.to_string())?.flatten() {
            let mut reference = reference;
            if let Ok(id) = reference.peel_to_commit() {
                hidden.push(id.id);
            }
        }
    }
    let mut branches = Vec::new();
    let mut tips: Vec<gix::ObjectId> = Vec::new();
    for reference in platform.local_branches().map_err(|error| error.to_string())?.flatten() {
        let mut reference = reference;
        let name = reference.name().shorten().to_string();
        if let Ok(commit) = reference.peel_to_commit() {
            if count(repo, vec![commit.id], &hidden)? > 0 {
                branches.push(name);
            }
            tips.push(commit.id);
        }
    }
    for reference in platform.tags().map_err(|error| error.to_string())?.flatten() {
        let mut reference = reference;
        if let Ok(commit) = reference.peel_to_commit() {
            tips.push(commit.id);
        }
    }
    if let Ok(head) = repo.head_id() {
        tips.push(head.detach());
    }
    tips.sort_unstable();
    tips.dedup();
    branches.sort();
    Ok((count(repo, tips, &hidden)?, branches))
}

/// How many commits `tips` reach that `hidden` do not.
fn count(repo: &gix::Repository, tips: Vec<gix::ObjectId>, hidden: &[gix::ObjectId]) -> Result<usize, String> {
    if tips.is_empty() {
        return Ok(0);
    }
    let walk = repo
        .rev_walk(tips)
        .with_hidden(hidden.iter().copied())
        .all()
        .map_err(|error| error.to_string())?;
    let mut count = 0;
    for info in walk {
        info.map_err(|error| error.to_string())?;
        count += 1;
    }
    Ok(count)
}

/// The tracked paths that differ from `HEAD`, staged or not, and the ones a
/// merge left unresolved: each once.
fn changed(repo: &gix::Repository) -> Result<Vec<String>, String> {
    let items = repo
        .status(gix::progress::Discard)
        .map_err(|error| error.to_string())?
        // What is not tracked is read by `not_committed`, whatever
        // `status.showUntrackedFiles` says: a setting that hides it from
        // `git status` does not keep it on the remote.
        .untracked_files(gix::status::UntrackedFiles::None)
        .index_worktree_rewrites(None)
        .index_worktree_submodules(gix::status::Submodule::AsConfigured { check_dirty: true })
        .into_iter(Vec::<gix::bstr::BString>::new())
        .map_err(|error| error.to_string())?;
    let mut paths = BTreeSet::new();
    for item in items {
        let item = item.map_err(|error| error.to_string())?;
        if let gix::status::Item::IndexWorktree(gix::status::index_worktree::Item::Modification {
            status: gix::status::plumbing::index_as_worktree::EntryStatus::NeedsUpdate(_),
            ..
        }) = &item
        {
            // Nothing changed; only the index's timestamps are behind.
            continue;
        }
        paths.insert(item.location().to_str_lossy().into_owned());
    }
    Ok(paths.into_iter().collect())
}

/// What git does not track, and what it ignores that no rule explains: a
/// folder with nothing tracked in it once, with a `/`. A path in a
/// submodule under `prefix` is explained by the paths of the working copy
/// around it.
fn not_committed(repo: &gix::Repository, prefix: &str, explained: &[&str]) -> Result<(Vec<String>, Vec<String>), String> {
    use gix::dir::entry::{Kind, Status};
    use gix::dir::walk::EmissionMode;

    if repo.workdir().is_none() {
        return Ok((Vec::new(), Vec::new()));
    }
    let index = repo.index_or_empty().map_err(|error| error.to_string())?;
    let options = repo
        .dirwalk_options()
        .map_err(|error| error.to_string())?
        .emit_untracked(EmissionMode::CollapseDirectory)
        .emit_ignored(Some(EmissionMode::CollapseDirectory))
        .emit_empty_directories(false)
        .recurse_repositories(false);
    let walk = repo
        .dirwalk_iter(index, Vec::<gix::bstr::BString>::new(), Default::default(), options)
        .map_err(|error| error.to_string())?;
    let (mut untracked, mut ignored) = (Vec::new(), Vec::new());
    for item in walk {
        let item = item.map_err(|error| error.to_string())?;
        let entry = item.entry;
        let folder = matches!(entry.disk_kind, Some(Kind::Directory | Kind::Repository));
        let mut path = entry.rela_path.to_str_lossy().into_owned();
        match entry.status {
            Status::Untracked => {
                if folder {
                    path.push('/');
                }
                untracked.push(path);
            }
            Status::Ignored(_) => {
                let whole = format!("{prefix}{path}");
                if !explained.iter().any(|by| covers(by, &whole)) {
                    if folder {
                        path.push('/');
                    }
                    ignored.push(path);
                }
            }
            Status::Tracked | Status::Pruned => {}
        }
    }
    untracked.sort();
    ignored.sort();
    Ok((untracked, ignored))
}

/// Whether the explained path `by` is `path` or a folder it is inside.
fn covers(by: &str, path: &str) -> bool {
    let path = path.trim_end_matches('/');
    let by = by.trim_end_matches('/');
    path.eq_ignore_ascii_case(by) || (path.len() > by.len() && path.as_bytes()[by.len()] == b'/' && path[..by.len()].eq_ignore_ascii_case(by))
}

/// How many entries the stash holds.
fn stashes(repo: &gix::Repository) -> usize {
    let Ok(Some(stash)) = repo.try_find_reference("refs/stash") else {
        return 0;
    };
    let mut platform = stash.log_iter();
    platform.all().ok().flatten().map_or(0, |lines| lines.filter(Result::is_ok).count())
}

/// What a reader calls an operation git left halfway.
const fn operation(state: gix::state::InProgress) -> &'static str {
    use gix::state::InProgress;
    match state {
        InProgress::Merge => "merge",
        InProgress::Rebase | InProgress::RebaseInteractive | InProgress::ApplyMailboxRebase => "rebase",
        InProgress::CherryPick | InProgress::CherryPickSequence => "cherry-pick",
        InProgress::Revert | InProgress::RevertSequence => "revert",
        InProgress::Bisect => "bisect",
        InProgress::ApplyMailbox => "patch series",
    }
}

/// Holds the time the files git reads first were last opened - the index,
/// the references, the packs, the root's `.gitignore` - while the working
/// copy is read. The loose objects a walk opens and the `.gitignore` files
/// further down are not held: see the ADR.
fn held(git: &Path, folder: &Path) -> Vec<crate::platform::AccessKept> {
    let mut files: Vec<PathBuf> = ["HEAD", "config", "index", "packed-refs", "FETCH_HEAD", "ORIG_HEAD"]
        .iter()
        .map(|name| git.join(name))
        .collect();
    files.push(folder.join(".gitignore"));
    files.push(folder.join(".gitattributes"));
    files.push(folder.join(".gitmodules"));
    if let Ok(packs) = std::fs::read_dir(git.join("objects").join("pack")) {
        files.extend(packs.flatten().map(|entry| entry.path()));
    }
    let mut seen = HashSet::new();
    files
        .into_iter()
        .filter(|path| path.is_file() && seen.insert(path.clone()))
        .map(|path| crate::platform::keeping_access_time(&path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(name: &str, url: &str) -> Option<Remote> {
        Some(Remote {
            name: name.to_owned(),
            url: url.to_owned(),
            here: false,
            heard: None,
        })
    }

    #[test]
    fn a_copy_wholly_on_its_remote_is_one_clone_away() {
        let now = SystemTime::now();
        let local = Local {
            remote: Some(Remote {
                heard: Some(now - std::time::Duration::from_secs(400 * 24 * 60 * 60)),
                ..remote("origin", "https://example.com/team/app.git").expect("a remote")
            }),
            ..Local::default()
        };
        let verdict = local.verdict("old app", now);
        assert_eq!(verdict.outcome, Some(Outcome::OnRemote));
        assert_eq!(
            verdict.about,
            "All of it is on origin at https://example.com/team/app.git, as of 13 months ago."
        );
        assert_eq!(verdict.command.as_deref(), Some("git clone https://example.com/team/app.git \"old app\""));
    }

    #[test]
    fn what_is_only_here_is_listed_and_nothing_is_one_clone_away() {
        let local = Local {
            remote: remote("origin", "git@example.com:team/app.git"),
            unpushed: 3,
            branches: vec!["spike".to_owned(), "wip".to_owned()],
            changed: vec!["Cargo.toml".to_owned(), "src/a.rs".to_owned(), "src/b.rs".to_owned(), "src/c.rs".to_owned()],
            untracked: vec!["notes.md".to_owned()],
            ignored: vec![".env".to_owned(), "data/".to_owned()],
            stashes: 1,
            operation: Some("merge"),
            ..Local::default()
        };
        assert!(!local.on_remote());
        let verdict = local.verdict("app", SystemTime::now());
        assert_eq!(verdict.outcome, None);
        assert_eq!(verdict.command, None);
        assert_eq!(
            verdict.about,
            "3 commits origin does not have, on spike, wip; 4 changes not committed: Cargo.toml, src/a.rs, src/b.rs and 1 more; \
             not tracked: notes.md; ignored, and nothing makes it again: .env, data/; 1 stash; a merge left halfway."
        );
    }

    #[test]
    fn each_thing_only_here_is_enough_to_keep_it_from_being_one_clone_away() {
        let clean = Local {
            remote: remote("origin", "https://example.com/a.git"),
            ..Local::default()
        };
        assert!(clean.on_remote());
        let unsettled: [fn(&mut Local); 10] = [
            |local| local.remote = None,
            |local| local.remote.as_mut().expect("a remote").here = true,
            |local| local.unpushed = 1,
            |local| local.changed.push("a".to_owned()),
            |local| local.untracked.push("a".to_owned()),
            |local| local.ignored.push(".env".to_owned()),
            |local| local.stashes = 1,
            |local| local.operation = Some("rebase"),
            |local| local.worktrees.push("D:/elsewhere".to_owned()),
            |local| local.alone.push("lib/".to_owned()),
        ];
        for (at, unsettle) in unsettled.iter().enumerate() {
            let mut local = clean.clone();
            unsettle(&mut local);
            assert!(!local.on_remote(), "case {at}");
        }
        let unread = Local {
            unread: Some("its commits: broken".to_owned()),
            ..clean
        };
        assert!(!unread.on_remote(), "what could not be read is not known to be on the remote");
    }

    #[test]
    fn a_copy_with_no_remote_says_so_first() {
        let local = Local {
            unpushed: 12,
            branches: vec!["main".to_owned()],
            ..Local::default()
        };
        assert_eq!(
            local.verdict("x", SystemTime::now()).about,
            "It has no remote, so nothing in it is anywhere else; 12 commits on main."
        );
    }

    #[test]
    fn an_explained_path_covers_itself_and_what_is_in_it_and_nothing_beside() {
        assert!(covers("node_modules", "node_modules/"));
        assert!(covers("web/node_modules", "web/node_modules/.cache/"));
        assert!(covers("target", "TARGET"));
        assert!(!covers("target", "target-old/"));
        assert!(!covers("web/node_modules", "web/"), "the folder around it holds more than it");
    }

    /// A working copy made by git itself in a temporary folder, its
    /// remote an address on the web and its branch as if pushed there: the
    /// remote-tracking branch is set to where the branch is, which is all a
    /// push leaves behind in the copy.
    struct Checkout {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    impl Checkout {
        fn new() -> Option<Self> {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().join("app");
            std::fs::create_dir(&root).expect("folder");
            let copy = Self { _dir: dir, root };
            if !copy.git(&["init", "--quiet", "--initial-branch=main"]) {
                eprintln!("git is not on the search path: skipped");
                return None;
            }
            copy.write("src/main.rs", "fn main() {}\n");
            copy.write(".gitignore", "node_modules/\n.env\n");
            copy.commit("first");
            assert!(copy.git(&["remote", "add", "origin", "https://ada:secret@example.com/team/app.git"]));
            copy.pushed();
            Some(copy)
        }

        fn git(&self, args: &[&str]) -> bool {
            std::process::Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(&self.root)
                .output()
                .is_ok_and(|output| output.status.success())
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.root.join(path);
            std::fs::create_dir_all(path.parent().expect("a folder")).expect("folders");
            std::fs::write(path, text).expect("write");
        }

        fn commit(&self, message: &str) {
            assert!(self.git(&["add", "--all"]));
            assert!(self.git(&["commit", "--quiet", "-m", message]));
        }

        fn pushed(&self) {
            assert!(self.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]));
        }

        fn read(&self, explained: &[&str]) -> Local {
            let explained: Vec<String> = explained.iter().map(|path| (*path).to_owned()).collect();
            read(&self.root, &explained).expect("a working copy")
        }
    }

    #[test]
    fn a_pushed_copy_whose_ignored_folder_a_rule_explains_is_on_its_remote() {
        let Some(copy) = Checkout::new() else { return };
        copy.write("node_modules/a/index.js", "x");
        let local = copy.read(&["node_modules"]);
        assert!(local.on_remote(), "{local:?}");
        let remote = local.remote.as_ref().expect("a remote");
        assert_eq!(remote.url, "https://example.com/team/app.git", "no user and no password in what is shown");
        assert!(!remote.here);

        let unexplained = copy.read(&[]);
        assert_eq!(unexplained.ignored, ["node_modules/"], "ignored, and no rule says it comes back");
        assert!(!unexplained.on_remote());
    }

    #[test]
    fn a_commit_not_pushed_is_counted_on_its_branch() {
        let Some(copy) = Checkout::new() else { return };
        copy.write("src/more.rs", "\n");
        copy.commit("second");
        let local = copy.read(&[]);
        assert_eq!((local.unpushed, local.branches.as_slice()), (1, ["main".to_owned()].as_slice()));
        copy.pushed();
        assert_eq!(copy.read(&[]).unpushed, 0);
    }

    #[test]
    fn changes_untracked_files_ignored_secrets_and_stashes_are_each_only_here() {
        let Some(copy) = Checkout::new() else { return };
        copy.write("src/main.rs", "fn main() { changed() }\n");
        copy.write("notes/today.md", "x");
        copy.write(".env", "TOKEN=x");
        let local = copy.read(&[]);
        assert_eq!(local.changed, ["src/main.rs"]);
        assert_eq!(local.untracked, ["notes/"], "a folder with nothing tracked in it, once");
        assert_eq!(local.ignored, [".env"]);
        assert!(copy.git(&["stash", "push", "--quiet"]));
        let stashed = copy.read(&[]);
        assert_eq!((stashed.stashes, stashed.changed.len()), (1, 0));
        assert!(!stashed.on_remote());
    }

    #[test]
    fn a_setting_that_hides_untracked_files_from_git_status_does_not_hide_them_here() {
        let Some(copy) = Checkout::new() else { return };
        assert!(copy.git(&["config", "status.showUntrackedFiles", "no"]));
        copy.write("draft.md", "x");
        assert_eq!(copy.read(&[]).untracked, ["draft.md"]);
    }

    #[test]
    fn a_remote_in_a_folder_of_this_computer_is_not_somewhere_else() {
        let Some(copy) = Checkout::new() else { return };
        assert!(copy.git(&["remote", "set-url", "origin", "file:///D:/bare/app.git"]));
        let local = copy.read(&[]);
        assert!(local.remote.as_ref().expect("a remote").here);
        assert!(!local.on_remote());
    }

    #[test]
    fn a_folder_whose_git_is_a_file_is_another_repositorys_and_not_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".git"), "gitdir: ../elsewhere/.git/worktrees/app\n").expect("write");
        assert_eq!(read(dir.path(), &[]), None);
        let empty = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(empty.path().join(".git")).expect("folder");
        assert_eq!(read(empty.path(), &[]), None, "a .git folder git would not open is no working copy");
    }

    #[test]
    fn a_word_with_a_space_is_quoted_for_the_shell() {
        assert_eq!(argument("app"), "app");
        assert_eq!(argument("old app"), "\"old app\"");
        assert_eq!(argument("R&D"), "\"R&D\"");
    }
}
