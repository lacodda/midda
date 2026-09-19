//! The scan as the window sees it: start it, watch it, read it.
//!
//! A scan of a system volume runs for minutes, so it cannot happen inside a
//! command — Tauri would hold the call open and the window would be a frozen
//! rectangle. It runs on a thread of its own and leaves its state here; the
//! window polls.
//!
//! Polling rather than events, deliberately. The counters are a number the
//! window repaints a few times a second, and an event per directory would push
//! several hundred thousand messages through the bridge to draw the same digit.
//! When the tree itself starts streaming in v0.6.0 that changes; a progress
//! counter does not need it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use midda_core::order::{self, Sort, Span};
use midda_core::treemap::{self, Layout, Tile};
use midda_core::{Progress, Tree};
use serde::{Deserialize, Serialize};

/// What the window is looking at, if anything.
///
/// The state is behind an `Arc` rather than owned outright because the scanning
/// thread outlives the command call that started it: Tauri hands a command a
/// borrow of the managed state, and a thread cannot hold one.
#[derive(Default)]
pub struct Session {
    shared: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    /// The counters of the scan now running, if one is.
    progress: Option<Arc<Progress>>,
    /// The last scan that finished.
    tree: Option<Tree>,
    /// Set when a scan ended badly, so the window can say why rather than
    /// showing an empty result that looks like an empty disk.
    failure: Option<String>,
    /// Whether a scan is running right now.
    running: bool,
}

/// One row as the table shows it.
///
/// Both sizes travel, always. The window defaults to `allocated` and offers the
/// other; sending only one would mean a round trip to the core to switch, and
/// the switch is a click.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: u32,
    pub name: String,
    /// The bytes the volume gives up if this goes away. What midda sorts and
    /// draws by.
    pub allocated: u64,
    /// The bytes a reader would get out of it.
    pub logical: u64,
    pub is_directory: bool,
    /// How many entries are in this subtree, this one included.
    pub entries: u64,
    /// The newest modification time anywhere beneath this, as milliseconds
    /// since the Unix epoch. `null` when the filesystem would not say.
    pub subtree_modified: Option<i64>,
    /// The full path, for the tooltip and for the reveal-in-Explorer of v0.2.0.
    pub path: String,
}

/// One page of an ordered list of rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub rows: Vec<Row>,
    /// Where these rows start in the whole ordered list, so the window can put
    /// them at the right height without counting.
    pub offset: u32,
    /// How many rows there are in total — what makes the scrollbar honest about
    /// a folder whose rows are not all here.
    pub total: u32,
}

/// What the window draws while a scan runs, and when it has finished.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanState {
    pub running: bool,
    /// Entries seen so far.
    pub entries: u64,
    /// Bytes on disk counted so far.
    pub bytes: u64,
    /// The finished scan, when there is one.
    pub result: Option<ScanResult>,
    /// Why the last scan ended badly, when it did.
    pub error: Option<String>,
}

/// A finished scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub root: Row,
    /// The cluster size of the volume, when it is known: what explains why a
    /// thousand tiny files cost more than they read.
    pub cluster_bytes: Option<u64>,
    /// How many entries the scan could not read. A system volume always has
    /// some, and a total that hid them would be quietly wrong.
    pub skipped: u64,
}

/// Starts a scan of `path`.
///
/// Returns as soon as the thread is running: the window shows the progress and
/// polls [`scan_progress`].
///
/// # Errors
///
/// When a scan is already running.
#[tauri::command]
pub fn start_scan(path: String, session: tauri::State<'_, Session>) -> Result<(), String> {
    let mut state = session.shared.lock().map_err(|_| "the scan session is poisoned".to_owned())?;
    if state.running {
        return Err("a scan is already running".to_owned());
    }

    let progress = Progress::new();
    state.progress = Some(Arc::clone(&progress));
    state.tree = None;
    state.failure = None;
    state.running = true;
    drop(state);

    let root = PathBuf::from(path);
    let session = Arc::clone(&session.shared);

    std::thread::spawn(move || {
        let outcome = midda_core::scan(&root, &progress);
        if let Ok(mut state) = session.lock() {
            match outcome {
                Ok(tree) => state.tree = Some(tree),
                Err(error) => state.failure = Some(error.to_string()),
            }
            state.running = false;
            state.progress = None;
        }
    });

    Ok(())
}

/// How the scan is going, and its result once it has one.
///
/// # Errors
///
/// When the session lock is poisoned.
#[tauri::command]
pub fn scan_progress(session: tauri::State<'_, Session>) -> Result<ScanState, String> {
    let state = session.shared.lock().map_err(|_| "the scan session is poisoned".to_owned())?;

    let (entries, bytes) = state.progress.as_ref().map_or_else(
        || {
            // A finished scan reports its own totals: the counters are gone, and
            // a window that showed zeros the moment a scan ended would flicker.
            state.tree.as_ref().map_or((0, 0), |tree| (tree.root().entries, tree.root().size.allocated))
        },
        |progress| (progress.entries(), progress.bytes()),
    );

    Ok(ScanState {
        running: state.running,
        entries,
        bytes,
        result: state.tree.as_ref().map(|tree| ScanResult {
            root: row(tree, midda_core::ROOT),
            cluster_bytes: tree.cluster_bytes(),
            skipped: tree.skipped().len() as u64,
        }),
        error: state.failure.clone(),
    })
}

/// Asks the running scan to stop.
///
/// # Errors
///
/// When the session lock is poisoned.
#[tauri::command]
pub fn cancel_scan(session: tauri::State<'_, Session>) -> Result<(), String> {
    let state = session.shared.lock().map_err(|_| "the scan session is poisoned".to_owned())?;
    if let Some(progress) = state.progress.as_ref() {
        progress.cancel();
    }
    Ok(())
}

/// One page of the children of `id`, ordered by `sort`.
///
/// Paged rather than whole: a folder on a system volume can hold hundreds of
/// thousands of children, and a table that received all of them on every click
/// of a column header would send megabytes across the bridge to redraw thirty
/// rows. The window asks for the span it is drawing.
///
/// # Errors
///
/// When there is no finished scan, or `id` is not in it.
#[tauri::command]
pub fn list_children(id: u32, sort: Sort, span: Span, session: tauri::State<'_, Session>) -> Result<Page, String> {
    let state = session.shared.lock().map_err(|_| "the scan session is poisoned".to_owned())?;
    let tree = state.tree.as_ref().ok_or_else(|| "nothing has been scanned yet".to_owned())?;

    if id as usize >= tree.len() {
        return Err(format!("no entry {id} in this scan"));
    }

    let (ids, total) = order::children_page(tree, id, sort, span);
    Ok(Page {
        rows: ids.into_iter().map(|child| row(tree, child)).collect(),
        offset: span.offset,
        total: u32::try_from(total).unwrap_or(u32::MAX),
    })
}

/// The trail from the scan root down to `id`, root first.
///
/// Asked of the core rather than remembered by the window: the window's own
/// copy is right only as long as the reader arrived by clicking, and from
/// v0.3.0 a click in the treemap arrives at a node the window has never drawn.
///
/// # Errors
///
/// When there is no finished scan, or `id` is not in it.
#[tauri::command]
pub fn trail_to(id: u32, session: tauri::State<'_, Session>) -> Result<Vec<Row>, String> {
    let state = session.shared.lock().map_err(|_| "the scan session is poisoned".to_owned())?;
    let tree = state.tree.as_ref().ok_or_else(|| "nothing has been scanned yet".to_owned())?;

    if id as usize >= tree.len() {
        return Err(format!("no entry {id} in this scan"));
    }

    let mut trail = Vec::new();
    let mut at = id;
    loop {
        trail.push(row(tree, at));
        if at == midda_core::ROOT {
            break;
        }
        at = tree.node(at).parent;
    }
    trail.reverse();
    Ok(trail)
}

/// The rectangles that draw the children of `id`.
///
/// The layout is the core's, not the window's: the areas, the order, the
/// cut-off for what is too small to see, and the tile that gathers what fell
/// under it. The window passes its own shape and its own idea of the smallest
/// useful rectangle, and multiplies the fractions it gets back by its size.
///
/// # Errors
///
/// When there is no finished scan, or `id` is not in it.
#[tauri::command]
pub fn treemap(id: u32, layout: Layout, session: tauri::State<'_, Session>) -> Result<Vec<Tile>, String> {
    let state = session.shared.lock().map_err(|_| "the scan session is poisoned".to_owned())?;
    let tree = state.tree.as_ref().ok_or_else(|| "nothing has been scanned yet".to_owned())?;

    if id as usize >= tree.len() {
        return Err(format!("no entry {id} in this scan"));
    }

    Ok(treemap::tiles(tree, id, layout))
}

/// Turns a node into a row.
fn row(tree: &Tree, id: u32) -> Row {
    let node = tree.node(id);
    Row {
        id,
        name: node.name.clone(),
        allocated: node.size.allocated,
        logical: node.size.logical,
        is_directory: node.is_directory(),
        entries: node.entries,
        subtree_modified: node.subtree_modified.and_then(millis_since_epoch),
        path: tree.path_of(id).to_string_lossy().into_owned(),
    }
}

/// A timestamp as JavaScript wants it: milliseconds since the Unix epoch.
///
/// A time before 1970 is possible on a filesystem and JavaScript handles a
/// negative one fine, so it is not discarded — only a value too large for an
/// `i64` of milliseconds is, and that is a clock, not a file.
fn millis_since_epoch(time: std::time::SystemTime) -> Option<i64> {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis()).ok().map(|millis| -millis),
    }
}
