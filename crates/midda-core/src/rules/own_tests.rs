//! A reader's own rules beside the shipped ones, what they keep, the folders
//! the shell moves, and what a folder says of itself.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::pattern::KnownFolder;
use super::*;
use crate::checks::{self, Machine};
use crate::tree::{Kind, Node};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// The top of a volume as the platform writes it.
const VOLUME: &str = if cfg!(windows) { "C:/" } else { "/" };

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + DAY * 20_000
}

/// A tree at `root` from `(path, bytes, days since written)`: a trailing `/`
/// is a folder.
fn tree_at(root: &str, entries: &[(&str, u64, u64)]) -> Tree {
    let mut tree = Tree::new(PathBuf::from(root), Some(4096));
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
        let written = Some(now() - DAY * u32::try_from(days).expect("a test age"));
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

/// What each finding is, as `rule-id path`, in path order.
fn said(rules: &RuleSet, tree: &Tree, findings: &[Finding]) -> Vec<String> {
    let root = tree.root_path().to_string_lossy().replace('\\', "/");
    let mut said: Vec<String> = findings
        .iter()
        .map(|finding| {
            let path = tree.path_of(finding.id).to_string_lossy().replace('\\', "/");
            let path = path.strip_prefix(&root).unwrap_or(&path).trim_start_matches('/').to_owned();
            format!("{} {path}", rules.rule_of(finding).id)
        })
        .collect();
    said.sort();
    said
}

fn own(files: &[(&str, &str)]) -> (RuleSet, Vec<Refused>) {
    let files: Vec<(String, String)> = files.iter().map(|(name, text)| ((*name).to_owned(), (*text).to_owned())).collect();
    RuleSet::with_own(&files, &Known::in_profiles())
}

/// A project with a Cargo build and a pnpm install.
fn projects() -> Tree {
    tree_at(
        "C:/work",
        &[
            ("app/", 0, 1),
            ("app/Cargo.toml", 1, 1),
            ("app/target/", 0, 1),
            ("app/target/release/", 0, 1),
            ("app/target/release/app.exe", 9000, 1),
            ("app/target/debug/", 0, 1),
            ("app/target/debug/app.exe", 7000, 1),
            ("web/", 0, 1),
            ("web/package.json", 1, 1),
            ("web/pnpm-lock.yaml", 1, 1),
            ("web/node_modules/", 0, 1),
            ("web/node_modules/a.js", 5000, 1),
        ],
    )
}

#[test]
fn every_file_in_the_rules_folder_is_shipped_and_in_its_order() {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rules");
    let mut written: Vec<String> = std::fs::read_dir(&folder)
        .expect("the rules folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".toml"))
        .collect();
    written.sort();
    let mut shipped: Vec<String> = SHIPPED.iter().map(|(name, _)| (*name).to_owned()).collect();
    shipped.sort();
    assert_eq!(written, shipped, "a rules file a pull request adds is shipped only once SHIPPED names it");
    for (name, text) in SHIPPED {
        let on_disk = std::fs::read_to_string(folder.join(name)).expect("reads");
        assert_eq!(on_disk.replace("\r\n", "\n"), text.replace("\r\n", "\n"), "{name} is the file in rules/");
    }
}

#[test]
fn a_readers_rule_is_tried_before_midda_s() {
    let mine = r#"
[[rule]]
id = "release-builds"
name = "Release build I hand out"
group = "build"
paths = ["target"]
beside = ["Cargo.toml"]

[[rule.restore]]
risk = "caution"
explain = "My release builds: they take an hour to make again."
"#;
    let (rules, refused) = own(&[("mine.toml", mine)]);
    assert!(refused.is_empty(), "{refused:?}");
    let tree = projects();
    let findings = rules.find(&tree);
    assert_eq!(said(&rules, &tree, &findings), ["node-modules web/node_modules", "release-builds app/target"]);
    let target = findings.iter().find(|finding| rules.rule_of(finding).id == "release-builds").expect("mine");
    assert_eq!(rules.restore_of(target).risk, Risk::Caution);
}

#[test]
fn a_file_that_does_not_read_costs_only_itself() {
    let good = "[[keep]]\npaths = [\"app/target\"]\n";
    let broken = "[[rule]\nid = ";
    let taken_id = "[[rule]]\nid = \"cargo-target\"\nname = \"x\"\ngroup = \"build\"\npaths = [\"x\"]\n[[rule.restore]]\nrisk = \"safe\"\nexplain = \"A.\"\n";
    let silent = "[[rule]]\nid = \"quiet\"\nname = \"x\"\ngroup = \"build\"\npaths = [\"x\"]\nrestore = []\n";
    let (rules, refused) = own(&[("broken.toml", broken), ("taken.toml", taken_id), ("good.toml", good), ("silent.toml", silent)]);
    let names: Vec<&str> = refused.iter().map(|refused| refused.name.as_str()).collect();
    assert_eq!(names, ["broken.toml", "taken.toml", "silent.toml"]);
    assert!(refused[1].why.contains("midda's dev.toml"), "{}", refused[1].why);
    assert!(refused[2].why.contains("way back"), "{}", refused[2].why);
    assert_eq!(rules.keeps().len(), 1, "the good one is read");
    assert_eq!(rules.keeps()[0].file, "good.toml");

    // Two of the reader's files naming one id: the second is refused.
    let mine = "[[place]]\nid = \"stash\"\nname = \"my stash\"\nconsumer = \"media\"\npaths = [\"/Stash\"]\n";
    let (_, refused) = own(&[("a.toml", mine), ("b.toml", mine)]);
    assert_eq!(refused.len(), 1);
    assert!(refused[0].why.contains("a.toml"), "{}", refused[0].why);
}

#[test]
fn what_is_kept_is_never_a_finding_and_what_holds_it_never_goes_whole() {
    let tree = projects();
    let keep = |path: &str| own(&[("kept.toml", &format!("[[keep]]\npaths = [\"{path}\"]\nwhy = \"mine\"\n"))]).0;

    let whole = keep("work/app/target");
    assert_eq!(said(&whole, &tree, &whole.find(&tree)), ["node-modules web/node_modules"]);

    // The artifact inside: the build folder is Cargo's and stays one thing,
    // nobody's to delete.
    let inside = keep("app/target/release/app.exe");
    let survey = inside.survey_at(&tree, now());
    assert_eq!(said(&inside, &tree, &survey.findings), ["node-modules web/node_modules"]);
    assert!(survey.closed.iter().any(|closed| closed.id == at(&tree, "app/target")), "held, not looked into");
    assert!(survey.units().contains(&at(&tree, "app/target")), "still one thing a tool makes");

    // The folder opened is in a kept one: nothing in it goes.
    let all = keep("/work");
    if cfg!(windows) {
        assert!(all.find(&tree).is_empty());
    }
    let above = keep("work");
    assert!(above.find(&tree).is_empty(), "kept by the root's own name");

    assert_eq!(inside.keeps()[0].why.as_deref(), Some("mine"));
}

#[test]
fn a_folder_that_is_emptied_keeps_what_is_kept_in_it() {
    let tree = tree_at(
        VOLUME,
        &[
            ("Users/", 0, 400),
            ("Users/me/", 0, 400),
            ("Users/me/AppData/", 0, 400),
            ("Users/me/AppData/Local/", 0, 400),
            ("Users/me/AppData/Local/Temp/", 0, 1),
            ("Users/me/AppData/Local/Temp/old.log", 100, 30),
            ("Users/me/AppData/Local/Temp/stash/", 0, 30),
            ("Users/me/AppData/Local/Temp/stash/notes.txt", 100, 30),
        ],
    );
    let (rules, _) = own(&[("kept.toml", "[[keep]]\npaths = [\"Temp/stash/notes.txt\"]\n")]);
    let survey = rules.survey_at(&tree, now());
    let temp = survey
        .findings
        .iter()
        .find(|finding| rules.rule_of(finding).id == "old-temporary-files")
        .expect("Temp");
    assert_eq!(temp.goes, Goes::Contents { taken: 1, of: 2 }, "the folder holding the kept file stays");
    assert_eq!(rules.going(&tree, temp, now()), [at(&tree, "Users/me/AppData/Local/Temp/old.log")]);
}

#[test]
fn a_known_folder_is_wherever_the_shell_keeps_it() {
    let mut known = Known::in_profiles();
    assert_eq!(known.places(KnownFolder::Downloads), ["/Users/*/Downloads"]);
    known.also(KnownFolder::Downloads, std::path::Path::new(r"D:\Downloads"));
    if cfg!(windows) {
        assert_eq!(known.places(KnownFolder::Downloads), ["/Users/*/Downloads", "D:/Downloads"]);
    }

    let read = |path: &str| Template::read(path, &known).map(|paths| paths.iter().map(Template::as_string).collect::<Vec<_>>());
    if cfg!(windows) {
        assert_eq!(
            read("{downloads}/*.iso"),
            Ok(vec!["/Users/*/Downloads/*.iso".to_owned(), "D:/Downloads/*.iso".to_owned()])
        );
    }
    assert_eq!(
        read("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}/x"),
        Ok(vec!["{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}/x".to_owned()]),
        "a GUID is a name"
    );
    assert!(read("{inbox}/*").expect_err("unknown").contains("{downloads}"));
    assert!(read("{downloads}x").expect_err("glued").contains("followed by `/`"));
    assert!(Template::new("{downloads}/x").is_err(), "only a set places a known folder");

    if cfg!(windows) {
        let set = RuleSet::with_own(&[], &known).0;
        let moved = tree_at(
            "D:/",
            &[
                ("Downloads/", 0, 400),
                ("Downloads/ubuntu.iso", 9000, 400),
                ("Downloads/blob", 10, 1),
                ("Other/", 0, 400),
                ("Other/x.iso", 9000, 400),
                ("Other/blob", 10, 1),
            ],
        );
        assert_eq!(
            said(&set, &moved, &set.survey_at(&moved, now()).findings),
            ["disc-image Downloads/ubuntu.iso"],
            "Downloads moved to D: is Downloads, and nothing else on D: is"
        );
        let elsewhere = tree_at("E:/", &[("Downloads/", 0, 400), ("Downloads/ubuntu.iso", 9000, 400)]);
        assert!(set.survey_at(&elsewhere, now()).findings.is_empty(), "a path with a drive is that drive's");
        let classified = crate::consumers::classify(&set, &moved, &set.survey_at(&moved, now()));
        assert_eq!(
            classified.of(at(&moved, "Downloads/blob")),
            Some(crate::consumers::Consumer::Documents),
            "the place moved with it"
        );
        assert_eq!(classified.of(at(&moved, "Other/blob")), Some(crate::consumers::Consumer::Unrecognised));
    }
}

#[test]
fn a_path_with_a_drive_is_that_volumes_only() {
    let path = Template::new("D:/Archive/**/target").expect("a path");
    assert_eq!(path.as_string(), "D:/Archive/**/target");
    assert!(Template::new("d:/x").is_ok_and(|path| path.as_string() == "D:/x"), "a drive in either case");
    if cfg!(windows) {
        let tree = tree_at("D:/Archive", &[("old/", 0, 1), ("old/target/", 0, 1)]);
        let outer = Outer::of(tree.root_path());
        assert!(path.matches(&tree, at(&tree, "old/target"), &outer));
        let other = tree_at("C:/Archive", &[("old/", 0, 1), ("old/target/", 0, 1)]);
        assert!(!path.matches(&other, at(&other, "old/target"), &Outer::of(other.root_path())));
        assert_eq!(
            pattern::written_as_rule(std::path::Path::new(r"d:\Work\app\target")).as_deref(),
            Some("D:/Work/app/target")
        );
    }
    assert_eq!(pattern::written_as_rule(std::path::Path::new("relative/path")), None);
}

#[test]
fn a_folder_that_says_it_is_a_cache_waits_for_its_marker_to_be_read() {
    let tree = tree_at(
        "C:/work",
        &[
            ("tool/", 0, 1),
            ("tool/.pytest_cache/", 0, 1),
            ("tool/.pytest_cache/CACHEDIR.TAG", 43, 1),
            ("tool/.pytest_cache/v/", 0, 1),
            ("tool/.pytest_cache/v/data", 5000, 1),
            ("rust/", 0, 1),
            ("rust/Cargo.toml", 1, 1),
            ("rust/target/", 0, 1),
            ("rust/target/CACHEDIR.TAG", 43, 1),
            ("plain/", 0, 1),
            ("plain/data", 5000, 1),
        ],
    );
    let rules = RuleSet::shipped();
    let survey = rules.survey_at(&tree, now());
    assert_eq!(said(rules, &tree, &survey.findings), ["cargo-target rust/target"], "Cargo's build first");
    let waiting: Vec<(&str, &str)> = survey
        .waiting
        .iter()
        .map(|waiting| (rules.rule_of(&waiting.finding).id.as_str(), waiting.check.key()))
        .collect();
    assert_eq!(waiting, [("tagged-cache", "marker")]);
    assert_eq!(rules.settle(&survey.waiting[0], &Seen::Fine), None, "a tag without the signature is none of it");
    let found = rules.settle(&survey.waiting[0], &Seen::Marked).expect("a signed one is");
    assert_eq!((found.group, rules.restore_of(&found).risk), (Group::Caches, Risk::Safe));
}

#[test]
fn a_marker_is_read_byte_for_byte_and_only_as_far_as_it_needs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rules = RuleSet::shipped();
    let rule = rules.rules().iter().find(|rule| rule.id == "tagged-cache").expect("the rule");
    let check = rule.check.clone().expect("a marker");
    let tagged = dir.path().join("tagged");
    std::fs::create_dir(&tagged).expect("mkdir");
    std::fs::write(
        tagged.join("CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n# This file is a cache directory tag.\n",
    )
    .expect("write");
    assert_eq!(checks::look(&tagged, &check, &Machine::default()), Seen::Marked);

    let forged = dir.path().join("forged");
    std::fs::create_dir(&forged).expect("mkdir");
    std::fs::write(forged.join("CACHEDIR.TAG"), "Signature: 0000000000000000000000000000000\n").expect("write");
    assert_eq!(checks::look(&forged, &check, &Machine::default()), Seen::Fine);
    let short = dir.path().join("short");
    std::fs::create_dir(&short).expect("mkdir");
    std::fs::write(short.join("CACHEDIR.TAG"), "Signature:").expect("write");
    assert_eq!(checks::look(&short, &check, &Machine::default()), Seen::Fine, "shorter than the signature");
}

#[test]
fn the_recycle_bin_is_found_at_once_and_told_of_by_its_records() {
    let tree = tree_at(
        VOLUME,
        &[
            ("$Recycle.Bin/", 0, 1),
            ("$Recycle.Bin/S-1-5-21-1-2-3-1001/", 0, 1),
            ("$Recycle.Bin/S-1-5-21-1-2-3-1001/$IABC.txt", 100, 1),
            ("$Recycle.Bin/S-1-5-21-1-2-3-1001/$RABC.txt", 9000, 1),
        ],
    );
    let rules = RuleSet::shipped();
    let survey = rules.survey_at(&tree, now());
    assert_eq!(survey.findings.len(), 1, "a finding before anything is read");
    assert_eq!(survey.told.len(), 1);
    assert_eq!(survey.told[0].about, About::RecycleBin);
    assert_eq!(survey.told[0].finding, survey.findings[0]);
    let told = Seen::Bin {
        about: Some("1 thing deleted, today.".to_owned()),
    };
    assert_eq!(rules.detail(&survey.findings[0], Some(&told)).as_deref(), Some("1 thing deleted, today."));
}

#[test]
fn what_windows_keeps_and_a_browser_caches_are_findings_of_their_own() {
    let tree = tree_at(
        VOLUME,
        &[
            ("Windows.old/", 0, 30),
            ("Windows.old/Windows/", 0, 30),
            ("Windows.old/Windows/explorer.exe", 9000, 30),
            ("Windows/", 0, 1),
            ("Windows/SoftwareDistribution/", 0, 1),
            ("Windows/SoftwareDistribution/DataStore/", 0, 1),
            ("Windows/SoftwareDistribution/DataStore/DataStore.edb", 500, 1),
            ("Windows/SoftwareDistribution/Download/", 0, 1),
            ("Windows/SoftwareDistribution/Download/abc/", 0, 1),
            ("Windows/SoftwareDistribution/Download/abc/update.cab", 8000, 3),
            ("Users/", 0, 1),
            ("Users/me/", 0, 1),
            ("Users/me/AppData/", 0, 1),
            ("Users/me/AppData/Local/", 0, 1),
            ("Users/me/AppData/Local/Microsoft/", 0, 1),
            ("Users/me/AppData/Local/Microsoft/Windows/", 0, 1),
            ("Users/me/AppData/Local/Microsoft/Windows/Explorer/", 0, 1),
            ("Users/me/AppData/Local/Microsoft/Windows/Explorer/thumbcache_256.db", 7000, 1),
            ("Users/me/AppData/Local/Microsoft/Windows/Explorer/iconcache_16.db", 7000, 1),
            ("Users/me/AppData/Local/Google/", 0, 1),
            ("Users/me/AppData/Local/Google/Chrome/", 0, 1),
            ("Users/me/AppData/Local/Google/Chrome/User Data/", 0, 1),
            ("Users/me/AppData/Local/Google/Chrome/User Data/Profile 1/", 0, 1),
            ("Users/me/AppData/Local/Google/Chrome/User Data/Profile 1/Cache/", 0, 1),
            ("Users/me/AppData/Local/Google/Chrome/User Data/Profile 1/Cache/Cache_Data/", 0, 1),
            ("Users/me/AppData/Local/Google/Chrome/User Data/Profile 1/Cache/Cache_Data/data_1", 6000, 1),
            ("Users/me/AppData/Local/Google/Chrome/User Data/Profile 1/Bookmarks", 10, 1),
        ],
    );
    let rules = RuleSet::shipped();
    let survey = rules.survey_at(&tree, now());
    assert_eq!(
        said(rules, &tree, &survey.findings),
        [
            "browser-cache Users/me/AppData/Local/Google/Chrome/User Data/Profile 1/Cache",
            "thumbnail-cache Users/me/AppData/Local/Microsoft/Windows/Explorer/thumbcache_256.db",
            "windows-old Windows.old",
            "windows-update-downloads Windows/SoftwareDistribution/Download",
        ],
        "the cache, not the profile; the downloads, not the update's database"
    );
    let group = |id: &str| {
        survey
            .findings
            .iter()
            .find(|finding| rules.rule_of(finding).id == id)
            .map(|finding| (finding.group, rules.restore_of(finding).risk))
    };
    assert_eq!(group("windows-old"), Some((Group::Windows, Risk::Caution)), "the way back to the old Windows");
    assert_eq!(group("browser-cache"), Some((Group::BrowserCaches, Risk::Safe)));
    let classified = crate::consumers::classify(rules, &tree, &survey);
    assert_eq!(classified.of(at(&tree, "Windows.old")), Some(crate::consumers::Consumer::System));
}

#[test]
fn an_installer_saved_to_the_desktop_is_asked_about_as_one_in_downloads() {
    let tree = tree_at(
        VOLUME,
        &[
            ("Users/", 0, 1),
            ("Users/me/", 0, 1),
            ("Users/me/Desktop/", 0, 1),
            ("Users/me/Desktop/ToolSetup.exe", 90_000, 3),
        ],
    );
    let survey = RuleSet::shipped().survey_at(&tree, now());
    assert_eq!(survey.waiting.len(), 1);
    assert_eq!(survey.waiting[0].check, Check::Installer);
}

#[test]
fn the_format_refuses_a_marker_or_a_keep_that_says_nothing() {
    let rule = |fields: &str| {
        format!("[[rule]]\nid = \"x\"\nname = \"x\"\ngroup = \"caches\"\npaths = [\"*\"]\n{fields}\n[[rule.restore]]\nrisk = \"safe\"\nexplain = \"A.\"\n")
    };
    let refused = |text: &str, why: &str| {
        let error = RuleSet::parse(text).expect_err(text);
        assert!(error.contains(why), "{text}: {error}");
    };
    assert!(RuleSet::parse(&rule("[rule.marker]\nfile = \"TAG\"\nstarts = \"x\"")).is_ok());
    refused(&rule("[rule.marker]\nfile = \"*.TAG\"\nstarts = \"x\""), "is a pattern");
    refused(&rule("[rule.marker]\nfile = \"TAG\"\nstarts = \"\""), "cannot be empty");
    refused(&rule("entries = \"files\"\n[rule.marker]\nfile = \"TAG\"\nstarts = \"x\""), "names folders");
    refused(&rule("check = \"marker\""), "unknown variant");
    refused(&rule("entries = \"files\"\nabout = \"recycle-bin\""), "is a folder");
    refused("[[keep]]\npaths = []\n", "at least one path");
    refused("[[keep]]\npaths = [\"x\"]\ncolour = \"red\"\n", "unknown field");
}
