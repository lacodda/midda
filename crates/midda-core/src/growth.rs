//! Who is eating the disk: the few folders that account for the growth, each
//! counted once, and the pace it is going at.
//!
//! A comparison says what every entry held at two moments ([`crate::compare`]);
//! the view of what was written lately says where recent bytes went
//! ([`crate::fresh::since`]). Both are trees, and a tree of a volume's growth
//! is a question rather than an answer: "what is filling C:" wants a handful
//! of names, each with a number, and the numbers adding up.
//!
//! # Growers
//!
//! A grower is a folder that, apart from the growers named under it, grew by
//! at least a fiftieth of everything that grew ([`SHARE`]). It is the rule
//! network traffic is summarised by — hierarchical heavy hitters, by prefix —
//! and it has the three properties a list of "who ate the space" needs:
//!
//! - **Specific.** A folder is named as deep as its growth is still one
//!   sizeable thing: `app\node_modules`, not `app`, when the dependencies are
//!   what grew; `node_modules`, not each of its twelve hundred packages, when
//!   none of them alone is sizeable.
//! - **Counted once.** A folder named under another is taken out of the
//!   other's share: what each line accounts for, and what is left over, add up
//!   to the growth of the whole.
//! - **Short.** Every line accounts for a fiftieth of the growth, so there are
//!   about fifty at most, whatever the size of the volume.
//!
//! Growth is net: what a folder holds now less what it held. A folder where
//! five gigabytes went and five came is where everything happened, and is not
//! eating the disk; the picture of a comparison shows it, this list does not.
//!
//! Files are never lines of their own. Their growth is their folder's, so a
//! download folder that took four installers reads as one line, four files,
//! rather than as four lines and no folder. The one exception is the root,
//! which is never a line — it is the whole — and whose own files are named
//! apart when they are sizeable: on a system volume those are the page file
//! and the hibernation file, and they belong in the answer.
//!
//! What grew in pieces too small to name — a fiftieth each at most — is said
//! as one number, the rest, so the list never claims to be all of it.
//!
//! The rule cannot tell from the shape of a tree that `target\debug` and
//! `target\release` are one build and `data` and `target` are two things; a
//! sizeable build folder may therefore come out as two or three lines. A folder
//! known to be one thing — a `node_modules` or a `target` the rules found
//! ([`crate::rules::RuleSet::units`]) — can be declared a *unit*: it is named
//! whole, and nothing under it is.
//!
//! # Pace
//!
//! Growth over a span is a pace: so much a day. Against the space free on the
//! volume, a pace says how long until it runs out — the question "before the
//! disk is full" asks. Only from pictures at least [`PACE_SPAN`] apart: a pace
//! read off an hour, one build in it, multiplied by twenty-four, is noise
//! stated as a forecast.

use std::time::Duration;

use crate::tree::{NodeId, ROOT, Tree};

/// A folder is named when, apart from what is named under it, it accounts
/// for at least one part in `SHARE` of everything that grew.
///
/// A fiftieth: few enough lines to read at a glance, fine enough that a
/// folder of a few hundred megabytes in a day's ten gigabytes is named.
pub const SHARE: u64 = 50;

/// How far apart two pictures have to be for their difference to be read as
/// a pace.
pub const PACE_SPAN: Duration = Duration::from_secs(24 * 60 * 60);

/// One line of the answer: a folder, and what it grew by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grower {
    /// The folder, in the tree that was asked about.
    pub id: NodeId,
    /// What the whole folder grew by, the growers inside it included.
    pub growth: i64,
    /// What it grew by apart from the growers named inside it: what this line
    /// accounts for, and what the lines add up by.
    pub own: i64,
    /// How many growers are named inside it.
    pub inside: u32,
    /// The line is the root's own files, not the root: the root itself is the
    /// whole and is never a line.
    pub loose: bool,
}

/// The growers of a tree, and what they leave unnamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Growers {
    /// The most first, by what each accounts for on its own.
    pub growers: Vec<Grower>,
    /// What grew in pieces too small to name, on balance: the growth of the
    /// whole less what the lines account for. Negative when the unnamed rest
    /// shrank.
    pub rest: i64,
    /// What a folder had to account for on its own to be named.
    pub threshold: u64,
}

/// The growers of `tree`, by `value` — what each entry grew by, its subtree
/// included, which can be negative — against `scale`, the growth a fiftieth
/// is taken of.
///
/// `scale` is what every folder's growth stays within: in a comparison, all
/// that grew, which no folder's net growth can exceed. A folder `unit` says
/// is one thing is named whole when it is sizeable, and nothing under it is.
///
/// `value` has to add up: a folder's is what its children's come to. Every
/// tree midda builds holds that — a folder carries the whole of what is under
/// it — and the lines add up to the whole because of it.
#[must_use]
pub fn growers(tree: &Tree, scale: u64, value: impl Fn(NodeId) -> i64, unit: impl Fn(NodeId) -> bool) -> Growers {
    let threshold = (scale / SHARE).max(1);
    let floor = i64::try_from(threshold).unwrap_or(i64::MAX);

    // Root first, each folder before what is in it; walked backwards, every
    // folder is reached after everything under it. Not the arena's order: a
    // tree kept up to date appends what arrives.
    let mut order = Vec::new();
    let mut stack = vec![ROOT];
    while let Some(id) = stack.pop() {
        order.push(id);
        let node = tree.node(id);
        // Nothing under a unit is looked at: it is one thing.
        if id == ROOT || !unit(id) {
            stack.extend(node.children.iter().copied().filter(|&child| tree.node(child).is_directory()));
        }
    }

    // What of each folder's growth the lines at or under it account for, and
    // how many lines those are.
    let mut named = vec![0_i64; tree.nodes().map(|(id, _)| id as usize + 1).max().unwrap_or(1)];
    let mut lines = vec![0_u32; named.len()];
    let mut growers = Vec::new();
    for &id in order.iter().rev() {
        let node = tree.node(id);
        let (below, inside) = if id != ROOT && unit(id) {
            (0, 0)
        } else {
            node.children
                .iter()
                .filter(|&&child| tree.node(child).is_directory())
                .fold((0_i64, 0_u32), |(bytes, count), &child| {
                    (bytes.saturating_add(named[child as usize]), count + lines[child as usize])
                })
        };
        if id == ROOT {
            named[ROOT as usize] = below;
            lines[ROOT as usize] = inside;
            continue;
        }
        let growth = value(id);
        let own = growth.saturating_sub(below);
        if own >= floor {
            growers.push(Grower {
                id,
                growth,
                own,
                inside,
                loose: false,
            });
            named[id as usize] = growth;
            lines[id as usize] = inside + 1;
        } else {
            named[id as usize] = below;
            lines[id as usize] = inside;
        }
    }

    let root = tree.root();
    let mut rest = value(ROOT).saturating_sub(named[ROOT as usize]);
    let loose = root
        .children
        .iter()
        .filter(|&&child| !tree.node(child).is_directory())
        .fold(0_i64, |bytes, &child| bytes.saturating_add(value(child)));
    if loose >= floor {
        growers.push(Grower {
            id: ROOT,
            growth: loose,
            own: loose,
            inside: 0,
            loose: true,
        });
        rest = rest.saturating_sub(loose);
    }

    growers.sort_by(|a, b| b.own.cmp(&a.own).then_with(|| a.id.cmp(&b.id)));
    Growers { growers, rest, threshold }
}

/// How fast something grew, and how long the free space lasts at that rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    /// What it grew by in a day, on average over the span. Negative when it
    /// shrank.
    pub per_day: i64,
    /// How long until `free` runs out at this pace, when it grows at all and
    /// the free space is known.
    pub full_in: Option<Duration>,
}

/// The pace of `growth` over `span`, and how long `free` lasts at it.
///
/// `None` when the span is shorter than [`PACE_SPAN`]: too short to say
/// anything about a day.
#[must_use]
pub fn pace(growth: i64, span: Duration, free: Option<u64>) -> Option<Pace> {
    if span < PACE_SPAN {
        return None;
    }
    let day = PACE_SPAN.as_secs_f64();
    #[allow(
        clippy::cast_precision_loss,
        reason = "a pace is an estimate; a disk's bytes fit an f64's 53 bits to the byte below eight petabytes"
    )]
    let (growth, free) = (growth as f64, free.map(|free| free as f64));
    let per_day = growth * day / span.as_secs_f64();
    // A free space that would last past what a Duration holds lasts forever
    // as far as anyone reading it is concerned: no forecast, not a panic.
    let full_in = free
        .filter(|_| per_day > 0.0)
        .and_then(|free| Duration::try_from_secs_f64(free / per_day * day).ok());
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a day's growth is bounded by the growth it was averaged over, which was an i64"
    )]
    let per_day = per_day.round() as i64;
    Some(Pace { per_day, full_in })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::size::Size;
    use crate::tree::{Kind, Node, listing_order};

    /// A tree from `(path, bytes)` pairs, `None` for a folder, and the growth
    /// of each entry by path: a file grew by what it says, a folder by what is
    /// under it.
    struct Grown {
        tree: Tree,
        growth: HashMap<NodeId, i64>,
    }

    impl Grown {
        fn new(entries: &[(&str, Option<i64>)]) -> Self {
            let mut tree = Tree::new(PathBuf::from("C:/disk"), Some(4096));
            let mut sorted: Vec<_> = entries.to_vec();
            sorted.sort_by(|a, b| {
                let (x, y) = (a.0.split('/').collect::<Vec<_>>(), b.0.split('/').collect::<Vec<_>>());
                x.len().cmp(&y.len()).then_with(|| {
                    x.iter()
                        .zip(&y)
                        .map(|(p, q)| listing_order(p, q))
                        .find(|order| order.is_ne())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            });
            let mut growth = HashMap::new();
            for (path, bytes) in sorted {
                let mut parts: Vec<&str> = path.split('/').collect();
                let name = parts.pop().expect("a name");
                let parent = parts.iter().fold(ROOT, |at, part| tree.child(at, part).expect("the folder comes first"));
                let id = match bytes {
                    Some(bytes) => tree.push(
                        parent,
                        Node {
                            size: Size::flat(bytes.unsigned_abs()),
                            ..Node::new(name.into(), parent, Kind::File)
                        },
                    ),
                    None => tree.push(parent, Node::new(name.into(), parent, Kind::Directory)),
                };
                growth.insert(id, bytes.unwrap_or(0));
            }
            // Folders grow by what is under them, deepest first.
            let ids: Vec<NodeId> = tree.nodes().map(|(id, _)| id).collect();
            for &id in ids.iter().rev() {
                if id != ROOT {
                    let parent = tree.node(id).parent;
                    let here = growth.get(&id).copied().unwrap_or(0);
                    *growth.entry(parent).or_insert(0) += here;
                }
            }
            Self { tree, growth }
        }

        fn value(&self, id: NodeId) -> i64 {
            self.growth.get(&id).copied().unwrap_or(0)
        }

        fn scale(&self) -> u64 {
            self.growth
                .iter()
                .filter(|&(&id, _)| !self.tree.node(id).is_directory())
                .map(|(_, &bytes)| bytes.max(0).unsigned_abs())
                .sum()
        }

        fn growers(&self) -> Growers {
            growers(&self.tree, self.scale(), |id| self.value(id), |_| false)
        }

        fn named(&self, growers: &Growers) -> Vec<String> {
            growers
                .growers
                .iter()
                .map(|grower| {
                    let path = self.tree.path_of(grower.id).to_string_lossy().replace('\\', "/");
                    let path = path.strip_prefix("C:/disk").unwrap_or(&path).trim_start_matches('/').to_owned();
                    if grower.loose { format!("{path}(files)") } else { path }
                })
                .collect()
        }
    }

    /// A package folder of `count` packages, each `bytes` new.
    fn packages(at: &str, count: usize, bytes: i64) -> Vec<(String, Option<i64>)> {
        let mut entries = vec![(at.to_owned(), None)];
        for index in 0..count {
            entries.push((format!("{at}/pkg{index:03}"), None));
            entries.push((format!("{at}/pkg{index:03}/index.js"), Some(bytes)));
        }
        entries
    }

    fn grown(entries: &[(String, Option<i64>)]) -> Grown {
        let borrowed: Vec<(&str, Option<i64>)> = entries.iter().map(|(path, bytes)| (path.as_str(), *bytes)).collect();
        Grown::new(&borrowed)
    }

    #[test]
    fn a_folder_of_many_small_new_things_is_one_line_not_each_of_them() {
        // 200 packages of 1 MB in node_modules, 10 MB of source: no package
        // is a fiftieth, the folder holding them is.
        let mut entries = vec![
            ("app".to_owned(), None),
            ("app/src".to_owned(), None),
            ("app/src/main.rs".to_owned(), Some(10 << 20)),
        ];
        entries.extend(packages("app/node_modules", 200, 1 << 20));
        let grown = grown(&entries);
        let growers = grown.growers();
        assert_eq!(grown.named(&growers), ["app/node_modules", "app/src"]);
        assert_eq!(growers.growers[0].growth, 200 << 20);
        assert_eq!(growers.growers[0].inside, 0);
    }

    #[test]
    fn a_folder_is_named_as_deep_as_its_growth_is_one_sizeable_thing() {
        // All of project's growth is in its build folder: the build folder is
        // named, not the project, and not the root.
        let grown = Grown::new(&[
            ("project", None),
            ("project/target", None),
            ("project/target/a.rlib", Some(300)),
            ("project/target/b.rlib", Some(300)),
            ("project/target/c.rlib", Some(400)),
        ]);
        let growers = grown.growers();
        assert_eq!(grown.named(&growers), ["project/target"]);
        assert_eq!(growers.rest, 0);
    }

    #[test]
    fn every_byte_is_accounted_for_once_by_the_lines_and_the_rest() {
        let mut entries = vec![
            ("Downloads".to_owned(), None),
            ("Downloads/a.iso".to_owned(), Some(4_000)),
            ("Downloads/b.msi".to_owned(), Some(3_000)),
            ("work".to_owned(), None),
            ("work/build".to_owned(), None),
            ("work/build/out.bin".to_owned(), Some(9_000)),
            ("work/notes.md".to_owned(), Some(2_500)),
            ("old".to_owned(), None),
            ("old/archive.zip".to_owned(), Some(-6_000)),
        ];
        // Many crumbs, each far under a fiftieth.
        for index in 0..40 {
            entries.push((format!("crumbs{index:02}"), None));
            entries.push((format!("crumbs{index:02}/log.txt"), Some(20)));
        }
        let grown = grown(&entries);
        let growers = grown.growers();
        let own: i64 = growers.growers.iter().map(|grower| grower.own).sum();
        assert_eq!(own + growers.rest, grown.value(ROOT), "the lines and the rest are the whole");
        assert_eq!(grown.named(&growers), ["work/build", "Downloads", "work"]);
        let work = growers.growers.iter().find(|grower| grower.own == 2_500).expect("work's own files");
        assert_eq!((work.growth, work.inside), (11_500, 1), "work grew 11.5k, 9k of it named under it");
        // The crumbs and the shrunk archive are the rest, on balance.
        assert_eq!(growers.rest, 40 * 20 - 6_000);
    }

    #[test]
    fn a_folder_with_a_grower_inside_is_named_only_for_what_it_grew_besides() {
        // cache grew 6 000 in a subfolder and 10 loose: the subfolder is
        // named, and cache, for the 10 besides, is not - a fiftieth of the
        // 11 010 that grew is 220.
        let grown = Grown::new(&[
            ("cache", None),
            ("cache/blobs", None),
            ("cache/blobs/x.bin", Some(6_000)),
            ("cache/index", Some(10)),
            ("other", None),
            ("other/y.bin", Some(5_000)),
        ]);
        let growers = grown.growers();
        assert_eq!(grown.named(&growers), ["cache/blobs", "other"]);
        assert_eq!(growers.rest, 10);
    }

    #[test]
    fn a_folder_that_churned_is_not_eating_the_disk() {
        // logs rotated: as much went as came. Net nothing, so not a line,
        // however much moved.
        let grown = Grown::new(&[
            ("logs", None),
            ("logs/new.log", Some(5_000)),
            ("logs/old.log", Some(-5_000)),
            ("data", None),
            ("data/db.bin", Some(1_000)),
        ]);
        let growers = grown.growers();
        assert_eq!(grown.named(&growers), ["data"]);
    }

    #[test]
    fn the_roots_own_files_are_named_when_sizeable_and_the_root_never_is() {
        // C:\ itself: the page file grew. Without its own line it would vanish
        // into the rest.
        let grown = Grown::new(&[("pagefile.sys", Some(8_000)), ("Users", None), ("Users/me.dat", Some(2_000))]);
        let growers = grown.growers();
        assert_eq!(grown.named(&growers), ["(files)", "Users"]);
        assert!(growers.growers[0].loose);
        assert_eq!(growers.growers[0].id, ROOT);
        assert_eq!(growers.rest, 0);
    }

    #[test]
    fn the_threshold_is_a_share_of_everything_that_grew() {
        // 50 folders of 100 each: every one is exactly a fiftieth.
        let entries: Vec<(String, Option<i64>)> = (0..50)
            .flat_map(|index| [(format!("f{index:02}"), None), (format!("f{index:02}/a"), Some(100))])
            .collect();
        let grown = grown(&entries);
        let growers = grown.growers();
        assert_eq!(growers.threshold, 100);
        assert_eq!(growers.growers.len(), 50, "a fiftieth each, so all fifty are named");

        // One more folder of 49 is under it, and is the rest.
        let mut entries = entries;
        entries.extend([("tiny".to_owned(), None), ("tiny/a".to_owned(), Some(49))]);
        let grown = self::grown(&entries);
        let growers = grown.growers();
        assert_eq!(growers.threshold, (5_000 + 49) / SHARE);
        assert_eq!(growers.growers.len(), 50);
        assert_eq!(growers.rest, 49);
    }

    #[test]
    fn a_unit_is_named_whole_and_nothing_under_it_is() {
        // target holds debug (600) and release (400): without knowing better
        // both are named; declared a unit, target is one line.
        let grown = Grown::new(&[
            ("target", None),
            ("target/debug", None),
            ("target/debug/a", Some(600)),
            ("target/release", None),
            ("target/release/b", Some(400)),
        ]);
        assert_eq!(grown.named(&grown.growers()), ["target/debug", "target/release"]);
        let target = grown.tree.child(ROOT, "target").expect("target");
        let as_unit = growers(&grown.tree, grown.scale(), |id| grown.value(id), |id| id == target);
        assert_eq!(grown.named(&as_unit), ["target"]);
        assert_eq!(as_unit.growers[0].inside, 0);
    }

    #[test]
    fn nothing_grew_names_nothing() {
        let grown = Grown::new(&[("a", None), ("a/b", Some(0))]);
        let growers = grown.growers();
        assert!(growers.growers.is_empty());
        assert_eq!(growers.rest, 0);
        assert_eq!(growers.threshold, 1);
    }

    #[test]
    fn a_pace_needs_a_day_between_the_pictures() {
        let hour = Duration::from_secs(3_600);
        assert_eq!(
            pace(5_000, hour * 23, Some(1_000_000)),
            None,
            "a build in an hour times twenty-four is not a pace"
        );
        let week = pace(70_000, hour * 24 * 7, Some(100_000)).expect("a week is a pace");
        assert_eq!(week.per_day, 10_000);
        assert_eq!(week.full_in, Some(hour * 24 * 10), "100 000 free at 10 000 a day");
    }

    #[test]
    fn a_folder_that_shrank_or_stood_still_is_not_filling_anything() {
        let day = PACE_SPAN;
        let shrinking = pace(-3_000, day * 3, Some(1_000)).expect("a pace");
        assert_eq!((shrinking.per_day, shrinking.full_in), (-1_000, None));
        assert_eq!(pace(0, day, Some(1_000)).expect("a pace").full_in, None);
        assert_eq!(pace(1_000, day, None).expect("a pace").full_in, None, "free space unknown");
    }
}
