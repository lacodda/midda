//! How a list of entries is ordered, and which slice of it the caller wants.
//!
//! Ordering lives here rather than in the window. A folder on a system volume
//! can hold hundreds of thousands of children, and sorting them on the other
//! side of a serialization boundary would mean sending every one of them across
//! it on every click of a column header. The core sorts and hands back the
//! window that is actually being drawn.
//!
//! The consequence worth stating: the CLI and the MCP door of v0.19 get exactly
//! this order, because there is only one implementation of it.

use serde::{Deserialize, Serialize};

use crate::size::SizeBasis;
use crate::tree::{Node, NodeId, Tree};

/// What a column is ordered by.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SortKey {
    /// What the volume gives up. midda's default, and the product's position.
    #[default]
    Allocated,
    /// What a reader would get out of the entry.
    Logical,
    /// The entry's own name.
    Name,
    /// The newest modification anywhere in the subtree — the age of the layer.
    Modified,
    /// How many entries are in the subtree.
    Entries,
}

/// Which way a column points.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// Smallest, earliest or first alphabetically at the top.
    Ascending,
    /// Largest, most recent or last alphabetically at the top. The default,
    /// because the first question anyone asks a disk analyzer is what is big.
    #[default]
    Descending,
}

/// A column and the way it points.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sort {
    pub key: SortKey,
    pub direction: Direction,
}

impl Sort {
    /// The order a freshly opened folder is shown in: biggest on disk first.
    #[must_use]
    pub const fn default_order() -> Self {
        Self {
            key: SortKey::Allocated,
            direction: Direction::Descending,
        }
    }

    /// The sort a click on `key` produces, given what is sorted now.
    ///
    /// Clicking the column already sorted flips it; clicking another column
    /// starts it at the direction that column is usually read in. A size column
    /// opens largest-first because that is the question being asked, and a name
    /// column opens A to Z because the alphabet has a direction people expect.
    #[must_use]
    pub fn toggled(self, key: SortKey) -> Self {
        if self.key == key {
            return Self {
                key,
                direction: self.direction.flipped(),
            };
        }
        Self {
            key,
            direction: key.natural_direction(),
        }
    }
}

impl Direction {
    /// The other way.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

impl SortKey {
    /// The direction this column is first shown in when it is picked.
    #[must_use]
    pub const fn natural_direction(self) -> Direction {
        match self {
            // "Which of these is big" and "what changed lately" are both
            // largest-first questions.
            Self::Allocated | Self::Logical | Self::Modified | Self::Entries => Direction::Descending,
            // The alphabet runs one way.
            Self::Name => Direction::Ascending,
        }
    }

    /// Which size this key reads, when it reads one.
    #[must_use]
    pub const fn basis(self) -> Option<SizeBasis> {
        match self {
            Self::Allocated => Some(SizeBasis::Allocated),
            Self::Logical => Some(SizeBasis::Logical),
            _ => None,
        }
    }
}

/// What a node compares as, under one key.
///
/// `Absent` is its own variant rather than a zero. A folder whose filesystem
/// would not report a timestamp is *unknown*, not *oldest*, and the two are
/// different facts: rank absence with the numbers and flip the sign, and a
/// descending sort floats every undated row to the top — the reader asks for
/// "newest first" and is handed the rows that have no date at all.
///
/// This is the same rule dowel's `table-sort` is built on, held here because
/// the sorting happens here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value<'a> {
    Number(u64),
    Text(&'a str),
    Absent,
}

/// Reads what a node compares as under `key`.
fn value_of<'a>(node: &'a Node, key: SortKey) -> Value<'a> {
    match key {
        SortKey::Allocated => Value::Number(node.size.allocated),
        SortKey::Logical => Value::Number(node.size.logical),
        SortKey::Entries => Value::Number(node.entries),
        SortKey::Name => Value::Text(&node.name),
        SortKey::Modified => node.subtree_modified.map_or(Value::Absent, |time| {
            // Before the epoch is a clock that was wrong, not a real date, and
            // it compares as the earliest thing there is rather than as absent:
            // the timestamp exists, it is merely implausible.
            Value::Number(time.duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs()))
        }),
    }
}

/// Compares two values that are both present.
///
/// Names compare case-insensitively first, because a reader scanning an
/// alphabetical list does not think of `Downloads` and `desktop` as living in
/// different halves of it. Exact order breaks the tie so the result is stable.
fn compare_present(a: &Value<'_>, b: &Value<'_>) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.cmp(b),
        (Value::Text(a), Value::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)),
        // Mixed kinds cannot happen: both come from the same key.
        _ => std::cmp::Ordering::Equal,
    }
}

/// Orders the children of `id`.
///
/// Absent values sort last whichever way the column points. Ties break on name,
/// so two runs of the same scan produce the same list — a table that reshuffles
/// equal rows on every rescan looks broken.
#[must_use]
pub fn children(tree: &Tree, id: NodeId, sort: Sort) -> Vec<NodeId> {
    let mut children = tree.node(id).children.clone();

    children.sort_unstable_by(|&a, &b| {
        let (left, right) = (tree.node(a), tree.node(b));
        let (a_value, b_value) = (value_of(left, sort.key), value_of(right, sort.key));

        // Presence is settled before the direction is applied, which is the
        // whole point: flipping the sort must not float the unknown rows up.
        let ordering = match (&a_value, &b_value) {
            (Value::Absent, Value::Absent) => std::cmp::Ordering::Equal,
            (Value::Absent, _) => return std::cmp::Ordering::Greater,
            (_, Value::Absent) => return std::cmp::Ordering::Less,
            _ => {
                let ordering = compare_present(&a_value, &b_value);
                match sort.direction {
                    Direction::Ascending => ordering,
                    Direction::Descending => ordering.reverse(),
                }
            }
        };

        // The tiebreaker is always ascending by name, whatever the column is
        // doing: a reader who cannot see why two rows are in this order at
        // least finds them alphabetical.
        ordering.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    children
}

/// A slice of an ordered list: what the window is actually drawing.
///
/// Asked for rather than inferred, because the window knows how tall it is and
/// the core does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    /// How many rows to skip.
    pub offset: u32,
    /// How many rows to take. Clamped by the caller's own limit so a window
    /// cannot ask for a million rows by asking for `u32::MAX`.
    pub limit: u32,
}

impl Default for Span {
    fn default() -> Self {
        // A screenful and then some. A window that asks for nothing would
        // otherwise render an empty folder that is not empty.
        Self { offset: 0, limit: 200 }
    }
}

/// The most rows one call will return.
///
/// A window draws a screenful; anything past a few hundred is a window that has
/// not told the truth about its height, and honouring it would mean building a
/// megabyte of JSON nobody looks at.
pub const MAX_SPAN: u32 = 1000;

impl Span {
    /// The range this span covers within a list of `total` rows.
    ///
    /// Returns an empty range when the offset is past the end — a folder that
    /// shrank under the reader is a real state, not an error.
    #[must_use]
    pub fn range(self, total: usize) -> std::ops::Range<usize> {
        let offset = (self.offset as usize).min(total);
        let limit = (self.limit.min(MAX_SPAN)) as usize;
        offset..(offset.saturating_add(limit)).min(total)
    }
}

/// One page of the children of `id`, ordered by `sort`.
///
/// Returns the ids in the page and how many children there are in total, so the
/// window can size its scrollbar without holding every row.
#[must_use]
pub fn children_page(tree: &Tree, id: NodeId, sort: Sort, span: Span) -> (Vec<NodeId>, usize) {
    let ordered = children(tree, id, sort);
    let total = ordered.len();
    let range = span.range(total);
    (ordered[range].to_vec(), total)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::size::Size;
    use crate::tree::{Kind, Node, ROOT, Tree};

    fn at(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    fn entry(name: &str, allocated: u64, logical: u64, modified: Option<SystemTime>, entries: u64) -> Node {
        Node {
            name: name.into(),
            parent: ROOT,
            kind: Kind::File,
            size: Size { logical, allocated },
            modified,
            subtree_modified: modified,
            entries,
            children: Vec::new(),
        }
    }

    /// Four entries that disagree under every key, so no two keys can be
    /// confused for one another by accident.
    fn sample() -> Tree {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        // name, allocated, logical, modified, entries
        tree.push(ROOT, entry("delta", 400, 4000, at(100), 4));
        tree.push(ROOT, entry("alpha", 300, 3000, at(400), 1));
        tree.push(ROOT, entry("Charlie", 200, 2000, None, 3));
        tree.push(ROOT, entry("bravo", 100, 1000, at(200), 2));
        tree
    }

    fn names(tree: &Tree, sort: Sort) -> Vec<String> {
        children(tree, ROOT, sort).into_iter().map(|id| tree.node(id).name.clone()).collect()
    }

    #[test]
    fn the_default_order_is_biggest_on_disk_first() {
        let tree = sample();
        assert_eq!(names(&tree, Sort::default_order()), ["delta", "alpha", "Charlie", "bravo"]);
        assert_eq!(Sort::default(), Sort::default_order());
    }

    #[test]
    fn the_two_sizes_can_disagree_and_the_key_decides() {
        // Allocated descends delta..bravo; logical ascends the same names. If
        // the two keys were ever confused the orders would match.
        let tree = sample();
        let by_logical = Sort {
            key: SortKey::Logical,
            direction: Direction::Ascending,
        };
        assert_eq!(names(&tree, by_logical), ["bravo", "Charlie", "alpha", "delta"]);
    }

    #[test]
    fn names_sort_without_regard_to_case() {
        // A reader scanning an alphabetical list does not think of `Charlie`
        // and `bravo` as living in different halves of it.
        let tree = sample();
        let by_name = Sort {
            key: SortKey::Name,
            direction: Direction::Ascending,
        };
        assert_eq!(names(&tree, by_name), ["alpha", "bravo", "Charlie", "delta"]);
    }

    #[test]
    fn an_undated_row_sorts_last_whichever_way_the_column_points() {
        // The rule this module exists for. `Charlie` has no timestamp: it is
        // unknown, not oldest, so it belongs at the bottom in both directions.
        // Rank it as a zero and a descending sort puts it at the top.
        let tree = sample();

        let newest_first = Sort {
            key: SortKey::Modified,
            direction: Direction::Descending,
        };
        assert_eq!(names(&tree, newest_first), ["alpha", "bravo", "delta", "Charlie"]);

        let oldest_first = Sort {
            key: SortKey::Modified,
            direction: Direction::Ascending,
        };
        assert_eq!(names(&tree, oldest_first), ["delta", "bravo", "alpha", "Charlie"]);
    }

    #[test]
    fn several_undated_rows_are_alphabetical_among_themselves() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        tree.push(ROOT, entry("zulu", 1, 1, None, 1));
        tree.push(ROOT, entry("alpha", 1, 1, None, 1));
        tree.push(ROOT, entry("dated", 1, 1, at(50), 1));

        let by_date = Sort {
            key: SortKey::Modified,
            direction: Direction::Descending,
        };
        assert_eq!(names(&tree, by_date), ["dated", "alpha", "zulu"]);
    }

    #[test]
    fn equal_rows_keep_the_same_order_between_two_runs() {
        // A table that reshuffles equal rows on every rescan looks broken.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        for name in ["Zebra", "apple", "Mango", "banana"] {
            tree.push(ROOT, entry(name, 4096, 4096, at(10), 1));
        }

        let sort = Sort::default_order();
        assert_eq!(names(&tree, sort), ["apple", "banana", "Mango", "Zebra"]);
        // And the tiebreaker does not flip with the column.
        let ascending = Sort {
            key: SortKey::Allocated,
            direction: Direction::Ascending,
        };
        assert_eq!(names(&tree, ascending), ["apple", "banana", "Mango", "Zebra"]);
    }

    #[test]
    fn sorting_by_entry_count_finds_the_folder_full_of_small_things() {
        let tree = sample();
        let by_entries = Sort {
            key: SortKey::Entries,
            direction: Direction::Descending,
        };
        assert_eq!(names(&tree, by_entries), ["delta", "Charlie", "bravo", "alpha"]);
    }

    #[test]
    fn clicking_the_sorted_column_flips_it_and_another_column_starts_naturally() {
        let sort = Sort::default_order();

        let flipped = sort.toggled(SortKey::Allocated);
        assert_eq!(
            flipped,
            Sort {
                key: SortKey::Allocated,
                direction: Direction::Ascending
            }
        );

        // A size question opens largest-first; the alphabet opens at A.
        assert_eq!(sort.toggled(SortKey::Name).direction, Direction::Ascending);
        assert_eq!(sort.toggled(SortKey::Modified).direction, Direction::Descending);
        assert_eq!(sort.toggled(SortKey::Entries).direction, Direction::Descending);
    }

    #[test]
    fn a_timestamp_before_the_epoch_is_early_rather_than_missing() {
        // A clock that was wrong still produced a date. It sorts as the
        // earliest thing there is, not as unknown — a row with no date at all
        // still belongs below it.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        tree.push(ROOT, entry("prehistoric", 1, 1, Some(SystemTime::UNIX_EPOCH - Duration::from_secs(86_400)), 1));
        tree.push(ROOT, entry("dated", 1, 1, at(50), 1));
        tree.push(ROOT, entry("undated", 1, 1, None, 1));

        let oldest_first = Sort {
            key: SortKey::Modified,
            direction: Direction::Ascending,
        };
        assert_eq!(names(&tree, oldest_first), ["prehistoric", "dated", "undated"]);
    }

    #[test]
    fn a_key_says_which_size_it_reads() {
        assert_eq!(SortKey::Allocated.basis(), Some(SizeBasis::Allocated));
        assert_eq!(SortKey::Logical.basis(), Some(SizeBasis::Logical));
        assert_eq!(SortKey::Name.basis(), None);
    }

    #[test]
    fn a_page_is_a_slice_of_the_ordered_list() {
        let tree = sample();
        let (page, total) = children_page(&tree, ROOT, Sort::default_order(), Span { offset: 1, limit: 2 });
        assert_eq!(total, 4, "the total is the whole folder, not the page");
        assert_eq!(page.iter().map(|&id| tree.node(id).name.as_str()).collect::<Vec<_>>(), ["alpha", "Charlie"]);
    }

    #[test]
    fn a_page_past_the_end_is_empty_rather_than_an_error() {
        // A folder that shrank under the reader is a real state.
        let tree = sample();
        let (page, total) = children_page(&tree, ROOT, Sort::default_order(), Span { offset: 99, limit: 10 });
        assert!(page.is_empty());
        assert_eq!(total, 4);
    }

    #[test]
    fn a_page_cannot_ask_for_more_than_the_cap() {
        // Honouring `u32::MAX` would mean building a megabyte of rows nobody
        // looks at.
        let span = Span { offset: 0, limit: u32::MAX };
        assert_eq!(span.range(10_000).len(), MAX_SPAN as usize);
    }

    #[test]
    fn a_page_is_clipped_to_what_is_there() {
        let span = Span { offset: 3, limit: 100 };
        assert_eq!(span.range(5), 3..5);
    }

    #[test]
    fn paging_through_a_folder_visits_every_row_once() {
        // The property that matters: a window scrolling a long list must not
        // skip a row or show one twice.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        for n in 0..250_u64 {
            tree.push(ROOT, entry(&format!("file-{n:04}"), n, n, at(n), 1));
        }

        let sort = Sort::default_order();
        let mut seen = Vec::new();
        let mut offset = 0;
        loop {
            let (page, total) = children_page(&tree, ROOT, sort, Span { offset, limit: 40 });
            assert_eq!(total, 250);
            if page.is_empty() {
                break;
            }
            offset += u32::try_from(page.len()).expect("a page fits in u32");
            seen.extend(page);
        }

        assert_eq!(seen.len(), 250);
        let whole = children(&tree, ROOT, sort);
        assert_eq!(seen, whole, "paging produced a different order from reading it all at once");
    }
}
