//! What `rules/system.toml` says, and the parts of the format it is written
//! with: files, ages, what goes, empty folders, whose an entry is, and checks.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::*;
use crate::checks::installer::Verdict;
use crate::consumers::{self, Consumer};
use crate::tree::{Kind, Node};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// The top of a volume as the platform writes it.
const VOLUME: &str = if cfg!(windows) { "C:/" } else { "/" };

/// The moment the tests look from.
fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + DAY * 20_000
}

/// A volume from paths, each `(path, bytes, days since written)`: a trailing
/// `/` is a folder, anything else a file. A folder's own time is its
/// `days`; `None` is a time the volume did not give.
fn volume(entries: &[(&str, u64, Option<u64>)]) -> Tree {
    let mut tree = Tree::new(PathBuf::from(VOLUME), Some(4096));
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|(path, _, _)| path.trim_end_matches('/').matches('/').count());
    for (path, bytes, days) in sorted {
        let folder = path.ends_with('/');
        let path = path.trim_end_matches('/');
        let (above, name) = path.rsplit_once('/').unwrap_or(("", path));
        let parent = above
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(ROOT, |at, part| tree.child(at, part).expect("the folder above comes first"));
        let written = days.map(|days| now() - DAY * u32::try_from(days).expect("a test age"));
        let node = Node {
            size: if folder { Size::zero() } else { Size::flat(bytes) },
            modified: written,
            subtree_modified: written,
            ..Node::new(name.into(), parent, if folder { Kind::Directory } else { Kind::File })
        };
        tree.insert(parent, node);
    }
    tree.roll_up();
    tree
}

fn at(tree: &Tree, path: &str) -> NodeId {
    path.split('/')
        .filter(|part| !part.is_empty())
        .fold(ROOT, |at, part| tree.child(at, part).unwrap_or_else(|| panic!("{path} is in the tree")))
}

/// What each finding is called and where, as `rule-id path`.
fn said(rules: &RuleSet, tree: &Tree, findings: &[Finding]) -> Vec<String> {
    let mut said: Vec<String> = findings
        .iter()
        .map(|finding| {
            let path = tree.path_of(finding.id).to_string_lossy().replace('\\', "/");
            format!("{} {}", rules.rule_of(finding).id, path.trim_start_matches(VOLUME))
        })
        .collect();
    said.sort();
    said
}

fn survey(tree: &Tree) -> Survey {
    RuleSet::shipped().survey_at(tree, now())
}

#[test]
fn a_rule_names_files_where_it_says_files() {
    let tree = volume(&[
        ("Windows/", 0, Some(1)),
        ("Windows/MEMORY.DMP", 800, Some(1)),
        ("Users/", 0, Some(1)),
        ("Users/me/", 0, Some(1)),
        ("Users/me/Documents/", 0, Some(1)),
        ("Users/me/Documents/MEMORY.DMP", 900, Some(1)),
        ("Data/", 0, Some(1)),
        ("Data/Windows/", 0, Some(1)),
        ("Data/Windows/MEMORY.DMP/", 0, Some(1)),
        ("Data/Windows/MEMORY.DMP/x", 10, Some(1)),
    ]);
    let rules = RuleSet::shipped();
    assert_eq!(said(rules, &tree, &survey(&tree).findings), ["memory-dump Windows/MEMORY.DMP"]);

    let folder = volume(&[
        ("Windows/", 0, Some(1)),
        ("Windows/MEMORY.DMP/", 0, Some(1)),
        ("Windows/MEMORY.DMP/x", 10, Some(1)),
    ]);
    assert!(survey(&folder).findings.is_empty(), "a folder by a file's name is not the file");
    let file = volume(&[
        ("Projects/", 0, Some(1)),
        ("Projects/app/", 0, Some(1)),
        ("Projects/app/Cargo.toml", 1, Some(1)),
        ("Projects/app/target", 10, Some(1)),
    ]);
    assert!(survey(&file).findings.is_empty(), "nor a file by a folder's");
}

#[test]
fn an_age_is_what_nothing_in_an_entry_was_written_for() {
    let tree = volume(&[
        ("Users/", 0, Some(400)),
        ("Users/me/", 0, Some(400)),
        ("Users/me/Downloads/", 0, Some(1)),
        ("Users/me/Downloads/old.zip", 500, Some(200)),
        ("Users/me/Downloads/fresh.zip", 500, Some(10)),
        ("Users/me/Downloads/timeless.zip", 500, None),
        // A folder made long ago with one file written into it lately is as
        // young as that file.
        ("Users/me/Downloads/photos/", 0, Some(300)),
        ("Users/me/Downloads/photos/a.jpg", 100, Some(300)),
        ("Users/me/Downloads/photos/b.jpg", 100, Some(2)),
        ("Users/me/Downloads/archive/", 0, Some(300)),
        ("Users/me/Downloads/archive/a.jpg", 100, Some(300)),
        // How the shell shows the folder: written once, years ago.
        ("Users/me/Downloads/desktop.ini", 288, Some(900)),
    ]);
    let rules = RuleSet::shipped();
    assert_eq!(
        said(rules, &tree, &survey(&tree).findings),
        ["old-download Users/me/Downloads/archive", "old-download Users/me/Downloads/old.zip"]
    );
    let finding = survey(&tree)
        .findings
        .into_iter()
        .find(|finding| finding.id == at(&tree, "Users/me/Downloads/archive"))
        .expect("found");
    assert_eq!((finding.entries, finding.goes), (2, Goes::Whole), "the folder and its file go");
    assert_eq!(rules.restore_of(&finding).risk, Risk::Caution, "a download is the reader's");
}

#[test]
fn a_temporary_folder_is_emptied_of_what_is_old_and_stays() {
    let tree = volume(&[
        ("Users/", 0, Some(1)),
        ("Users/me/", 0, Some(1)),
        ("Users/me/AppData/", 0, Some(1)),
        ("Users/me/AppData/Local/", 0, Some(1)),
        ("Users/me/AppData/Local/Temp/", 0, Some(1)),
        ("Users/me/AppData/Local/Temp/old.tmp", 3000, Some(30)),
        ("Users/me/AppData/Local/Temp/unpacked/", 0, Some(20)),
        ("Users/me/AppData/Local/Temp/unpacked/setup.exe", 5000, Some(20)),
        ("Users/me/AppData/Local/Temp/in-use.tmp", 7000, Some(1)),
        ("Windows/", 0, Some(1)),
        ("Windows/Temp/", 0, Some(1)),
        ("Windows/Temp/today.log", 100, Some(0)),
    ]);
    let rules = RuleSet::shipped();
    let findings = survey(&tree).findings;
    assert_eq!(
        said(rules, &tree, &findings),
        ["old-temporary-files Users/me/AppData/Local/Temp"],
        "nothing old in Windows' own"
    );
    let temp = findings[0];
    assert_eq!(temp.goes, Goes::Contents { taken: 2, of: 3 });
    assert_eq!((temp.holds.logical, temp.frees.logical), (8000, 8000), "what goes, not the folder");
    assert_eq!(temp.entries, 3, "a file, and a folder with one in it");
    let going = rules.going(&tree, &temp, now());
    let names: Vec<&str> = going.iter().map(|&id| tree.node(id).name.as_str()).collect();
    assert_eq!(names, ["old.tmp", "unpacked"]);
    assert_eq!(rules.detail(&temp, None).as_deref(), Some("2 of the 3 entries in it."));
    assert_eq!(rules.restore_of(&temp).risk, Risk::Safe);
    assert!(
        survey(&tree).units().is_empty(),
        "no tool makes it again: not coloured as a build, nor one line of growth"
    );
}

#[test]
fn the_recycle_bin_is_emptied_whole_and_belongs_to_windows() {
    let tree = volume(&[
        ("$Recycle.Bin/", 0, Some(1)),
        ("$Recycle.Bin/S-1-5-21-1-2-3-1001/", 0, Some(1)),
        ("$Recycle.Bin/S-1-5-21-1-2-3-1001/$IABC.txt", 100, Some(1)),
        ("$Recycle.Bin/S-1-5-21-1-2-3-1001/$RABC.txt", 9000, Some(400)),
        ("$Recycle.Bin/S-1-5-21-1-2-3-1001/desktop.ini", 100, Some(400)),
        ("$Recycle.Bin/S-1-5-18/", 0, Some(1)),
    ]);
    let rules = RuleSet::shipped();
    let survey = survey(&tree);
    assert_eq!(
        said(rules, &tree, &survey.findings),
        ["deleted-files $Recycle.Bin/S-1-5-21-1-2-3-1001"],
        "an empty bin is not one"
    );
    let bin = survey.findings[0];
    assert_eq!(bin.goes, Goes::Contents { taken: 2, of: 3 }, "the shell's desktop.ini stays");
    assert_eq!(rules.restore_of(&bin).risk, Risk::Caution, "deleted for good once emptied");
    assert_eq!(rules.detail(&bin, None), None, "a bin's entries are not its items");

    let classified = consumers::classify(rules, &tree, &survey);
    assert_eq!(classified.of(bin.id), Some(Consumer::System));
    assert_eq!(classified.frees(Consumer::System).logical, 9100);
}

#[test]
fn an_old_download_is_still_what_it_is() {
    let tree = volume(&[
        ("Users/", 0, Some(400)),
        ("Users/me/", 0, Some(400)),
        ("Users/me/Downloads/", 0, Some(1)),
        ("Users/me/Downloads/talk.mp4", 70_000, Some(200)),
    ]);
    let rules = RuleSet::shipped();
    let survey = survey(&tree);
    assert_eq!(said(rules, &tree, &survey.findings), ["old-download Users/me/Downloads/talk.mp4"]);
    let classified = consumers::classify(rules, &tree, &survey);
    assert_eq!(
        classified.of(at(&tree, "Users/me/Downloads/talk.mp4")),
        Some(Consumer::Media),
        "a video, old or not"
    );
    assert!(survey.units().is_empty(), "an old download is no unit of growth");
}

#[test]
fn an_empty_folder_is_a_leftover_only_among_the_reader_s_own_files() {
    let tree = volume(&[
        ("Users/", 0, Some(1)),
        ("Users/me/", 0, Some(1)),
        ("Users/me/Documents/", 0, Some(1)),
        ("Users/me/Documents/notes.txt", 10, Some(1)),
        ("Users/me/Documents/old project/", 0, Some(1)),
        ("Users/me/Documents/old project/src/", 0, Some(1)),
        ("Users/me/Documents/old project/src/empty too/", 0, Some(1)),
        ("Users/me/Documents/zero/", 0, Some(1)),
        ("Users/me/Documents/zero/placeholder", 0, Some(1)),
        ("Users/me/Documents/locked/", 0, Some(1)),
        ("Users/me/Pictures/", 0, Some(1)),
        ("Users/me/AppData/", 0, Some(1)),
        ("Users/me/AppData/Roaming/", 0, Some(1)),
        ("Users/me/AppData/Roaming/Tool/", 0, Some(1)),
        ("Users/me/AppData/Roaming/Tool/Logs/", 0, Some(1)),
        ("Projects/", 0, Some(1)),
        ("Projects/app/", 0, Some(1)),
        ("Projects/app/main.rs", 10, Some(1)),
        ("Projects/app/assets/", 0, Some(1)),
        ("Projects/app/.git/", 0, Some(1)),
        ("Projects/app/.git/HEAD", 20, Some(1)),
        ("Projects/app/.git/refs/", 0, Some(1)),
        ("Projects/app/.git/refs/tags/", 0, Some(1)),
        ("Program Files/", 0, Some(1)),
        ("Program Files/Tool/", 0, Some(1)),
        ("Program Files/Tool/plugins/", 0, Some(1)),
        // Empty on purpose since an update of 2025.
        ("inetpub/", 0, Some(1)),
        // OneDrive keeps the Documents it moved at its top, under the
        // reader's own word for them; an empty folder inside is a leftover.
        ("Users/me/OneDrive/", 0, Some(1)),
        ("Users/me/OneDrive/Документы/", 0, Some(1)),
        ("Users/me/OneDrive/Рабочий стол/", 0, Some(1)),
        ("Users/me/OneDrive/Рабочий стол/notes.txt", 10, Some(1)),
        ("Users/me/OneDrive/Рабочий стол/old/", 0, Some(1)),
    ]);
    let mut tree = tree;
    tree.skip(tree.path_of(at(&tree, "Users/me/Documents/locked")), "access is denied".to_owned());
    let rules = RuleSet::shipped();
    assert_eq!(
        said(rules, &tree, &survey(&tree).findings),
        [
            "empty-folder Projects/app/assets",
            "empty-folder Users/me/Documents/old project",
            "empty-folder Users/me/OneDrive/Рабочий стол/old"
        ],
        "the topmost empty folder, among documents and what nothing names; not a place itself (Pictures), \
         nor one with a file of nothing in it, nor one the scan could not read, nor a program's, nor git's"
    );
}

#[test]
fn a_shortcut_and_an_installer_wait_for_their_check() {
    let tree = volume(&[
        ("Users/", 0, Some(1)),
        ("Users/me/", 0, Some(1)),
        ("Users/me/Desktop/", 0, Some(1)),
        ("Users/me/Desktop/Tool.lnk", 2, Some(1)),
        ("Users/me/Desktop/Games/", 0, Some(1)),
        ("Users/me/Desktop/Games/Game.lnk", 2, Some(1)),
        ("Users/me/Desktop/notes.txt", 2, Some(1)),
        ("Users/me/Downloads/", 0, Some(1)),
        ("Users/me/Downloads/ToolSetup.exe", 90_000, Some(3)),
        // Old as well: still an installer first, the earlier rule.
        ("Users/me/Downloads/OldSetup.exe", 90_000, Some(200)),
    ]);
    let rules = RuleSet::shipped();
    let survey = survey(&tree);
    assert!(survey.findings.is_empty(), "nothing is said before the files are read");
    let mut waiting: Vec<(String, Check)> = survey
        .waiting
        .iter()
        .map(|waiting| (tree.node(waiting.finding.id).name.clone(), waiting.check))
        .collect();
    waiting.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        waiting,
        [
            ("Game.lnk".to_owned(), Check::Shortcut),
            ("OldSetup.exe".to_owned(), Check::Installer),
            ("Tool.lnk".to_owned(), Check::Shortcut),
            ("ToolSetup.exe".to_owned(), Check::Installer)
        ]
    );

    let shortcut = survey.waiting.iter().find(|waiting| waiting.check == Check::Shortcut).expect("a shortcut");
    assert_eq!(rules.settle(shortcut, &Seen::Fine), None, "a shortcut that works is none of the rule's");
    let broken = Seen::Broken {
        target: r"C:\Program Files\Tool\tool.exe".to_owned(),
    };
    let finding = rules.settle(shortcut, &broken).expect("a broken one is");
    assert_eq!((finding.group, rules.restore_of(&finding).risk), (Group::Leftovers, Risk::Safe));
    assert_eq!(
        rules.detail(&finding, Some(&broken)).as_deref(),
        Some(r"It points to C:\Program Files\Tool\tool.exe, which is not there.")
    );

    let installer = survey.waiting.iter().find(|waiting| waiting.check == Check::Installer).expect("an installer");
    let verdict = |outcome| Seen::Installer(Verdict { outcome, about: String::new() });
    let risk = |outcome| rules.restore_of(&rules.settle(installer, &verdict(outcome)).expect("always the rule's")).risk;
    assert_eq!(risk(Some(Outcome::Installed)), Risk::Safe);
    assert_eq!(risk(Some(Outcome::OlderInstalled)), Risk::Caution);
    assert_eq!(risk(None), Risk::Caution);
    let older = rules.settle(installer, &verdict(Some(Outcome::OlderInstalled))).expect("the rule's");
    assert!(
        rules.restore_of(&older).explain.contains("update not yet run"),
        "the way the answer names, not the last"
    );
}

#[test]
fn the_format_refuses_what_does_not_fit_together() {
    let rule = |fields: &str, restore: &str| format!("[[rule]]\nid = \"x\"\nname = \"x\"\ngroup = \"leftovers\"\npaths = [\"x\"]\n{fields}\n{restore}");
    let plain = "[[rule.restore]]\nrisk = \"safe\"\nexplain = \"A.\"\n";
    let refused = |fields: &str, restore: &str, why: &str| {
        let error = RuleSet::parse(&rule(fields, restore)).expect_err(fields);
        assert!(error.contains(why), "{fields}: {error}");
    };
    assert!(RuleSet::parse(&rule("entries = \"files\"\ncheck = \"shortcut\"", plain)).is_ok());
    refused("entries = \"files\"\ngoes = \"contents\"", plain, "only a folder is emptied");
    refused("entries = \"all\"\nhollow = true", plain, "only a folder is empty");
    refused("entries = \"files\"\ninside = [\"a\"]", plain, "nothing is inside a file");
    refused("check = \"installer\"", plain, "a check reads a file");
    refused("older = \"soon\"", plain, "not an age");
    refused("older = \"0 days\"", plain, "every entry");
    refused("older = \"7 fortnights\"", plain, "not an age");
    refused(
        "",
        "[[rule.restore]]\nif = \"installed\"\nrisk = \"safe\"\nexplain = \"A.\"\n[[rule.restore]]\nrisk = \"safe\"\nexplain = \"B.\"\n",
        "this rule has no check",
    );
    refused(
        "entries = \"files\"\ncheck = \"installer\"",
        "[[rule.restore]]\nwhen = \"a\"\nrisk = \"safe\"\nexplain = \"A.\"\n[[rule.restore]]\nrisk = \"safe\"\nexplain = \"B.\"\n",
        "`if`, not `when`",
    );
    refused(
        "entries = \"files\"\ncheck = \"shortcut\"",
        "[[rule.restore]]\nif = \"installed\"\nrisk = \"safe\"\nexplain = \"A.\"\n[[rule.restore]]\nrisk = \"safe\"\nexplain = \"B.\"\n",
        "never says `installed`",
    );
    refused(
        "entries = \"files\"\ncheck = \"installer\"",
        "[[rule.restore]]\nif = \"installed\"\nrisk = \"safe\"\nexplain = \"A.\"\n",
        "no `when` and no `if`",
    );
    refused("within = [\"nobody\"]", plain, "unknown variant");
    assert_eq!(read_age("2 weeks"), Ok(14 * DAY));
    assert_eq!(read_age("1 year"), Ok(365 * DAY));
}

#[test]
fn every_shipped_group_lists_in_one_order() {
    let rules = RuleSet::shipped();
    for rule in rules.rules() {
        assert!(Group::ALL.contains(&rule.group), "{}", rule.id);
    }
    let shipped: HashSet<Group> = rules.rules().iter().map(|rule| rule.group).collect();
    assert_eq!(
        shipped.len(),
        Group::ALL.len(),
        "every group has a rule, and no rule a group the window does not list"
    );
}

#[test]
fn the_story_says_what_of_windows_can_go_not_that_a_tool_makes_it() {
    let tree = volume(&[
        ("Windows/", 0, Some(1)),
        ("Windows/explorer.exe", 4_000_000_000, Some(1)),
        ("Windows/MEMORY.DMP", 2_000_000_000, Some(1)),
    ]);
    let rules = RuleSet::shipped();
    let classified = consumers::classify(rules, &tree, &survey(&tree));
    let story = consumers::story::tell(rules, &tree, &classified, None);
    assert!(story.text.contains("1.9 GB of it can go."), "{}", story.text);
    assert!(!story.text.contains("A tool makes"), "{}", story.text);
    assert!(story.brief.contains("; 1.9 GB of it can go"), "{}", story.brief);
}

#[test]
fn a_file_is_checked_again_only_when_it_changed() {
    let entries = |written: u64| {
        volume(&[
            ("Users/", 0, Some(1)),
            ("Users/me/", 0, Some(1)),
            ("Users/me/Desktop/", 0, Some(1)),
            ("Users/me/Desktop/Gone.lnk", 2, Some(written)),
            ("Users/me/Desktop/Works.lnk", 2, Some(1)),
        ])
    };
    let rules = RuleSet::shipped();
    let mut kept = crate::checks::Kept::default();
    let mut asked = Vec::new();
    let mut look = |entry: &Waiting| {
        let name = entry.path.file_name().expect("a name").to_string_lossy().into_owned();
        asked.push(name.clone());
        if name == "Gone.lnk" {
            Seen::Broken { target: "X".to_owned() }
        } else {
            Seen::Fine
        }
    };
    let tree = entries(5);
    let settled = kept.settle(rules, &survey(&tree).waiting, &mut look);
    assert_eq!(settled.len(), 1, "the broken one");
    assert_eq!(tree.node(settled[0].0.id).name, "Gone.lnk");
    kept.settle(rules, &survey(&tree).waiting, &mut look);
    let changed = entries(2);
    let again = kept.settle(rules, &survey(&changed).waiting, &mut look);
    assert_eq!(again.len(), 1, "what was seen is still said");
    asked.sort();
    assert_eq!(
        asked,
        ["Gone.lnk", "Gone.lnk", "Works.lnk"],
        "once each, and again only for the one written since"
    );
}

#[test]
fn a_scan_of_a_folder_knows_whose_the_folders_above_it_are() {
    let menu = format!("{VOLUME}Users/me/AppData/Roaming/Microsoft/Windows/Start Menu");
    let mut tree = Tree::new(PathBuf::from(&menu), Some(4096));
    let programs = tree.push(ROOT, Node::new("Programs".into(), ROOT, Kind::Directory));
    tree.push(programs, Node::new("Gone".into(), programs, Kind::Directory));
    tree.roll_up();
    assert!(survey(&tree).findings.is_empty(), "inside AppData, a program's empty folder is not a leftover");

    let mut documents = Tree::new(PathBuf::from(format!("{VOLUME}Users/me/Documents/work")), Some(4096));
    documents.push(ROOT, Node::new("empty".into(), ROOT, Kind::Directory));
    documents.roll_up();
    assert_eq!(survey(&documents).findings.len(), 1, "inside Documents, it is");

    let mut saves = Tree::new(PathBuf::from(format!("{VOLUME}Users/me/Documents/My Games/Game")), Some(4096));
    saves.push(ROOT, Node::new("slot".into(), ROOT, Kind::Directory));
    saves.roll_up();
    assert!(
        survey(&saves).findings.is_empty(),
        "the deepest place above says: a game's saves, though in Documents"
    );
}
