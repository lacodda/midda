//! How old the layers of a disk are, and which of them have turned to stone.
//!
//! The name of the product is the question this answers: a midden is dug in
//! layers, and under this week's files lie folders nothing has touched in
//! years - a `target/` of a project long finished, the photos of a phone two
//! phones ago, an installer of a program since replaced. Their size is not
//! news; their age is.
//!
//! # One measure of age
//!
//! A layer is as old as the newest thing in it: [`untouched`] is how long ago
//! anything beneath a node was written, read from the subtree's write time
//! the roll-up keeps ([`crate::tree::Node::subtree_modified`]). Not a folder's
//! own write time, which moves when a tool merely touches it, and not when a
//! file was last opened: that is kept beside the age where the volume keeps it
//! ([`crate::tree::Node::accessed`]), and is not part of it (decided
//! 2026-09-11 - an access time is not kept on most volumes, and where it is,
//! every antivirus scan and indexer moves it). The rules' `older` asks the same
//! function ([`crate::rules`]), so "older than a week" in a rules file and
//! "untouched for a year" here are one measure, not two.
//!
//! A time the volume would not give is not old: nothing here calls a layer a
//! fossil on a guess.
//!
//! # Fossils
//!
//! A node untouched for at least [`FOSSIL`] is a fossil, and what the window
//! lists are the largest fossils that are not inside a larger one
//! ([`fossils`]): a folder is named whole when nothing in it has moved, its
//! parts only when it is still alive around them. Each byte is in one fossil
//! at most, so the list adds up. And it adds up to the same as the strata:
//! a folder's write time is never older than its newest file's, so the
//! files untouched for a year are exactly the ones inside the fossils
//! ([`strata`]). See `docs/adr/0017-the-age-of-the-layers.md`.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::order::{self, Sort, SortKey, Span, Value};
use crate::size::{Size, SizeBasis};
use crate::tree::{Node, NodeId, ROOT, Tree};
use crate::treemap::{self, Category, Layout, Tile};

/// A day, as the ages here count them.
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long nothing in a layer may have been written before it is a fossil:
/// a year of 365 days, as the rules count a year.
///
/// One year rather than a choice: the question is "what has nothing touched
/// in a year", a span long enough that every season's work has passed through
/// a folder that is still in use, and a setting would make the reader decide
/// what old means before seeing anything. The strata say what is older still.
pub const FOSSIL: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// How long ago anything in `node` was written, as of `now`: `None` when the
/// volume gave no time, or gave one after `now` - a clock set wrong is not an
/// age.
#[must_use]
pub fn untouched(node: &Node, now: SystemTime) -> Option<Duration> {
    now.duration_since(node.subtree_modified?).ok()
}

/// Whether nothing in `node` has been written for [`FOSSIL`].
#[must_use]
pub fn is_fossil(node: &Node, now: SystemTime) -> bool {
    untouched(node, now).is_some_and(|age| age >= FOSSIL)
}

/// An age, as a band of the strata: how long nothing in a layer has been
/// written.
///
/// The bands get longer as they go down, as layers in the ground do: a month
/// is worth telling from half a year, and five years from two, but nobody
/// asks whether a folder is four years or four years and a month old.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stratum {
    /// Written in the last 30 days.
    Month,
    /// In the last half year (182 days), not the last month.
    HalfYear,
    /// In the last year, not the last half year.
    Year,
    /// One to two years ago: the first band of fossils.
    TwoYears,
    /// Two to five years ago.
    FiveYears,
    /// Over five years ago.
    Older,
    /// The volume gave no time.
    Undated,
}

impl Stratum {
    /// Every band, the youngest first and the undated last: the order the
    /// window draws them in.
    pub const ALL: [Self; 7] = [
        Self::Month,
        Self::HalfYear,
        Self::Year,
        Self::TwoYears,
        Self::FiveYears,
        Self::Older,
        Self::Undated,
    ];

    /// The band of an age.
    #[must_use]
    pub fn of(age: Option<Duration>) -> Self {
        let Some(age) = age else {
            return Self::Undated;
        };
        if age < 30 * DAY {
            Self::Month
        } else if age < 182 * DAY {
            Self::HalfYear
        } else if age < FOSSIL {
            Self::Year
        } else if age < 2 * FOSSIL {
            Self::TwoYears
        } else if age < 5 * FOSSIL {
            Self::FiveYears
        } else {
            Self::Older
        }
    }

    /// The band `node` is in, as of `now`.
    #[must_use]
    pub fn of_node(node: &Node, now: SystemTime) -> Self {
        Self::of(untouched(node, now))
    }

    /// Whether a layer in this band is a fossil.
    #[must_use]
    pub const fn is_fossil(self) -> bool {
        matches!(self, Self::TwoYears | Self::FiveYears | Self::Older)
    }
}

/// What one band of the strata holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub stratum: Stratum,
    /// What its files take, both ways.
    pub size: Size,
    /// How many files.
    pub files: u64,
}

/// The bytes under a folder by how long ago each file was written: every band,
/// the youngest first, empty ones included so the window draws one bar of
/// fixed order.
///
/// By file, each file once - a file's age is its own write time, which is
/// also the age of the smallest layer holding it. A second name for bytes
/// counted elsewhere holds nothing here, as it holds nothing in the tree.
#[must_use]
pub fn strata(tree: &Tree, under: NodeId, now: SystemTime) -> Vec<Layer> {
    let mut layers: Vec<Layer> = Stratum::ALL
        .iter()
        .map(|&stratum| Layer {
            stratum,
            size: Size::zero(),
            files: 0,
        })
        .collect();
    let mut stack = vec![under];
    while let Some(at) = stack.pop() {
        let node = tree.node(at);
        if node.is_directory() {
            stack.extend_from_slice(&node.children);
            continue;
        }
        let band = Stratum::of_node(node, now);
        let layer = &mut layers[Stratum::ALL.iter().position(|&stratum| stratum == band).unwrap_or(Stratum::ALL.len() - 1)];
        layer.size.add(node.size);
        layer.files += 1;
    }
    layers
}

/// One fossil: a node nothing in which has been written for [`FOSSIL`], inside
/// no larger one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fossil {
    pub id: NodeId,
    pub size: Size,
    pub entries: u64,
    /// When anything in it was last written.
    pub written: Option<SystemTime>,
    /// When a file in it was last opened, where the volume keeps that.
    pub opened: Option<SystemTime>,
}

impl Fossil {
    fn of(tree: &Tree, id: NodeId) -> Self {
        let node = tree.node(id);
        Self {
            id,
            size: node.size,
            entries: node.entries,
            written: node.subtree_modified,
            opened: node.accessed,
        }
    }
}

/// The fossils under a folder, the largest on `basis` first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fossils {
    /// Up to the number asked for.
    pub listed: Vec<Fossil>,
    /// How many fossils there are, listed or not, and what they take.
    pub count: u64,
    pub size: Size,
    /// The fossils past the ones listed: how many, and what they take.
    pub more: u64,
    pub more_size: Size,
}

/// The largest fossils strictly beneath `under`, each inside no larger one, the
/// largest on `basis` first and `limit` of them listed.
///
/// A folder is named whole when nothing in it has been written for a year,
/// and not looked inside: its parts are older still, and the reader's
/// question is about the folder. A folder still in use is looked inside, for
/// the parts of it that are not. Empty files and folders are passed over -
/// a fossil that takes nothing gives the reader nothing to do.
#[must_use]
pub fn fossils(tree: &Tree, under: NodeId, now: SystemTime, basis: SizeBasis, limit: usize) -> Fossils {
    let mut found: Vec<Fossil> = Vec::new();
    let mut stack: Vec<NodeId> = tree.node(under).children.clone();
    while let Some(at) = stack.pop() {
        let node = tree.node(at);
        if is_fossil(node, now) {
            if basis.of(node.size) > 0 {
                found.push(Fossil::of(tree, at));
            }
        } else if node.is_directory() {
            stack.extend_from_slice(&node.children);
        }
    }
    // Largest first, ties by id: a fresh order every time the same tree is
    // asked. The order a scan would list them in - what every other list
    // breaks ties by - walks both paths per comparison, and on a volume of
    // a million entries with thousands of fossils of one cluster each that
    // was most of the time (measured: 266 ms of fossils on 1.26 million
    // entries); it is kept for the rows shown.
    found.sort_unstable_by(|a, b| basis.of(b.size).cmp(&basis.of(a.size)).then(a.id.cmp(&b.id)));
    let shown = limit.min(found.len());
    found[..shown].sort_by(|a, b| basis.of(b.size).cmp(&basis.of(a.size)).then_with(|| tree.scan_order(a.id, b.id)));

    let mut fossils = Fossils {
        listed: Vec::new(),
        count: found.len() as u64,
        size: Size::zero(),
        more: 0,
        more_size: Size::zero(),
    };
    for (at, fossil) in found.into_iter().enumerate() {
        fossils.size.add(fossil.size);
        if at < limit {
            fossils.listed.push(fossil);
        } else {
            fossils.more += 1;
            fossils.more_size.add(fossil.size);
        }
    }
    fossils
}

/// What of each entry nothing has written to for [`FOSSIL`]: the folder seen
/// by its old bytes alone.
///
/// The view "untouched for a year" is the folder as it is with every file
/// younger than a year taken out of the sums: a folder holds what of it is
/// old, and a folder holding nothing old is not listed. Worked out in one pass
/// over the tree, as of a moment, and asked of as often as the window draws.
#[derive(Debug, Clone)]
pub struct Untouched {
    /// By node id: the bytes beneath it in files untouched for a year.
    size: Vec<Size>,
    /// The moment the ages are as of.
    now: SystemTime,
}

impl Untouched {
    /// What of every entry of `tree` is untouched as of `now`.
    #[must_use]
    pub fn of(tree: &Tree, now: SystemTime) -> Self {
        let ids: Vec<NodeId> = tree.nodes().map(|(id, _)| id).collect();
        let slots = ids.iter().copied().max().map_or(1, |max| max as usize + 1);
        let mut size = vec![Size::zero(); slots];
        // A parent precedes its children in the arena, however the tree got
        // here: backwards, each node is whole before its parent is told.
        for &id in ids.iter().rev() {
            let node = tree.node(id);
            if !node.is_directory() && is_fossil(node, now) {
                size[id as usize] = node.size;
            }
            if id != ROOT {
                let own = size[id as usize];
                size[node.parent as usize].add(own);
            }
        }
        Self { size, now }
    }

    /// The bytes beneath `id` untouched for a year.
    #[must_use]
    pub fn size(&self, id: NodeId) -> Size {
        self.size.get(id as usize).copied().unwrap_or_else(Size::zero)
    }

    /// The moment the ages are as of.
    #[must_use]
    pub const fn now(&self) -> SystemTime {
        self.now
    }

    /// One page of the children of `id` that hold anything untouched on
    /// `basis`, ordered by `sort` - a size by what of it is untouched.
    #[must_use]
    pub fn children_page(&self, tree: &Tree, id: NodeId, sort: Sort, basis: SizeBasis, span: Span) -> (Vec<NodeId>, usize) {
        let mut ordered = order::ordered(tree, id, sort.direction, |child| match sort.key {
            SortKey::Allocated => Value::Number(self.size(child).allocated),
            SortKey::Logical => Value::Number(self.size(child).logical),
            key => order::value_of(tree.node(child), key),
        });
        ordered.retain(|&child| basis.of(self.size(child)) > 0);
        order::page(ordered, span)
    }

    /// The picture of the children of `id` by what of each is untouched,
    /// each tile as old as its layer.
    #[must_use]
    pub fn tiles(&self, tree: &Tree, id: NodeId, layout: Layout) -> Vec<Tile> {
        treemap::weighted(
            tree,
            id,
            layout,
            |child| layout.basis.of(self.size(child)),
            // The picture is coloured by age: the strata are the colours.
            |_| Category::Other,
            |child| Some(Stratum::of_node(tree.node(child), self.now)),
            |_| None,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tree::Kind;

    const NOW: Duration = Duration::from_secs(2_000_000_000);

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + NOW
    }

    /// A moment `days` before [`now`].
    fn ago(days: u64) -> Option<SystemTime> {
        Some(now() - DAY * u32::try_from(days).expect("days fit"))
    }

    fn file(name: &str, parent: NodeId, bytes: u64, written: Option<SystemTime>) -> Node {
        Node {
            size: Size::flat(bytes),
            modified: written,
            subtree_modified: written,
            ..Node::new(name.into(), parent, Kind::File)
        }
    }

    fn folder(name: &str, parent: NodeId, written: Option<SystemTime>) -> Node {
        Node {
            modified: written,
            subtree_modified: written,
            ..Node::new(name.into(), parent, Kind::Directory)
        }
    }

    /// root /, each folder's children pushed in the listing order, as a
    /// scanner pushes them:
    ///   note.txt 5 (400 days)
    ///   photos-2019/ 2000 (six and a half years)
    ///   projects/ (written yesterday)
    ///     live-app/ (a file today, its build folder two years old)
    ///       main.rs 10 (today), target/ 500 (800 days)
    ///     old-app/ (three years: everything in it)
    ///       src.rs 100 (1200 days), target/ 900 (1100 days)
    ///   undated.bin 70 (no time)
    fn sample() -> Tree {
        let mut tree = Tree::new(PathBuf::from("C:/disk"), Some(4096));
        tree.stamp_root(ago(1), None);
        tree.push(ROOT, file("note.txt", ROOT, 5, ago(400)));
        let photos = tree.push(ROOT, folder("photos-2019", ROOT, ago(2400)));
        tree.push(photos, file("beach.jpg", photos, 2000, ago(2400)));
        let projects = tree.push(ROOT, folder("projects", ROOT, ago(1)));
        let live = tree.push(projects, folder("live-app", projects, ago(800)));
        tree.push(live, file("main.rs", live, 10, ago(0)));
        let build = tree.push(live, folder("target", live, ago(800)));
        tree.push(build, file("app.exe", build, 500, ago(800)));
        let old = tree.push(projects, folder("old-app", projects, ago(1100)));
        tree.push(old, file("src.rs", old, 100, ago(1200)));
        let target = tree.push(old, folder("target", old, ago(1100)));
        tree.push(target, file("app.exe", target, 900, ago(1100)));
        tree.push(ROOT, file("undated.bin", ROOT, 70, None));
        tree.roll_up();
        tree
    }

    fn named(tree: &Tree, fossils: &Fossils) -> Vec<String> {
        fossils
            .listed
            .iter()
            .map(|fossil| tree.path_of(fossil.id).to_string_lossy().replace('\\', "/"))
            .collect()
    }

    #[test]
    fn a_layer_is_as_old_as_the_newest_thing_in_it() {
        let tree = sample();
        let live = tree.child(tree.child(ROOT, "projects").expect("projects"), "live-app").expect("live-app");
        // Its own write time is 800 days ago; a file in it was written today.
        assert_eq!(untouched(tree.node(live), now()), Some(Duration::ZERO));
        assert!(!is_fossil(tree.node(live), now()));
    }

    #[test]
    fn a_time_the_volume_did_not_give_or_one_from_the_future_is_no_age() {
        let tree = sample();
        let undated = tree.child(ROOT, "undated.bin").expect("undated");
        assert_eq!(untouched(tree.node(undated), now()), None);
        assert!(!is_fossil(tree.node(undated), now()), "an unknown time is not old");

        let future = Node {
            subtree_modified: Some(now() + DAY),
            ..Node::new("ahead".into(), ROOT, Kind::File)
        };
        assert_eq!(untouched(&future, now()), None, "a clock set wrong is not an age");
    }

    #[test]
    fn a_fossil_is_named_whole_and_a_living_folder_by_its_dead_parts() {
        let tree = sample();
        let fossils = fossils(&tree, ROOT, now(), SizeBasis::Logical, 10);
        assert_eq!(
            named(&tree, &fossils),
            [
                "C:/disk/photos-2019",
                "C:/disk/projects/old-app",
                "C:/disk/projects/live-app/target",
                "C:/disk/note.txt"
            ],
            "the largest first; old-app whole, not its target; live-app only by its build folder"
        );
        assert_eq!(fossils.count, 4);
        assert_eq!(fossils.size, Size::flat(2000 + 1000 + 500 + 5));
    }

    #[test]
    fn a_year_to_the_day_is_a_fossil_and_a_day_less_is_not() {
        let mut tree = Tree::new(PathBuf::from("C:/edge"), None);
        tree.push(ROOT, file("year.bin", ROOT, 1, ago(365)));
        tree.push(ROOT, file("almost.bin", ROOT, 1, ago(364)));
        tree.roll_up();
        let fossils = fossils(&tree, ROOT, now(), SizeBasis::Logical, 10);
        assert_eq!(named(&tree, &fossils), ["C:/edge/year.bin"]);
    }

    #[test]
    fn what_is_not_listed_is_counted() {
        let tree = sample();
        let fossils = fossils(&tree, ROOT, now(), SizeBasis::Logical, 1);
        assert_eq!(fossils.listed.len(), 1);
        assert_eq!(fossils.more, 3);
        assert_eq!(fossils.more_size, Size::flat(1000 + 500 + 5));
    }

    #[test]
    fn a_fossil_says_when_it_was_written_and_when_a_file_in_it_was_opened() {
        let mut tree = Tree::new(PathBuf::from("C:/opened"), None);
        let old = tree.push(ROOT, folder("old", ROOT, ago(900)));
        tree.push(
            old,
            Node {
                accessed: ago(20),
                ..file("read.pdf", old, 10, ago(900))
            },
        );
        tree.push(old, file("never.pdf", old, 10, ago(1000)));
        tree.roll_up();
        let fossils = fossils(&tree, ROOT, now(), SizeBasis::Logical, 10);
        let fossil = fossils.listed[0];
        assert_eq!(fossil.written, ago(900));
        assert_eq!(fossil.opened, ago(20), "opened last month, untouched for years: both are said");
    }

    #[test]
    fn the_strata_add_up_to_the_folder_and_their_old_bands_to_its_fossils() {
        let tree = sample();
        let layers = strata(&tree, ROOT, now());
        assert_eq!(layers.iter().map(|layer| layer.stratum).collect::<Vec<_>>(), Stratum::ALL);
        let total = layers.iter().fold(Size::zero(), |mut total, layer| {
            total.add(layer.size);
            total
        });
        assert_eq!(total, tree.root().size, "every byte is in one band");

        let old = layers.iter().filter(|layer| layer.stratum.is_fossil()).fold(Size::zero(), |mut old, layer| {
            old.add(layer.size);
            old
        });
        assert_eq!(
            old,
            fossils(&tree, ROOT, now(), SizeBasis::Logical, 0).size,
            "the old bands and the fossils are one count"
        );

        let band = |stratum: Stratum| layers.iter().find(|layer| layer.stratum == stratum).expect("every band");
        assert_eq!(band(Stratum::Month).size, Size::flat(10));
        assert_eq!(band(Stratum::Year).size, Size::zero(), "an empty band is still drawn");
        assert_eq!(band(Stratum::TwoYears).size, Size::flat(5));
        assert_eq!(band(Stratum::FiveYears).size, Size::flat(500 + 100 + 900));
        assert_eq!((band(Stratum::FiveYears).files), 3);
        assert_eq!(band(Stratum::Older).size, Size::flat(2000));
        assert_eq!(band(Stratum::Undated).files, 1);
    }

    #[test]
    fn the_untouched_view_holds_a_folder_by_its_old_bytes_and_lists_only_what_holds_any() {
        let tree = sample();
        let untouched = Untouched::of(&tree, now());
        let projects = tree.child(ROOT, "projects").expect("projects");
        let live = tree.child(projects, "live-app").expect("live-app");
        assert_eq!(untouched.size(live), Size::flat(500), "today's main.rs is not in it");
        assert_eq!(untouched.size(projects), Size::flat(500 + 1000));
        assert_eq!(
            untouched.size(ROOT),
            fossils(&tree, ROOT, now(), SizeBasis::Logical, 0).size,
            "the view and the fossils count the same bytes"
        );

        let sort = Sort {
            key: SortKey::Logical,
            direction: order::Direction::Descending,
        };
        let (page, total) = untouched.children_page(&tree, live, sort, SizeBasis::Logical, Span::default());
        assert_eq!(total, 1);
        assert_eq!(tree.node(page[0]).name, "target", "main.rs holds nothing untouched, and is not listed");
        let (page, _) = untouched.children_page(&tree, ROOT, sort, SizeBasis::Logical, Span::default());
        let names: Vec<&str> = page.iter().map(|&id| tree.node(id).name.as_str()).collect();
        assert_eq!(
            names,
            ["photos-2019", "projects", "note.txt"],
            "by what of each is untouched; the undated file is not old"
        );

        let tiles = untouched.tiles(&tree, projects, Layout::default());
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].bytes, 1000, "old-app, whole");
        assert_eq!(tiles[0].stratum, Some(Stratum::FiveYears));
        assert_eq!(tiles[1].bytes, 500, "live-app, by its build folder");
        assert_eq!(tiles[1].stratum, Some(Stratum::Month), "a living folder is coloured as alive");
    }

    /// How long the layers of a real folder take to work out, for a live
    /// check before a release:
    /// `MIDDA_LAYERS_ON=C:\some\folder cargo test --release -p midda-core layers_of_a_real_folder -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads a real folder named by MIDDA_LAYERS_ON"]
    fn layers_of_a_real_folder() {
        let Ok(root) = std::env::var("MIDDA_LAYERS_ON") else {
            return;
        };
        let tree = crate::scan(std::path::Path::new(&root), &crate::Progress::new()).expect("the scan succeeds");
        let now = SystemTime::now();
        let started = std::time::Instant::now();
        let strata = strata(&tree, ROOT, now);
        let took_strata = started.elapsed();
        let started = std::time::Instant::now();
        let fossils = fossils(&tree, ROOT, now, SizeBasis::Allocated, 30);
        let took_fossils = started.elapsed();
        let started = std::time::Instant::now();
        let untouched = Untouched::of(&tree, now);
        let took_untouched = started.elapsed();
        assert_eq!(untouched.size(ROOT), fossils.size);
        eprintln!(
            "{} entries; strata {took_strata:?}, fossils {took_fossils:?} ({} of them, {} B), untouched {took_untouched:?}; {:?}",
            tree.root().entries,
            fossils.count,
            fossils.size.allocated,
            strata.iter().map(|layer| (layer.stratum, layer.size.allocated)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_bands_meet_where_they_say() {
        assert_eq!(Stratum::of(Some(29 * DAY)), Stratum::Month);
        assert_eq!(Stratum::of(Some(30 * DAY)), Stratum::HalfYear);
        assert_eq!(Stratum::of(Some(181 * DAY)), Stratum::HalfYear);
        assert_eq!(Stratum::of(Some(182 * DAY)), Stratum::Year);
        assert_eq!(Stratum::of(Some(FOSSIL - DAY)), Stratum::Year);
        assert_eq!(Stratum::of(Some(FOSSIL)), Stratum::TwoYears);
        assert_eq!(Stratum::of(Some(2 * FOSSIL)), Stratum::FiveYears);
        assert_eq!(Stratum::of(Some(5 * FOSSIL)), Stratum::Older);
        assert_eq!(Stratum::of(None), Stratum::Undated);
        assert!(Stratum::TwoYears.is_fossil() && !Stratum::Year.is_fossil() && !Stratum::Undated.is_fossil());
    }
}
