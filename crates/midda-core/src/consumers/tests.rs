use std::path::PathBuf;

use super::story::tell;
use super::*;
use crate::platform::Space;
use crate::tree::{Kind, Node};

/// The top of a volume as the platform writes it: `C:/` here, `/` on Linux,
/// where `C:` would be a folder like any other.
const VOLUME: &str = if cfg!(windows) { "C:/" } else { "/" };

const GB: u64 = 1024 * 1024 * 1024;
const MB: u64 = 1024 * 1024;

/// A tree at `root` from paths: a trailing `/` is a folder, anything else a
/// file of `bytes`.
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
            .fold(ROOT, |at, part| tree.child(at, part).unwrap_or_else(|| panic!("{part} comes before {path}")));
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

/// The id of the entry at `path` below the root.
fn at(tree: &Tree, path: &str) -> NodeId {
    path.split('/')
        .filter(|part| !part.is_empty())
        .fold(ROOT, |at, part| tree.child(at, part).unwrap_or_else(|| panic!("{path} is in the tree")))
}

fn classified(tree: &Tree) -> Classified {
    let rules = RuleSet::shipped();
    classify(rules, tree, &rules.survey(tree))
}

/// A system volume with a little of everything, in the shipped rules' terms.
fn volume() -> Tree {
    tree_of(
        VOLUME,
        &[
            ("Windows/", 0),
            ("Windows/System32/", 0),
            ("Windows/System32/kernel32.dll", 6 * GB),
            ("Windows/WinSxS/", 0),
            ("Windows/WinSxS/component.cab", 9 * GB),
            ("pagefile.sys", 16 * GB),
            ("hiberfil.sys", 6 * GB),
            ("Program Files/", 0),
            ("Program Files/Office/", 0),
            ("Program Files/Office/winword.exe", 5 * GB),
            ("Program Files/Office/splash.png", 100 * MB),
            ("Program Files (x86)/", 0),
            ("Program Files (x86)/Steam/", 0),
            ("Program Files (x86)/Steam/steam.exe", 300 * MB),
            ("Program Files (x86)/Steam/steamapps/", 0),
            ("Program Files (x86)/Steam/steamapps/common/", 0),
            ("Program Files (x86)/Steam/steamapps/common/Game/", 0),
            ("Program Files (x86)/Steam/steamapps/common/Game/data.pak", 60 * GB),
            ("Program Files (x86)/Steam/steamapps/common/Game/intro.mp4", 2 * GB),
            ("Users/", 0),
            ("Users/me/", 0),
            ("Users/me/AppData/", 0),
            ("Users/me/AppData/Local/", 0),
            ("Users/me/AppData/Local/Google/", 0),
            ("Users/me/AppData/Local/Google/Chrome/", 0),
            ("Users/me/AppData/Local/Google/Chrome/User Data/", 0),
            ("Users/me/AppData/Local/Google/Chrome/User Data/Default/", 0),
            ("Users/me/AppData/Local/Google/Chrome/User Data/Default/Cache", 3 * GB),
            ("Users/me/AppData/Local/Microsoft/", 0),
            ("Users/me/AppData/Local/Microsoft/Windows/", 0),
            ("Users/me/AppData/Local/Microsoft/Windows/thumbcache.db", GB),
            ("Users/me/AppData/Local/Docker/", 0),
            ("Users/me/AppData/Local/Docker/settings.json", MB),
            ("Users/me/AppData/Local/Docker/wsl/", 0),
            ("Users/me/AppData/Local/Docker/wsl/disk/", 0),
            ("Users/me/AppData/Local/Docker/wsl/disk/docker_data.vhdx", 30 * GB),
            ("Users/me/AppData/Local/Temp/", 0),
            ("Users/me/AppData/Local/Temp/x.tmp", 2 * GB),
            ("Users/me/Documents/", 0),
            ("Users/me/Documents/report.docx", 200 * MB),
            ("Users/me/Documents/clip.mp4", 4 * GB),
            ("Users/me/Documents/notes", 10 * MB),
            ("Users/me/Documents/My Games/", 0),
            ("Users/me/Documents/My Games/save.dat", 500 * MB),
            ("Users/me/Documents/app/", 0),
            ("Users/me/Documents/app/package.json", 1),
            ("Users/me/Documents/app/node_modules/", 0),
            ("Users/me/Documents/app/node_modules/movie.mp4", GB),
            ("Projects/", 0),
            ("Projects/rust/", 0),
            ("Projects/rust/Cargo.toml", 1),
            ("Projects/rust/main.rs", MB),
            ("Projects/rust/target/", 0),
            ("Projects/rust/target/app.exe", 40 * GB),
            ("Projects/rust/.git/", 0),
            ("Projects/rust/.git/HEAD", 1),
            ("Projects/rust/.git/pack", 3 * GB),
            ("Data/", 0),
            ("Data/blob.bin", 7 * GB),
        ],
    )
}

#[test]
fn every_byte_goes_to_one_consumer_and_they_add_up_to_the_whole() {
    let tree = volume();
    let classified = classified(&tree);
    let mut sum = Size::zero();
    for consumer in Consumer::ALL {
        sum.add(classified.total(consumer));
    }
    assert_eq!(sum, tree.root().size);
    let summary = classified.summary(&tree, SizeBasis::Allocated);
    let shares: u64 = summary.shares.iter().map(|share| share.size.allocated).sum();
    assert_eq!(shares, tree.root().size.allocated, "and the summary says the same");
}

#[test]
fn the_deepest_place_decides() {
    let tree = volume();
    let classified = classified(&tree);
    let of = |path| classified.of(at(&tree, path));
    assert_eq!(of("Program Files/Office/winword.exe"), Some(Consumer::Programs));
    assert_eq!(
        of("Program Files (x86)/Steam/steamapps/common/Game/data.pak"),
        Some(Consumer::Games),
        "the Steam library inside Program Files is games"
    );
    assert_eq!(of("Users/me/AppData/Local/Google/Chrome/User Data/Default/Cache"), Some(Consumer::Browsers));
    assert_eq!(
        of("Users/me/AppData/Local/Temp/x.tmp"),
        Some(Consumer::System),
        "Windows' temp inside a profile"
    );
    assert_eq!(
        of("Users/me/Documents/My Games/save.dat"),
        Some(Consumer::Games),
        "a firm place inside a soft one"
    );
    assert_eq!(of("Users/me/AppData/Local/Docker/settings.json"), Some(Consumer::Development));
    assert_eq!(
        of("Users/me/AppData/Local/Docker/wsl/disk/docker_data.vhdx"),
        Some(Consumer::VirtualDisks),
        "Docker's disk inside Docker's folder"
    );
    assert_eq!(of("Projects/rust/.git/pack"), Some(Consumer::Development), "a repository's history");
}

#[test]
fn a_place_at_the_top_of_a_volume_is_not_every_folder_of_its_name() {
    let tree = volume();
    let classified = classified(&tree);
    assert_eq!(classified.of(at(&tree, "Windows/WinSxS/component.cab")), Some(Consumer::System));
    assert_eq!(
        classified.of(at(&tree, "Users/me/AppData/Local/Microsoft/Windows/thumbcache.db")),
        Some(Consumer::Programs),
        "a profile's folder called Windows is the profile's"
    );
}

#[test]
fn a_folder_a_rule_claims_is_its_groups_whatever_place_it_is_in() {
    let tree = volume();
    let classified = classified(&tree);
    // A node_modules in Documents, holding a video: the rule says what the
    // folder is, and nothing in it is looked at.
    let modules = at(&tree, "Users/me/Documents/app/node_modules");
    assert_eq!(classified.of(modules), Some(Consumer::Development));
    assert_eq!(
        classified.of(at(&tree, "Users/me/Documents/app/node_modules/movie.mp4")),
        None,
        "not looked inside"
    );
    assert!(classified.units().contains(&modules));
    assert_eq!(classified.of(at(&tree, "Projects/rust/target")), Some(Consumer::Development));
    assert_eq!(classified.frees(Consumer::Development).allocated, 41 * GB, "what the findings free");
}

#[test]
fn a_files_own_name_speaks_before_a_soft_place_and_after_a_firm_one() {
    let tree = volume();
    let classified = classified(&tree);
    let of = |path| classified.of(at(&tree, path));
    assert_eq!(of("Users/me/Documents/clip.mp4"), Some(Consumer::Media), "a video in Documents");
    assert_eq!(of("Users/me/Documents/report.docx"), Some(Consumer::Documents));
    assert_eq!(
        of("Users/me/Documents/notes"),
        Some(Consumer::Documents),
        "a name that says nothing, in Documents"
    );
    assert_eq!(
        of("Program Files (x86)/Steam/steamapps/common/Game/intro.mp4"),
        Some(Consumer::Games),
        "a video inside a game is the game's"
    );
    assert_eq!(of("Program Files/Office/splash.png"), Some(Consumer::Programs));
    assert_eq!(of("Projects/rust/main.rs"), Some(Consumer::Documents), "code where no place is");
    assert_eq!(of("Data/blob.bin"), Some(Consumer::Unrecognised), "and nothing guessed where nothing says");
}

#[test]
fn a_scan_of_a_place_is_that_place_throughout() {
    let tree = tree_of(&format!("{VOLUME}Windows"), &[("System32/", 0), ("System32/notes.txt", 100)]);
    let classified = classified(&tree);
    assert_eq!(classified.of(at(&tree, "System32/notes.txt")), Some(Consumer::System));
}

#[test]
fn the_lines_of_a_consumer_are_the_places_that_hold_it() {
    let tree = volume();
    let classified = classified(&tree);
    let summary = classified.summary(&tree, SizeBasis::Allocated);
    let consumers: Vec<Consumer> = summary.shares.iter().map(|share| share.consumer).collect();
    assert_eq!(consumers[..2], [Consumer::Games, Consumer::Development], "the most first");

    let system = summary
        .shares
        .iter()
        .find(|share| share.consumer == Consumer::System)
        .expect("Windows is there");
    let loose = system.lines.iter().find(|line| line.loose).expect("the root's own files are a line");
    assert_eq!(loose.whole, 22 * GB, "the page and hibernation files");
    assert!(system.lines.iter().any(|line| line.id == at(&tree, "Windows/WinSxS")));

    let development = summary
        .shares
        .iter()
        .find(|share| share.consumer == Consumer::Development)
        .expect("development");
    let target = at(&tree, "Projects/rust/target");
    assert_eq!(development.lines[0].id, target, "a claimed folder is a line whole");
}

#[test]
fn a_place_is_said_once_by_the_line_that_is_it() {
    // Downloads is a line for its own file; the folder inside it holds
    // nearly all of it, and is a line too - by its own path, not a second
    // "Downloads".
    let tree = tree_of(
        VOLUME,
        &[
            ("Users/", 0),
            ("Users/me/", 0),
            ("Users/me/Downloads/", 0),
            ("Users/me/Downloads/manual.pdf", GB),
            ("Users/me/Downloads/books/", 0),
            ("Users/me/Downloads/books/a.epub", 9 * GB),
        ],
    );
    let classified = classified(&tree);
    let summary = classified.summary(&tree, SizeBasis::Allocated);
    let lines = &summary.shares[0].lines;
    let downloads = at(&tree, "Users/me/Downloads");
    let books = at(&tree, "Users/me/Downloads/books");
    let named = |id| lines.iter().find(|line| line.id == id).expect("a line").named;
    assert_eq!(named(downloads).map(|named| named.id), Some(downloads));
    assert_eq!(named(books), None);
}

#[test]
fn the_roots_own_files_are_called_by_the_places_that_hold_them() {
    let tree = tree_of(
        VOLUME,
        &[("pagefile.sys", 16 * GB), ("DumpStack.log", 12 * 1024), ("Windows/", 0), ("Windows/x.dll", GB)],
    );
    let classified = classified(&tree);
    let summary = classified.summary(&tree, SizeBasis::Allocated);
    let loose = summary.shares[0].lines.iter().find(|line| line.loose).expect("the root's own files");
    assert_eq!(
        classified.name_of(RuleSet::shipped(), &tree, Consumer::System, loose).as_deref(),
        Some("the page file"),
        "not the crash log beside it"
    );
}

#[test]
fn two_places_of_one_name_are_said_by_their_paths() {
    let rules = RuleSet::parse("[[place]]\nid = \"libraries\"\nname = \"a library\"\nconsumer = \"games\"\npaths = [\"/A\", \"/B\"]\n").expect("reads");
    let tree = tree_of(VOLUME, &[("A/", 0), ("A/x.pak", 6 * GB), ("B/", 0), ("B/y.pak", 4 * GB)]);
    let classified = classify(&rules, &tree, &rules.survey(&tree));
    let text = tell(&rules, &tree, &classified, None).text;
    let path = |name: &str| tree.path_of(at(&tree, name)).display().to_string();
    assert!(text.contains(&format!(": {} 6.0 GB, {} 4.0 GB.", path("A"), path("B"))), "{text}");
}

#[test]
fn the_story_says_how_full_and_why() {
    let tree = volume();
    let rules = RuleSet::shipped();
    let classified = classified(&tree);
    // The scan counts 196 GB; the volume uses 200 of 230.
    let space = Space {
        total: 230 * GB,
        free: 30 * GB,
    };
    let story = tell(rules, &tree, &classified, Some(space));
    let text = &story.text;
    assert!(
        text.starts_with(&format!("{} is 87% full: 200 GB used of 230 GB, 30 GB free. Most", tree.root_path().display())),
        "{text}"
    );
    assert!(text.contains("Most of it is five things."), "{text}");
    assert!(
        text.contains("Games take 63 GB (32%): the Steam library 62 GB."),
        "the library is all one game: {text}"
    );
    assert!(text.contains("A tool makes 41 GB of it again."), "{text}");
    assert!(
        text.contains("Windows takes 39 GB (20%): the page file and the hibernation file 22 GB, the component store 9.0 GB."),
        "{text}"
    );
    assert!(
        text.contains("Virtual disks take 30 GB (15%): Docker's disk 30 GB. midda cannot see inside it."),
        "{text}"
    );
    assert!(!text.contains('`'), "paths are plain for a person");
    assert!(!text.contains("Browsers"), "a sixth consumer is not a reason: {text}");

    let brief = &story.brief;
    assert!(brief.starts_with("**What fills"), "{brief}");
    assert!(brief.contains("- **Browsers** 3.0 GB"), "every consumer is in the brief: {brief}");
    assert!(brief.contains("`"), "paths are code for a chat: {brief}");
}

/// The virtual disks of `tree`, told as if compacting each gave back `back`.
fn looked(tree: &Tree, classified: &Classified, back: u64) -> Vec<crate::vdisk::Looked> {
    crate::vdisk::find(RuleSet::shipped(), tree, classified)
        .into_iter()
        .map(|disk| {
            let look = crate::vdisk::Look {
                format: Some(crate::vdisk::Format::Vhdx),
                inside: crate::vdisk::Inside::Unread(String::new()),
                distribution: None,
            };
            let mut told = crate::vdisk::tell(&disk, &look);
            told.gives_back = Some(back);
            told.worth = true;
            crate::vdisk::Looked { disk, look, told }
        })
        .collect()
}

#[test]
fn the_story_says_what_compacting_the_virtual_disks_gives_back() {
    let tree = volume();
    let rules = RuleSet::shipped();
    let classified = classified(&tree);
    let summary = classified.summary(&tree, SizeBasis::Allocated);
    let story = super::story::tell_from(rules, &tree, &classified, &summary, None, &looked(&tree, &classified, 20 * GB));
    assert!(
        story.text.contains("Docker's disk 30 GB. Compacting it gives back about 20 GB."),
        "{}",
        story.text
    );
    assert!(
        story.brief.contains("- **Virtual disks** 30 GB (15%); compacting gives back about 20 GB"),
        "{}",
        story.brief
    );

    let little = super::story::tell_from(rules, &tree, &classified, &summary, None, &looked(&tree, &classified, 100 * MB));
    assert!(
        little.text.contains("Docker's disk 30 GB. Compacting it would give back next to nothing."),
        "{}",
        little.text
    );
    assert!(!little.brief.contains("compacting"), "{}", little.brief);
}

#[test]
fn the_story_of_a_folder_says_what_it_holds() {
    let tree = tree_of(
        &format!("{VOLUME}Projects"),
        &[("rust/", 0), ("rust/Cargo.toml", 1), ("rust/target/", 0), ("rust/target/x", 3 * GB)],
    );
    let rules = RuleSet::shipped();
    let story = tell(rules, &tree, &classified(&tree), Some(Space { total: GB, free: GB }));
    assert!(
        story.text.starts_with(&format!("{} holds 3.0 GB.", tree.root_path().display())),
        "{}",
        story.text
    );
    assert!(story.text.contains("Most of it is one thing."), "{}", story.text);
    assert!(!story.text.contains("full"), "a folder is not a volume: {}", story.text);
}

#[test]
fn the_story_says_when_the_scan_counts_less_than_the_volume_uses() {
    let tree = volume();
    let rules = RuleSet::shipped();
    let story = tell(
        rules,
        &tree,
        &classified(&tree),
        Some(Space {
            total: 500 * GB,
            free: 40 * GB,
        }),
    );
    assert!(
        story.text.contains("The scan counts 196 GB of it; the rest is in what it could not open."),
        "{}",
        story.text
    );
}

#[test]
fn every_consumer_is_written_by_its_key() {
    for consumer in Consumer::ALL.into_iter().filter(|&consumer| consumer != Consumer::Unrecognised) {
        let text = format!("[[place]]\nid = \"x\"\nname = \"x\"\nconsumer = \"{}\"\npaths = [\"x\"]\n", consumer.key());
        let set = RuleSet::parse(&text).unwrap_or_else(|why| panic!("{}: {why}", consumer.key()));
        assert_eq!(set.places()[0].consumer, consumer);
    }
}

#[test]
fn the_shipped_places_name_something_of_every_consumer() {
    let rules = RuleSet::shipped();
    assert!(rules.places().len() > 40, "the shipped places are there");
    for consumer in Consumer::ALL.into_iter().filter(|&consumer| consumer != Consumer::Unrecognised) {
        let named = rules.places().iter().any(|place| place.consumer == consumer) || rules.file_kinds().iter().any(|kind| kind.consumer == consumer);
        assert!(named, "nothing says what is {}", consumer.key());
    }
    for place in rules.places() {
        assert!(!place.name.ends_with('.'), "{}: a name, not a sentence", place.id);
    }
}
