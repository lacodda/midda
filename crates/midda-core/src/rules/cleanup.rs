//! A cache cleaned by its own tool rather than deleted.
//!
//! A package manager knows its cache better than a folder delete does:
//! `pnpm store prune` removes the packages no project links any more and
//! leaves the ones in use, `npm cache clean` empties the cache and leaves npm
//! knowing it did. So for a cache whose tool cleans it, midda says how much
//! can come back and runs the tool's own command.
//!
//! Only after asking the tool where it cleans. A scan of `C:\` finds every
//! profile's npm cache, and `npm cache clean` empties the cache of whoever
//! runs it — run against the folder of another profile, it would clean the
//! wrong one and the reader would watch nothing come back. The tool is asked
//! first (`npm config get cache`, `pnpm store path`), and the command runs
//! only when the answer is the folder found. The same holds for a store of
//! an older layout: pnpm 10 prunes `store\v10`, not the `store\v3` an older
//! pnpm left beside it.
//!
//! A tool is looked for on the search path the caller gives, never in the
//! current directory, and is run with no input, no window and no shell
//! between: the command is the rule's, word for word. It runs in the folder
//! above the cache, not in whatever folder midda was started from: a
//! package manager reads the project it is started in, and one started in a
//! project that asks for another version of it goes to fetch that version
//! first (found on screen: `pnpm store path` did not answer in thirty
//! seconds from midda's own checkout, and answered in half a second from
//! beside the store).

use std::ffi::OsStr;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::fresh::store::same_folder;

/// How long a tool has to say where its cache is. `npm` takes a second on a
/// cold start; a tool that has not answered in this is not going to.
const LOCATE_WITHIN: Duration = Duration::from_secs(30);

/// How much of what a cleanup printed is kept to show: the end, where a
/// tool says what it did or why it did not.
const OUTPUT_TAIL: usize = 2000;

/// A cache's own cleanup: the tool, the command that cleans, and the command
/// that says where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleanup {
    /// The ways to call the tool, the first found taken: `pip`, or `py -m
    /// pip` where only the launcher is on the path.
    tools: Vec<Vec<String>>,
    /// What cleans, after the tool: `store prune`.
    run: Vec<String>,
    /// What prints the folder it cleans, after the tool: `store path`.
    locate: Vec<String>,
    /// What the command does, said to the reader.
    pub explain: String,
}

/// The shape of a cleanup in a rules file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Raw {
    tools: Vec<Vec<String>>,
    run: Vec<String>,
    locate: Vec<String>,
    explain: String,
}

impl Raw {
    pub(super) fn check(self) -> Result<Cleanup, String> {
        if self.tools.is_empty() || self.tools.iter().any(|tool| tool.first().is_none_or(|program| program.trim().is_empty())) {
            return Err("a cleanup needs a tool to call".to_owned());
        }
        if self.run.is_empty() || self.locate.is_empty() {
            return Err("a cleanup needs both the command that cleans and the one that says where".to_owned());
        }
        if self.explain.trim().is_empty() {
            return Err("a cleanup explains what it does".to_owned());
        }
        Ok(Cleanup {
            tools: self.tools,
            run: self.run,
            locate: self.locate,
            explain: self.explain,
        })
    }
}

/// A tool found on the search path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tool {
    /// The file that runs.
    program: PathBuf,
    /// The words the rule wrote, the first being the tool's name.
    words: Vec<String>,
}

impl Tool {
    /// The tool with `args`, to run from beside `folder`.
    fn command(&self, args: &[String], folder: &Path) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.words[1..])
            .args(args)
            .current_dir(folder.parent().unwrap_or(folder))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            // CREATE_NO_WINDOW: a console tool started from a window would
            // otherwise flash a console of its own.
            command.creation_flags(0x0800_0000);
        }
        command
    }

    fn line(&self, args: &[String]) -> String {
        self.words.iter().chain(args).map(String::as_str).collect::<Vec<_>>().join(" ")
    }
}

/// Whether a cleanup would run on a folder, as the window shows it before
/// asking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reach {
    /// The command as it would run: `pnpm store prune`.
    pub line: String,
    /// The folder the tool says it cleans, when it said.
    pub cleans: Option<PathBuf>,
    /// Why it would not run on this folder, when it would not.
    pub refused: Option<String>,
}

/// What a cleanup did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ran {
    /// The command as it ran.
    pub line: String,
    /// Whether the tool said it succeeded.
    pub succeeded: bool,
    /// The end of what it printed, both streams.
    pub output: String,
}

impl Cleanup {
    /// The command as a reader would type it with the first of the tools:
    /// what the window offers before anything is looked up.
    #[must_use]
    pub fn line(&self) -> String {
        self.tools[0].iter().chain(&self.run).map(String::as_str).collect::<Vec<_>>().join(" ")
    }

    /// The first of the tools that is on `search` — a `PATH`.
    fn tool(&self, search: Option<&OsStr>) -> Option<Tool> {
        self.tools
            .iter()
            .find_map(|words| find_program(&words[0], search?).map(|program| Tool { program, words: words.clone() }))
    }

    /// Whether the cleanup reaches `folder`: the tool is there, and says the
    /// folder it cleans is this one. Runs the tool, which only prints.
    #[must_use]
    pub fn reach(&self, folder: &Path, search: Option<&OsStr>) -> Reach {
        match self.located(folder, search) {
            Ok((tool, cleans)) => Reach {
                line: tool.line(&self.run),
                cleans: Some(cleans),
                refused: None,
            },
            Err((line, cleans, why)) => Reach {
                line,
                cleans,
                refused: Some(why),
            },
        }
    }

    /// The tool, and where it cleans, when that is `folder`; otherwise the
    /// line it would have run, where it cleans if it said, and why not.
    fn located(&self, folder: &Path, search: Option<&OsStr>) -> Result<(Tool, PathBuf), (String, Option<PathBuf>, String)> {
        let Some(tool) = self.tool(search) else {
            let names: Vec<&str> = self.tools.iter().map(|words| words[0].as_str()).collect();
            return Err((self.line(), None, format!("{} is not on this machine's search path", names.join(" or "))));
        };
        let line = tool.line(&self.run);
        let asked = tool.line(&self.locate);
        let output = output_of(tool.command(&self.locate, folder), Some(LOCATE_WITHIN)).map_err(|why| (line.clone(), None, format!("{asked} {why}")))?;
        if !output.status.success() {
            return Err((line, None, format!("{asked} failed: {}", tail(&output, 300))));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let Some(said) = stdout.lines().map(str::trim).rfind(|line| !line.is_empty()) else {
            return Err((line, None, format!("{asked} printed nothing")));
        };
        let cleans = PathBuf::from(said);
        if !same_folder(&cleans, folder) {
            return Err((
                line,
                Some(cleans.clone()),
                format!("this {} cleans {}, not this folder", words_of(&tool), cleans.display()),
            ));
        }
        Ok((tool, cleans))
    }

    /// Runs the cleanup on `folder`, after asking the tool again that this
    /// is the folder it cleans — the answer given when the window asked may
    /// be minutes old.
    ///
    /// # Errors
    ///
    /// When the tool is not there, does not clean this folder, or could not
    /// be started. A tool that ran and failed is not an error: it is a
    /// [`Ran`] that did not succeed, with what the tool said.
    pub fn run(&self, folder: &Path, search: Option<&OsStr>) -> Result<Ran, String> {
        let (tool, _) = self.located(folder, search).map_err(|(_, _, why)| why)?;
        let line = tool.line(&self.run);
        let output = output_of(tool.command(&self.run, folder), None).map_err(|why| format!("{line} {why}"))?;
        Ok(Ran {
            line,
            succeeded: output.status.success(),
            output: tail(&output, OUTPUT_TAIL),
        })
    }
}

/// The tool as the reader knows it: `pnpm`, `py -m pip`.
fn words_of(tool: &Tool) -> String {
    tool.words.join(" ")
}

/// Where `name` is on `search`, the way a shell would find it — in the
/// folders listed, never in the current one: a tool picked up from whatever
/// folder midda was started in is a tool nobody chose.
fn find_program(name: &str, search: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(search)
        .filter(|folder| folder.is_absolute())
        .find_map(|folder| candidates(name).into_iter().map(|file| folder.join(file)).find(|path| runnable(path)))
}

/// The file names `name` runs as.
fn candidates(name: &str) -> Vec<String> {
    if cfg!(windows) && Path::new(name).extension().is_none() {
        // The default PATHEXT, in its order: `npm` is `npm.cmd`, and the
        // extensionless `npm` beside it is a shell script Windows cannot run.
        [".com", ".exe", ".bat", ".cmd"].iter().map(|extension| format!("{name}{extension}")).collect()
    } else {
        vec![name.to_owned()]
    }
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    path.is_file()
}

/// Runs `command` to the end — or until `within` has passed, when it is
/// given — and returns what it printed.
///
/// Both streams are read while it runs, each by a thread of its own: a tool
/// that fills one pipe while nobody reads it waits forever.
fn output_of(mut command: Command, within: Option<Duration>) -> Result<Output, String> {
    let mut child = command.spawn().map_err(|error| format!("could not be started: {error}"))?;
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>));
    let stderr = drain(child.stderr.take().map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| format!("could not be waited for: {error}"))? {
            break status;
        }
        if within.is_some_and(|within| started.elapsed() > within) {
            let _ = child.kill();
            let _ = child.wait();
            // The readers are left to finish on their own: a tool started
            // through a script can leave a process behind holding the pipes.
            return Err(format!("did not answer in {} s", within.map_or(0, |within| within.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// The last `limit` bytes of what a tool printed, both streams, cut at a
/// character.
fn tail(output: &Output, limit: usize) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let errors = String::from_utf8_lossy(&output.stderr);
    if !errors.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(errors.trim());
    }
    if text.len() <= limit {
        return text;
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::rules::RuleSet;

    /// A tool called `faketool` in a folder of its own: `where` prints
    /// `cache`, `here` the folder it runs in, `clean` deletes the file `blob`
    /// in the cache, anything else fails.
    struct Fake {
        _folder: tempfile::TempDir,
        search: OsString,
    }

    fn fake(cache: &Path) -> Fake {
        let folder = tempfile::tempdir().expect("a folder for the tool");
        let shown = cache.display();
        if cfg!(windows) {
            let script = format!(
                "@echo off\r\nif \"%~1\"==\"here\" (\r\n  echo %CD%\r\n  exit /b 0\r\n)\r\nif \"%~1\"==\"where\" (\r\n  echo {shown}\r\n  exit /b 0\r\n)\r\nif \"%~1\"==\"clean\" (\r\n  del /q \"{shown}\\blob\"\r\n  echo cleaned\r\n  exit /b 0\r\n)\r\necho unknown 1>&2\r\nexit /b 3\r\n"
            );
            std::fs::write(folder.path().join("faketool.cmd"), script).expect("write the tool");
        } else {
            let script = format!(
                "#!/bin/sh\ncase \"$1\" in\n  here) pwd ;;\n  where) echo '{shown}' ;;\n  clean) rm -f '{shown}/blob'; echo cleaned ;;\n  *) echo unknown >&2; exit 3 ;;\nesac\n"
            );
            let path = folder.path().join("faketool");
            std::fs::write(&path, script).expect("write the tool");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("make it runnable");
            }
        }
        let search = std::env::join_paths([folder.path()]).expect("a search path");
        Fake { _folder: folder, search }
    }

    fn cleanup(tools: &str, run: &str) -> Cleanup {
        located(tools, run, "where")
    }

    fn located(tools: &str, run: &str, locate: &str) -> Cleanup {
        let text = format!(
            "[[rule]]\nid = \"fake\"\nname = \"fake\"\ngroup = \"caches\"\npaths = [\"fake-cache\"]\n\
             [[rule.restore]]\nrisk = \"safe\"\nexplain = \"A cache.\"\n\
             [rule.cleanup]\ntools = {tools}\nrun = {run}\nlocate = [\"{locate}\"]\nexplain = \"It empties itself.\"\n"
        );
        let rules = RuleSet::parse(&text).expect("the rule reads");
        rules.rules()[0].cleanup.clone().expect("a cleanup")
    }

    #[test]
    fn the_first_tool_found_is_asked_and_runs_only_on_the_folder_it_names() {
        let cache = tempfile::tempdir().expect("a cache");
        std::fs::write(cache.path().join("blob"), b"bytes").expect("write");
        let fake = fake(cache.path());
        let cleanup = cleanup("[[\"not-a-tool-anywhere\"], [\"faketool\"]]", "[\"clean\"]");

        let reach = cleanup.reach(cache.path(), Some(&fake.search));
        assert_eq!(reach.refused, None, "{reach:?}");
        assert_eq!(reach.line, "faketool clean", "the tool found, not the first written");
        assert!(reach.cleans.as_deref().is_some_and(|cleans| same_folder(cleans, cache.path())));

        let other = tempfile::tempdir().expect("another folder");
        let refused = cleanup.reach(other.path(), Some(&fake.search)).refused.expect("refused");
        assert!(refused.contains("not this folder"), "{refused}");
        assert!(cleanup.run(other.path(), Some(&fake.search)).is_err(), "and does not run there");
        assert!(cache.path().join("blob").exists(), "nothing was cleaned by the refusal");

        let ran = cleanup.run(cache.path(), Some(&fake.search)).expect("it runs");
        assert!(ran.succeeded, "{ran:?}");
        assert!(ran.output.contains("cleaned"), "{ran:?}");
        assert!(!cache.path().join("blob").exists(), "the tool cleaned its cache");
    }

    #[test]
    fn a_tool_that_is_not_there_is_said_by_name() {
        let cache = tempfile::tempdir().expect("a cache");
        let cleanup = cleanup("[[\"not-a-tool-anywhere\"], [\"nor-this\"]]", "[\"clean\"]");
        let reach = cleanup.reach(cache.path(), Some(OsStr::new("")));
        assert_eq!(reach.line, "not-a-tool-anywhere clean");
        let refused = reach.refused.expect("refused");
        assert!(refused.contains("not-a-tool-anywhere or nor-this"), "{refused}");
        assert!(cleanup.reach(cache.path(), None).refused.is_some(), "no search path, no tool");
    }

    #[test]
    fn a_tool_that_fails_is_a_run_that_did_not_succeed_with_what_it_said() {
        let cache = tempfile::tempdir().expect("a cache");
        let fake = fake(cache.path());
        let cleanup = cleanup("[[\"faketool\"]]", "[\"explode\"]");
        let ran = cleanup.run(cache.path(), Some(&fake.search)).expect("it ran");
        assert!(!ran.succeeded);
        assert!(ran.output.contains("unknown"), "{ran:?}");
    }

    #[test]
    fn a_tool_runs_from_beside_the_folder_not_from_wherever_midda_was_started() {
        let above = tempfile::tempdir().expect("a folder");
        let cache = above.path().join("cache");
        std::fs::create_dir(&cache).expect("a cache");
        let fake = fake(&cache);
        // Asked where it is, the tool says the folder it was started in.
        let reach = located("[[\"faketool\"]]", "[\"clean\"]", "here").reach(&cache, Some(&fake.search));
        let ran_in = reach.cleans.expect("it answered");
        // Canonical on both sides: a runner's temporary folder is spelled
        // with a short name, and the shell answers with the long one.
        let real = |path: &Path| std::fs::canonicalize(path).expect("a real folder");
        assert_eq!(real(&ran_in), real(above.path()), "the tool ran beside the cache");
    }

    #[test]
    fn the_current_folder_is_never_searched() {
        // Tests run in the crate's folder, so `src/rules/mod.rs` is there to
        // be found by a relative entry - and must not be.
        assert!(find_program("mod.rs", OsStr::new("src/rules")).is_none());
        assert!(find_program("mod.rs", OsStr::new(".")).is_none());
        // The same file by an absolute entry is found where any file runs: a
        // relative one was refused, not missed.
        if cfg!(windows) {
            let absolute = std::fs::canonicalize("src/rules").expect("the crate's own folder");
            assert!(find_program("mod.rs", absolute.as_os_str()).is_some());
        }
    }

    #[test]
    fn the_shipped_cleanups_are_the_tools_own_commands() {
        let lines: Vec<String> = RuleSet::shipped()
            .rules()
            .iter()
            .filter_map(|rule| rule.cleanup.as_ref().map(Cleanup::line))
            .collect();
        assert_eq!(lines, ["npm cache clean --force", "pnpm store prune", "pip cache purge"]);
    }

    #[test]
    fn what_a_tool_printed_is_kept_from_its_end() {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt as _;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt as _;
        let output = Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: "a".repeat(10).into_bytes(),
            stderr: "défaut".as_bytes().to_vec(),
        };
        assert_eq!(tail(&output, 100), format!("{}\ndéfaut", "a".repeat(10)));
        // Five bytes from the end is the middle of the `é`: the cut moves on
        // to the next character rather than splitting it.
        assert_eq!(tail(&output, 5), "…faut");
    }
}
