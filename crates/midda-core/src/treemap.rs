//! The picture of a folder: rectangles whose area is what each child occupies.
//!
//! A table answers "which of these is biggest" one row at a time. A treemap
//! answers "what does this folder look like" in one glance — and the whole
//! argument of the product is that the glance is the fast way to find the forty
//! gigabytes nobody meant to keep.
//!
//! # Why this is in the core
//!
//! Squarified layout is arithmetic over a list of values and a rectangle. It is
//! not about pixels: this module works in *fractions* of the box it is given,
//! and the window multiplies by whatever size it happens to be. Keeping it here
//! means the CLI and the MCP door of v0.19 draw the same picture — the same
//! order, the same cut-off, the same "everything else" — without a second
//! implementation to drift.
//!
//! # Why the cut-off is here too
//!
//! A folder can hold hundreds of thousands of children. Sending all of them to
//! the window so it can decide which are too small to draw would send megabytes
//! to draw a hundred rectangles. The decision needs the area, and the area is
//! computed here — so the cut-off is made here, and what falls under it comes
//! back as one rectangle that says how many entries are in it. A picture that
//! silently dropped them would not add up to the folder it claims to show.

use serde::{Deserialize, Serialize};

use crate::size::SizeBasis;
use crate::tree::{Kind, NodeId, Tree};

/// A rectangle in fractions of the box being drawn.
///
/// `0.0..=1.0` on both axes, origin top-left. Fractions rather than pixels
/// because the core does not know how big the window is, and because a window
/// that resizes should not have to ask again.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// The whole box.
    #[must_use]
    pub const fn unit() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }

    /// The shorter of the two sides — which way a squarified row is laid along.
    #[must_use]
    fn shorter_side(self) -> f64 {
        self.width.min(self.height)
    }

    /// How far from square this is, where 1.0 is a square. Used only in tests
    /// and by callers that want to judge a layout.
    #[must_use]
    pub fn aspect(self) -> f64 {
        if self.width <= 0.0 || self.height <= 0.0 {
            return f64::INFINITY;
        }
        (self.width / self.height).max(self.height / self.width)
    }
}

/// One rectangle of a laid-out treemap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    /// The node this stands for, or `None` for the tile that gathers what was
    /// too small to draw.
    pub id: Option<NodeId>,
    /// What to write on it.
    pub name: String,
    /// The bytes this tile's area is proportional to, on the basis asked for.
    pub bytes: u64,
    /// Where it sits, in fractions of the box.
    pub rect: Rect,
    /// Whether the node is a directory — a tile you can descend into.
    pub is_directory: bool,
    /// What kind of thing this is, for the colour.
    pub category: Category,
    /// How many entries this tile stands for. One for a file, the subtree count
    /// for a directory, and the number gathered for the remainder tile.
    pub entries: u64,
    /// The full path, for the tooltip. Empty for the remainder tile, which
    /// stands for many paths and so has none.
    ///
    /// Carried on the tile rather than fetched on hover: a layout is asked for
    /// when the folder, the basis or the window's shape changes, and a hover
    /// happens sixty times a second. Two hundred paths is a few kilobytes once;
    /// a round trip per hover is a round trip per frame.
    pub path: String,
}

/// What a tile is made of, as far as the colour is concerned.
///
/// Deliberately coarse. The rules engine of v0.9 will say "this is 8.9 GB of
/// `node_modules`, recreated by `npm install`" — that is a *finding*, with an
/// explanation attached, and it is not this. This is the four-way read a person
/// gets from the picture before they have read a single label: is this my work,
/// is it media, is it something a build made, or is it something else.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Something a tool made and a tool can make again: build output, caches,
    /// dependency directories. The colour the eye should go to first.
    Build,
    /// Source, documents, configuration — the things that are typed.
    Source,
    /// Images, audio, video, archives, disk images.
    Media,
    /// Everything not recognised.
    #[default]
    Other,
}

/// Directory names that are build output or a cache, whatever sits above them.
///
/// Names rather than paths on purpose: `node_modules` is `node_modules`
/// wherever it is found, and a path pattern would need a project root to
/// anchor to. The rules engine of v0.9 does the anchored, explained version;
/// this is the colour of a rectangle.
const BUILD_DIRECTORIES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "bin",
    "obj",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".gradle",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".cache",
    "cache",
    "caches",
    ".ccache",
    "vendor",
    "packages",
    "Debug",
    "Release",
];

/// Extensions of things that are typed, or that describe what was typed.
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "c", "h", "cpp", "hpp", "cs", "rb", "php", "swift", "sh", "ps1", "sql", "html",
    "css", "scss", "md", "txt", "json", "toml", "yaml", "yml", "xml", "ini", "cfg", "lock", "csv",
];

/// Extensions of things that were recorded, drawn or packed.
const MEDIA_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico", "tiff", "svg", "psd", "mp3", "wav", "flac", "ogg", "m4a", "aac", "mp4", "mkv", "mov", "avi",
    "webm", "wmv", "zip", "7z", "rar", "gz", "bz2", "xz", "zst", "tar", "iso", "vhd", "vhdx", "vmdk", "pdf",
];

impl Category {
    /// What to colour a node.
    ///
    /// A directory is read by its name; a file by its extension. A directory
    /// under a build directory is build output too — but that is not decided
    /// here, because this sees one node: the caller passes the parent's
    /// category down, and anything inside build output inherits it. Without
    /// that, the inside of `node_modules` would be coloured as source, which
    /// is exactly backwards.
    #[must_use]
    pub fn of(name: &str, kind: Kind, inherited: Option<Self>) -> Self {
        // Inheritance wins over the node's own look: a `src` directory inside
        // `node_modules` is still something `npm install` recreates.
        if let Some(Self::Build) = inherited {
            return Self::Build;
        }

        match kind {
            Kind::Directory => {
                if BUILD_DIRECTORIES.iter().any(|known| known.eq_ignore_ascii_case(name)) {
                    Self::Build
                } else {
                    Self::Other
                }
            }
            Kind::File => {
                let extension = name.rsplit_once('.').map_or("", |(_, tail)| tail).to_ascii_lowercase();
                if extension.is_empty() {
                    return Self::Other;
                }
                if SOURCE_EXTENSIONS.contains(&extension.as_str()) {
                    Self::Source
                } else if MEDIA_EXTENSIONS.contains(&extension.as_str()) {
                    Self::Media
                } else {
                    Self::Other
                }
            }
        }
    }
}

/// What the window is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    /// Which size the areas are proportional to.
    pub basis: SizeBasis,
    /// How wide the box is, relative to its height.
    ///
    /// The layout works in fractions, but *squaring* needs to know the real
    /// shape: a row that looks square in a 2:1 box is not square on screen.
    /// The window passes width/height and the arithmetic corrects for it.
    pub aspect: f64,
    /// The smallest fraction of the box a tile may take before it is gathered
    /// into the remainder.
    ///
    /// A fraction rather than pixels, again because the core does not know the
    /// window's size — the window divides its own smallest useful rectangle by
    /// its area and passes the result.
    pub min_area: f64,
    /// The most tiles to lay out, whatever their size.
    ///
    /// A hard stop for the pathological folder: ten thousand children each
    /// worth a tenth of a percent would all clear `min_area` on a large screen
    /// and take a second to draw.
    pub max_tiles: usize,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            basis: SizeBasis::Allocated,
            aspect: 1.0,
            // A thousandth of the box: on a 900×600 map that is a rectangle of
            // about 540 square pixels — roughly 23×23, the smallest thing worth
            // a border and a hover.
            min_area: 0.001,
            max_tiles: 250,
        }
    }
}

/// The most tiles one call will return, whatever the caller asks for.
///
/// A window draws a picture, and a picture of more than a few hundred
/// rectangles is a grey rectangle. This is the same argument as `MAX_SPAN` in
/// [`crate::order`], for the same reason: an honest cap beats an honoured
/// absurdity.
pub const MAX_TILES: usize = 1000;

/// The rectangles for the children of `id`.
///
/// Returned largest-first, which is both what squarified layout needs and the
/// order the table shows by default — the two views agree about which thing is
/// the big one.
///
/// Children that would be too small to see are gathered into a final tile with
/// `id: None`, so the picture still adds up to the folder. A folder whose
/// children are all tiny gets one tile rather than a hundred thousand.
#[must_use]
pub fn tiles(tree: &Tree, id: NodeId, layout: Layout) -> Vec<Tile> {
    let parent_category = category_of(tree, id);

    let mut weighted: Vec<(NodeId, u64)> = tree
        .node(id)
        .children
        .iter()
        .map(|&child| (child, bytes_of(tree, child, layout.basis)))
        .filter(|&(_, bytes)| bytes > 0)
        .collect();

    // Largest first, ties broken on name so the picture is the same between two
    // runs of the same scan — a treemap that reshuffles equal tiles looks like
    // the disk changed.
    weighted.sort_unstable_by(|&(a, a_bytes), &(b, b_bytes)| {
        b_bytes
            .cmp(&a_bytes)
            .then_with(|| tree.node(a).name.to_lowercase().cmp(&tree.node(b).name.to_lowercase()))
    });

    let total: u64 = weighted.iter().map(|&(_, bytes)| bytes).sum();
    if total == 0 {
        return Vec::new();
    }

    let cap = layout.max_tiles.clamp(1, MAX_TILES);
    let min_area = layout.min_area.clamp(0.0, 1.0);

    // Where to stop drawing one tile per child. Two conditions, and the first
    // one met wins: the tile would be too small to see, or there are already
    // more tiles than a picture can carry. One slot is held back for the
    // remainder when there is going to be one.
    #[allow(clippy::cast_precision_loss, reason = "a byte count past 2^53 is not a disk")]
    let drawn = weighted
        .iter()
        .take(cap)
        .position(|&(_, bytes)| (bytes as f64) / (total as f64) < min_area)
        .unwrap_or_else(|| weighted.len().min(cap));

    let (head, tail) = weighted.split_at(drawn);

    let mut values: Vec<f64> = head
        .iter()
        .map(|&(_, bytes)| {
            #[allow(clippy::cast_precision_loss, reason = "a byte count past 2^53 is not a disk")]
            let share = (bytes as f64) / (total as f64);
            share
        })
        .collect();

    let gathered: u64 = tail.iter().map(|&(_, bytes)| bytes).sum();
    if gathered > 0 {
        #[allow(clippy::cast_precision_loss, reason = "a byte count past 2^53 is not a disk")]
        let share = (gathered as f64) / (total as f64);
        values.push(share);
    }

    let rects = squarify(&values, Rect::unit(), layout.aspect);

    let mut tiles: Vec<Tile> = head
        .iter()
        .zip(rects.iter())
        .map(|(&(child, bytes), &rect)| {
            let node = tree.node(child);
            Tile {
                id: Some(child),
                name: node.name.clone(),
                bytes,
                rect,
                is_directory: node.is_directory(),
                category: Category::of(&node.name, node.kind, parent_category),
                entries: node.entries,
                path: tree.path_of(child).to_string_lossy().into_owned(),
            }
        })
        .collect();

    if gathered > 0
        && let Some(&rect) = rects.last()
    {
        let count = tail.iter().map(|&(child, _)| tree.node(child).entries).sum();
        tiles.push(Tile {
            id: None,
            name: remainder_name(tail.len()),
            bytes: gathered,
            rect,
            is_directory: false,
            category: Category::Other,
            entries: count,
            path: String::new(),
        });
    }

    tiles
}

/// What the gathered tile is called.
fn remainder_name(count: usize) -> String {
    if count == 1 {
        "1 smaller entry".to_owned()
    } else {
        format!("{count} smaller entries")
    }
}

/// The size a node contributes, on one basis.
fn bytes_of(tree: &Tree, id: NodeId, basis: SizeBasis) -> u64 {
    let size = tree.node(id).size;
    match basis {
        SizeBasis::Allocated => size.allocated,
        SizeBasis::Logical => size.logical,
    }
}

/// The category a node inherits to its children, walking up to the root.
///
/// Walks rather than caches: the depth of a path is tens, not thousands, and a
/// cached column on every node would be a second thing to keep in step with the
/// scan.
fn category_of(tree: &Tree, id: NodeId) -> Option<Category> {
    let mut at = id;
    loop {
        let node = tree.node(at);
        if node.is_directory() && BUILD_DIRECTORIES.iter().any(|known| known.eq_ignore_ascii_case(&node.name)) {
            return Some(Category::Build);
        }
        if at == crate::tree::ROOT {
            return None;
        }
        at = node.parent;
    }
}

/// Lays `values` out inside `rect` so the rectangles come out as square as they
/// can.
///
/// The squarified algorithm (Bruls, Huizing, van Wijk, 2000): take the values
/// largest-first, keep adding them to the current row while doing so improves
/// the worst aspect ratio in that row, and lay the row along the shorter side
/// of what is left. The alternative — slice-and-dice, one strip per value — is
/// two lines of code and produces slivers a person cannot click, which is the
/// whole reason the paper exists.
///
/// `values` are shares that should sum to 1.0; `aspect` is the real
/// width-over-height of the box, so "square" means square on screen rather than
/// square in fraction space.
#[must_use]
pub fn squarify(values: &[f64], rect: Rect, aspect: f64) -> Vec<Rect> {
    // Zero rather than the unit box: a value of zero gets no rectangle, and an
    // uninitialised one that read as the whole box would cover everything.
    let mut out = vec![
        Rect {
            x: rect.x,
            y: rect.y,
            width: 0.0,
            height: 0.0,
        };
        values.len()
    ];
    if values.is_empty() {
        return Vec::new();
    }

    // The scale that turns fraction space into screen shape. A box twice as
    // wide as it is tall makes a fraction-square rectangle look like a 2:1
    // sliver, and the row-breaking decision has to see what the reader sees.
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };

    let mut remaining: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .map(|(index, &value)| (index, value.max(0.0)))
        .filter(|&(_, value)| value > 0.0)
        .collect();

    let sum: f64 = remaining.iter().map(|&(_, value)| value).sum();
    if sum <= 0.0 {
        return out;
    }

    // Work in the area of the box we were handed, whatever the values add up
    // to: a caller that passes shares summing to 0.98 should still fill it.
    let area = rect.width * rect.height;
    let scale = area / sum;
    for entry in &mut remaining {
        entry.1 *= scale;
    }

    let mut free = rect;
    let mut row: Vec<(usize, f64)> = Vec::new();
    let mut at = 0;

    while at < remaining.len() {
        let candidate = remaining[at];
        let side = free.shorter_side();
        if side <= 0.0 {
            break;
        }

        let current = worst_ratio(&row, side, free, aspect);
        let mut extended = row.clone();
        extended.push(candidate);
        let next = worst_ratio(&extended, side, free, aspect);

        if row.is_empty() || next <= current {
            row = extended;
            at += 1;
            continue;
        }

        free = place_row(&row, free, &mut out);
        row = Vec::new();
    }

    if !row.is_empty() {
        place_row(&row, free, &mut out);
    }

    out
}

/// The worst aspect ratio in a row laid along `side` of `free`, corrected for
/// the box's real shape.
///
/// Returns infinity for an empty row, so the first value is always accepted.
fn worst_ratio(row: &[(usize, f64)], side: f64, free: Rect, aspect: f64) -> f64 {
    if row.is_empty() {
        return f64::INFINITY;
    }

    let sum: f64 = row.iter().map(|&(_, area)| area).sum();
    if sum <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }

    // How thick the row is: its area spread along the side it runs on.
    let thickness = sum / side;

    // A row runs *along the shorter side*: that is what keeps its tiles from
    // being slivers, and it is the whole trick of the algorithm. When the
    // shorter side is the height, the row is a vertical strip — the tiles are
    // stacked down it and their width is the row's thickness.
    let horizontal = free.width <= free.height;

    row.iter()
        .map(|&(_, area)| {
            let length = area / thickness;
            let (width, height) = if horizontal { (length, thickness) } else { (thickness, length) };
            // Correct to screen proportions before judging squareness.
            let (width, height) = (width * aspect, height);
            if width <= 0.0 || height <= 0.0 {
                return f64::INFINITY;
            }
            (width / height).max(height / width)
        })
        .fold(0.0_f64, f64::max)
}

/// Puts a finished row into `out` and returns what is left of `free`.
fn place_row(row: &[(usize, f64)], free: Rect, out: &mut [Rect]) -> Rect {
    let sum: f64 = row.iter().map(|&(_, area)| area).sum();
    // The same choice `worst_ratio` made, and it has to be the same one: the
    // ratios were judged for a row running this way.
    let horizontal = free.width <= free.height;
    let side = free.shorter_side();
    if side <= 0.0 || sum <= 0.0 {
        return free;
    }

    // Not clamped to the free side. A clamp looks like a guard and is a lie:
    // it would keep the rectangle inside the box by making its area wrong,
    // which is the one thing a treemap may not do. The last row's thickness is
    // exactly what is left, because the areas were scaled to the box.
    let thickness = sum / side;

    let mut along = 0.0;
    for &(index, area) in row {
        let length = area / thickness;
        out[index] = if horizontal {
            Rect {
                x: free.x + along,
                y: free.y,
                width: length,
                height: thickness,
            }
        } else {
            Rect {
                x: free.x,
                y: free.y + along,
                width: thickness,
                height: length,
            }
        };
        along += length;
    }

    if horizontal {
        Rect {
            x: free.x,
            y: free.y + thickness,
            width: free.width,
            height: (free.height - thickness).max(0.0),
        }
    } else {
        Rect {
            x: free.x + thickness,
            y: free.y,
            width: (free.width - thickness).max(0.0),
            height: free.height,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::size::Size;
    use crate::tree::{Node, ROOT};

    fn node(name: &str, parent: NodeId, kind: Kind, allocated: u64) -> Node {
        Node {
            name: name.into(),
            parent,
            kind,
            size: Size { logical: allocated, allocated },
            modified: None,
            subtree_modified: None,
            entries: 1,
            children: Vec::new(),
        }
    }

    /// How much of the box a set of rectangles covers.
    fn covered(rects: &[Rect]) -> f64 {
        rects.iter().map(|rect| rect.width * rect.height).sum()
    }

    /// Whether any two rectangles overlap. The property a treemap lives or dies
    /// by: an overlap means one file is drawn on top of another and the picture
    /// is a lie about who owns the space.
    fn overlaps(rects: &[Rect]) -> bool {
        const EPS: f64 = 1e-9;
        rects.iter().enumerate().any(|(i, a)| {
            rects
                .iter()
                .skip(i + 1)
                .any(|b| a.x < b.x + b.width - EPS && b.x < a.x + a.width - EPS && a.y < b.y + b.height - EPS && b.y < a.y + a.height - EPS)
        })
    }

    #[test]
    fn the_rectangles_fill_the_box_exactly_once() {
        // The property the whole picture rests on: every share drawn, none
        // drawn twice, nothing left blank.
        let values = [0.4, 0.25, 0.15, 0.1, 0.06, 0.04];
        let rects = squarify(&values, Rect::unit(), 1.0);

        assert!((covered(&rects) - 1.0).abs() < 1e-9, "the box is filled: {}", covered(&rects));
        assert!(!overlaps(&rects), "no two tiles may cover the same pixel");
    }

    #[test]
    fn each_rectangle_has_the_area_it_was_given() {
        // The one thing a treemap promises. A tile for 40% of a folder that
        // covers 30% of the box is the picture lying about the disk.
        let values = [0.5, 0.3, 0.12, 0.08];
        let rects = squarify(&values, Rect::unit(), 1.0);

        for (&value, rect) in values.iter().zip(rects.iter()) {
            let area = rect.width * rect.height;
            assert!((area - value).abs() < 1e-9, "a tile of {value} covers {area}");
        }
    }

    #[test]
    fn squarified_beats_a_row_of_slivers() {
        // The reason this is not four lines of slice-and-dice: laid out in one
        // strip, twenty equal values give tiles twenty times longer than they
        // are tall, and a person cannot hit one with a mouse.
        let values = [0.05_f64; 20];
        let rects = squarify(&values, Rect::unit(), 1.0);

        let worst = rects.iter().map(|rect| rect.aspect()).fold(0.0_f64, f64::max);
        assert!(worst < 3.0, "the worst tile is {worst}:1, which is a sliver");
    }

    #[test]
    fn a_wide_box_is_squared_against_the_screen_not_the_fractions() {
        // A rectangle that is square in fraction space is a 3:1 sliver in a box
        // three times as wide as it is tall. The layout has to see what the
        // reader sees, which is what `aspect` is for.
        let values = [0.25_f64; 4];
        let rects = squarify(&values, Rect::unit(), 3.0);

        let worst = rects
            .iter()
            .map(|rect| {
                // Judge in screen proportions: width is stretched by 3.
                let (width, height) = (rect.width * 3.0, rect.height);
                (width / height).max(height / width)
            })
            .fold(0.0_f64, f64::max);
        assert!(worst < 2.0, "the worst tile on screen is {worst}:1");
    }

    #[test]
    fn the_row_that_is_judged_is_the_row_that_is_placed() {
        // Two places decide which way a row runs: the one that judges whether
        // the row is still improving, and the one that lays it out. If they
        // disagree, the layout is still valid — areas exact, no overlaps — and
        // every property test above still passes. What breaks is the only
        // thing the algorithm is for: the tiles stop being square.
        //
        // Found by mutation: flipping the comparison in `worst_ratio` alone
        // changed nothing any test could see. A realistic folder — one big
        // thing and a long tail — in a box that is not square is what tells
        // them apart.
        let values = [0.34, 0.21, 0.13, 0.09, 0.07, 0.05, 0.04, 0.03, 0.025, 0.015];
        let aspect = 1.6;
        let rects = squarify(&values, Rect::unit(), aspect);

        let worst = rects
            .iter()
            .map(|rect| {
                let (width, height) = (rect.width * aspect, rect.height);
                if width <= 0.0 || height <= 0.0 {
                    return f64::INFINITY;
                }
                (width / height).max(height / width)
            })
            .fold(0.0_f64, f64::max);

        // A correct squarified layout of this keeps every tile under 3:1. The
        // two halves disagreeing pushes it past 4:1.
        assert!(worst < 3.0, "the worst tile on screen is {worst:.2}:1");
    }

    #[test]
    fn one_value_takes_the_whole_box() {
        let rects = squarify(&[1.0], Rect::unit(), 1.0);
        assert_eq!(rects.len(), 1);
        assert!((rects[0].width - 1.0).abs() < 1e-9);
        assert!((rects[0].height - 1.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_to_lay_out_is_not_an_error() {
        // An empty folder is a real state, and a picture of it is no picture.
        assert!(squarify(&[], Rect::unit(), 1.0).is_empty());
        assert_eq!(squarify(&[0.0, 0.0], Rect::unit(), 1.0).len(), 2);
    }

    /// root / { big (600), medium (300), small (100) }
    fn sample() -> Tree {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        let big = tree.push(ROOT, node("big", ROOT, Kind::Directory, 0));
        tree.push(big, node("payload.bin", big, Kind::File, 600));
        let medium = tree.push(ROOT, node("medium", ROOT, Kind::Directory, 0));
        tree.push(medium, node("payload.bin", medium, Kind::File, 300));
        tree.push(ROOT, node("small.txt", ROOT, Kind::File, 100));
        tree.roll_up();
        tree
    }

    #[test]
    fn a_folder_becomes_tiles_largest_first() {
        let tree = sample();
        let tiles = tiles(&tree, ROOT, Layout::default());

        assert_eq!(tiles.iter().map(|tile| tile.name.as_str()).collect::<Vec<_>>(), ["big", "medium", "small.txt"]);
        // And their areas are their shares.
        let rects: Vec<Rect> = tiles.iter().map(|tile| tile.rect).collect();
        assert!((covered(&rects) - 1.0).abs() < 1e-9);
        assert!((rects[0].width * rects[0].height - 0.6).abs() < 1e-9);
    }

    #[test]
    fn the_basis_changes_the_picture() {
        // The two sizes disagreeing is the product's whole argument, and the
        // picture has to be able to show either one.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        tree.push(
            ROOT,
            Node {
                size: Size { logical: 10, allocated: 4096 },
                ..node("many-tiny", ROOT, Kind::Directory, 0)
            },
        );
        tree.push(
            ROOT,
            Node {
                size: Size {
                    logical: 2048,
                    allocated: 2048,
                },
                ..node("one-real", ROOT, Kind::File, 0)
            },
        );
        tree.roll_up();

        let on_disk = tiles(&tree, ROOT, Layout::default());
        assert_eq!(on_disk[0].name, "many-tiny");

        let logical = tiles(
            &tree,
            ROOT,
            Layout {
                basis: SizeBasis::Logical,
                ..Layout::default()
            },
        );
        assert_eq!(logical[0].name, "one-real");
    }

    #[test]
    fn what_is_too_small_to_see_is_gathered_rather_than_dropped() {
        // The picture must add up to the folder. Dropping the tail would make
        // the box lie by however much it dropped.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), Some(4096));
        tree.push(ROOT, node("huge.bin", ROOT, Kind::File, 1_000_000));
        for n in 0..500 {
            tree.push(ROOT, node(&format!("tiny-{n}.bin"), ROOT, Kind::File, 100));
        }
        tree.roll_up();

        let tiles = tiles(&tree, ROOT, Layout::default());
        assert!(tiles.len() < 20, "500 invisible files became {} tiles", tiles.len());

        let last = tiles.last().expect("a remainder tile");
        assert_eq!(last.id, None, "the remainder stands for no single node");
        assert_eq!(last.entries, 500);
        assert!(last.name.contains("500"), "the remainder says how many: {}", last.name);

        // And the areas still add up.
        let rects: Vec<Rect> = tiles.iter().map(|tile| tile.rect).collect();
        assert!((covered(&rects) - 1.0).abs() < 1e-9);
        assert!(!overlaps(&rects));
    }

    #[test]
    fn a_folder_of_nothing_but_small_things_is_one_tile() {
        // Every child under the cut-off: the answer is one rectangle, not a
        // hundred thousand, and not an empty box.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        for n in 0..2_000 {
            tree.push(ROOT, node(&format!("f-{n}.bin"), ROOT, Kind::File, 1_000));
        }
        tree.roll_up();

        let tiles = tiles(
            &tree,
            ROOT,
            Layout {
                min_area: 0.01,
                ..Layout::default()
            },
        );
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].id, None);
        assert_eq!(tiles[0].entries, 2_000);
    }

    #[test]
    fn the_tile_count_is_capped_however_many_clear_the_cut_off() {
        // Ten thousand children each worth a tenth of a percent clear a small
        // `min_area` on a big screen. A picture of ten thousand rectangles is a
        // grey rectangle that takes a second to draw.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        for n in 0..3_000 {
            tree.push(ROOT, node(&format!("f-{n}.bin"), ROOT, Kind::File, 1_000));
        }
        tree.roll_up();

        let tiles = tiles(
            &tree,
            ROOT,
            Layout {
                min_area: 0.0,
                max_tiles: 40,
                ..Layout::default()
            },
        );
        assert!(tiles.len() <= 41, "capped at 40 plus the remainder, got {}", tiles.len());
        assert_eq!(tiles.last().expect("a remainder").id, None);

        let rects: Vec<Rect> = tiles.iter().map(|tile| tile.rect).collect();
        assert!((covered(&rects) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_zero_sized_child_takes_no_space_at_all() {
        // An empty folder has no area, and a tile of no area is an invisible
        // click target that steals hovers from its neighbour.
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        tree.push(ROOT, node("real.bin", ROOT, Kind::File, 500));
        tree.push(ROOT, node("empty", ROOT, Kind::Directory, 0));
        tree.roll_up();

        let tiles = tiles(&tree, ROOT, Layout::default());
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].name, "real.bin");
    }

    #[test]
    fn an_empty_folder_draws_nothing() {
        let tree = Tree::new(PathBuf::from("C:/scan"), None);
        assert!(tiles(&tree, ROOT, Layout::default()).is_empty());
    }

    #[test]
    fn build_output_is_coloured_apart_from_what_was_typed() {
        assert_eq!(Category::of("node_modules", Kind::Directory, None), Category::Build);
        assert_eq!(Category::of("target", Kind::Directory, None), Category::Build);
        assert_eq!(Category::of("main.rs", Kind::File, None), Category::Source);
        assert_eq!(Category::of("holiday.mp4", Kind::File, None), Category::Media);
        assert_eq!(Category::of("something.qqq", Kind::File, None), Category::Other);
        assert_eq!(Category::of("LICENSE", Kind::File, None), Category::Other);
    }

    #[test]
    fn a_name_is_matched_whatever_its_case() {
        // `Debug` and `debug`, `Release` and `release` — MSBuild writes both.
        assert_eq!(Category::of("NODE_MODULES", Kind::Directory, None), Category::Build);
        assert_eq!(Category::of("MAIN.RS", Kind::File, None), Category::Source);
    }

    #[test]
    fn everything_inside_build_output_is_build_output() {
        // The rule that makes the colour honest: `node_modules/react/src` is
        // not source someone typed, it is something `npm install` will write
        // again. Without inheritance the inside of the biggest reclaimable
        // directory on the disk would be coloured as the thing you must keep.
        assert_eq!(Category::of("src", Kind::Directory, Some(Category::Build)), Category::Build);
        assert_eq!(Category::of("index.ts", Kind::File, Some(Category::Build)), Category::Build);
    }

    #[test]
    fn the_inheritance_is_read_from_the_tree_not_guessed() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        let modules = tree.push(ROOT, node("node_modules", ROOT, Kind::Directory, 0));
        let package = tree.push(modules, node("react", modules, Kind::Directory, 0));
        tree.push(package, node("index.js", package, Kind::File, 1_000));
        tree.roll_up();

        // Drawn from inside `node_modules/react`: a JavaScript file that would
        // otherwise read as source.
        let tiles = tiles(&tree, package, Layout::default());
        assert_eq!(tiles[0].name, "index.js");
        assert_eq!(tiles[0].category, Category::Build);
    }

    #[test]
    fn a_tile_carries_the_path_its_tooltip_shows() {
        // Built from the scan root and the names on the way down, so the
        // separator is the platform's own.
        let tree = sample();
        let tiles = tiles(&tree, ROOT, Layout::default());
        assert_eq!(tiles[0].path, PathBuf::from("C:/scan").join("big").to_string_lossy());
        assert_eq!(tiles[2].path, PathBuf::from("C:/scan").join("small.txt").to_string_lossy());
    }

    #[test]
    fn the_remainder_has_no_path_because_it_is_many() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        tree.push(ROOT, node("huge.bin", ROOT, Kind::File, 1_000_000));
        for n in 0..200 {
            tree.push(ROOT, node(&format!("t-{n}.bin"), ROOT, Kind::File, 10));
        }
        tree.roll_up();

        let tiles = tiles(&tree, ROOT, Layout::default());
        let last = tiles.last().expect("a remainder");
        assert_eq!(last.id, None);
        assert!(last.path.is_empty(), "a tile that stands for many paths has none");
    }

    #[test]
    fn a_directory_tile_says_it_can_be_entered() {
        let tree = sample();
        let tiles = tiles(&tree, ROOT, Layout::default());
        assert!(tiles[0].is_directory, "big is a folder");
        assert!(!tiles[2].is_directory, "small.txt is not");
        assert_eq!(tiles[0].id, Some(tree.root().children[0]));
    }

    #[test]
    fn the_cap_cannot_be_raised_past_what_a_picture_can_carry() {
        let mut tree = Tree::new(PathBuf::from("C:/scan"), None);
        for n in 0..3_000 {
            tree.push(ROOT, node(&format!("f-{n}.bin"), ROOT, Kind::File, 1_000));
        }
        tree.roll_up();

        let tiles = tiles(
            &tree,
            ROOT,
            Layout {
                min_area: 0.0,
                max_tiles: usize::MAX,
                ..Layout::default()
            },
        );
        assert!(tiles.len() <= MAX_TILES + 1, "asked for everything, got {}", tiles.len());
    }

    #[test]
    fn a_remainder_of_one_entry_says_so_in_the_singular() {
        assert_eq!(remainder_name(1), "1 smaller entry");
        assert_eq!(remainder_name(2), "2 smaller entries");
    }
}
