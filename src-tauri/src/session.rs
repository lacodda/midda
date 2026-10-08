//! The folder the window has open: reading it, keeping it current, and the
//! rows and rectangles drawn from it.
//!
//! A folder is opened once and then kept, by a thread of its own — the keeper.
//! It puts the index saved last time on screen first, if there is one; brings
//! it up to date from the change journal when this process may read it, or
//! reads the folder again when it may not; and then follows the journal or a
//! watcher for as long as the window is open, applying what changes as it
//! changes. The window polls. Every answer it is given carries two numbers:
//! the `version` of what is on screen, which moves with every change, and the
//! `arena` the ids belong to, which moves only when the tree was read again
//! and every id handed out before means something else now (ADR 0009).
//!
//! Polling rather than events, deliberately: the window repaints a counter a
//! few times a second, and an event per directory read would push hundreds of
//! thousands of messages through the bridge to draw the same digit.
//!
//! Beside the folder the window can hold two other trees, each asked for and
//! then kept as of the asking: what the change journal says was written
//! lately, and a comparison of two pictures of the folder — a snapshot and
//! now, or two snapshots (ADR 0010). Opening a folder keeps the index saved
//! last time as the picture of last time before anything brings it up to
//! date.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use midda_core::compare::{Comparison, Mark, PlaceKind};
use midda_core::fresh::since::{Change, Changes};
use midda_core::fresh::store::{self, Unusable};
use midda_core::order::{self, Sort, Span};
use midda_core::snapshot::{self, Kind, Snapshot};
use midda_core::treemap::{self, Layout, Tile};
use midda_core::{Index, JournalPosition, NodeId, Progress, ROOT, Size, SizeBasis, Tree, compare};
use serde::{Deserialize, Serialize};

/// How often the keeper asks the watcher or the journal what changed.
const TICK: Duration = Duration::from_millis(500);

/// How long changes may wait in memory before the index is saved anyway: a
/// crash loses at most this much of the picture.
const SAVE_EVERY: Duration = Duration::from_secs(5 * 60);

/// What the window has open, and the thread keeping it.
pub struct Session {
    shared: Arc<RwLock<State>>,
    keeper: Mutex<Option<Keeper>>,
    /// Where indexes are saved between runs.
    store: PathBuf,
    /// Where snapshots are kept, a folder per scanned folder.
    snapshots: PathBuf,
}

impl Session {
    /// A session keeping what it saves under `data`: indexes in `index`,
    /// snapshots in `snapshots`.
    #[must_use]
    pub fn new(data: &Path) -> Self {
        Self {
            shared: Arc::new(RwLock::new(State::default())),
            keeper: Mutex::new(None),
            store: data.join("index"),
            snapshots: data.join("snapshots"),
        }
    }

    /// Where the snapshots of the open folder are kept.
    fn snapshot_folder(&self) -> Result<(PathBuf, PathBuf), String> {
        let root = self.shared.read().map_err(poisoned)?.root.clone().ok_or_else(|| "nothing is open".to_owned())?;
        Ok((snapshot::folder_for(&self.snapshots, &root), root))
    }

    /// Stops keeping the open folder, saving its index first. Called when
    /// the window closes, and before another folder is opened.
    pub fn close(&self) {
        let keeper = self.keeper.lock().ok().and_then(|mut keeper| keeper.take());
        if let Some(keeper) = keeper {
            keeper.stop.store(true, Ordering::Relaxed);
            if let Ok(state) = self.shared.read()
                && let Some(reading) = &state.reading
            {
                reading.progress.cancel();
            }
            let _ = keeper.thread.join();
        }
    }
}

/// The keeper thread, and the two things the window can ask of it.
struct Keeper {
    stop: Arc<AtomicBool>,
    reread: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

#[derive(Default)]
struct State {
    /// The folder open.
    root: Option<PathBuf>,
    /// The index on screen, when there is one.
    index: Option<Index>,
    /// A full read running now: the first, or one under an index on screen.
    reading: Option<Reading>,
    /// Why the folder could not be read at all.
    failure: Option<String>,
    freshness: Freshness,
    /// How long the last full read took.
    took: Option<Duration>,
    /// When a change was last applied.
    applied_at: Option<SystemTime>,
    /// Moves with every change to what is on screen.
    version: u64,
    /// Moves when the index is replaced and its ids renumbered.
    arena: u64,
    /// What was written since a moment, once asked.
    changes: Option<Changes>,
    /// Moves when the answer above is replaced.
    changes_arena: u64,
    /// Two pictures of the folder compared, once asked.
    comparison: Option<Compared>,
    /// Moves when the comparison above is replaced.
    comparison_arena: u64,
    /// Why the picture of last time was not kept when the folder was opened,
    /// when it was not.
    last_note: Option<String>,
}

/// A comparison, and the two moments it is between.
struct Compared {
    comparison: Comparison,
    then: SystemTime,
    now: SystemTime,
}

struct Reading {
    progress: Arc<Progress>,
}

/// How the index on screen is kept current.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(not(windows), allow(dead_code, reason = "the journal and the watcher are Windows-only"))]
pub enum Mode {
    /// Nothing is open.
    #[default]
    None,
    /// The volume's change journal: every change, and those made while midda
    /// was closed.
    Journal,
    /// A watcher on the folder: what changes while the window is open.
    Watching,
    /// Nothing: the picture is as it was read.
    Still,
}

/// What the window says about how current its picture is.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Freshness {
    pub mode: Mode,
    /// The index on screen is the one saved last time, from this moment, and
    /// has not been brought up to date yet.
    pub saved_at: Option<i64>,
    /// The journal is being read from where the saved index left off.
    pub catching_up: bool,
    /// Why the picture is as current as it is, when that needs saying: what
    /// stopped the watcher, why the saved index was read again.
    pub note: Option<String>,
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
    /// The full path, for the tooltip and for finding the row again after the
    /// tree was read anew.
    pub path: String,
    /// Why the two sizes differ, when they do, as the bits of
    /// `midda_core::Traits`.
    ///
    /// A number rather than a list of names: it is read by one function on the
    /// other side, and a row of a table that carries four strings it will not
    /// print is four allocations per row times a hundred thousand rows.
    pub traits: u8,
    /// How many names these bytes have on the volume, when the platform said.
    pub links: Option<u32>,
    /// In the view of what changed: whether this file is new or was written
    /// to. `null` everywhere else.
    pub change: Option<Change>,
    /// In a comparison: what this held then and how much moved under it.
    /// `null` everywhere else.
    pub compared: Option<ComparedRow>,
}

/// What a row of a comparison carries beyond a row: the other moment.
///
/// Both sizes, as a row carries both: the switch between them is a click and
/// should not be a round trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparedRow {
    /// What it held then. What it holds now is the row's own size.
    pub before: Size,
    /// How many bytes moved under it: grown and freed, added.
    pub moved: Size,
    pub mark: Mark,
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

/// The way from the root down to one entry, and the ids it is given in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trail {
    pub arena: u64,
    pub rows: Vec<Row>,
}

/// Which tree a question is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum View {
    /// The folder as it is.
    Index,
    /// What was written since a moment.
    Changes,
    /// Two pictures of the folder compared.
    Compare,
}

/// What the window draws: how the read is going, the result, and how
/// current it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// A full read is running — the first, or one under the picture on screen.
    pub reading: bool,
    /// Entries seen so far by the running read.
    pub entries: u64,
    /// Bytes on disk counted so far by the running read.
    pub bytes: u64,
    /// MFT records read so far: the one number that moves while the fast
    /// scanner reads the table, before any entry is placed in a tree.
    pub records: u64,
    /// The picture on screen, when there is one.
    pub result: Option<ScanResult>,
    /// Why the folder could not be read, when it could not.
    pub error: Option<String>,
    pub freshness: Freshness,
    /// When a change was last applied, as milliseconds since the epoch.
    pub applied_at: Option<i64>,
    pub version: u64,
    pub arena: u64,
}

/// A picture on screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub root: Row,
    /// The cluster size of the volume, when it is known: what explains why a
    /// thousand tiny files cost more than they read.
    pub cluster_bytes: Option<u64>,
    /// How many entries the read could not open.
    pub skipped: u64,
    /// How many entries turned out to be second names for bytes counted
    /// elsewhere.
    pub shared_names: u64,
    /// The bytes those second names would have been counted as, and are not.
    pub shared_reclaimed: u64,
    /// Which scanner read this: `walk` or `mft`.
    pub scanned_by: String,
    /// Why the MFT reader gave way to the walk, when it did.
    pub fallback: Option<String>,
    /// How long the last full read took, in milliseconds. Zero for an index
    /// read back from disk and not read again yet.
    pub elapsed_ms: u64,
    /// When the folder was last read in full.
    pub read_at: Option<i64>,
}

/// What was written since a moment, as the window shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(not(windows), allow(dead_code, reason = "only the change journal, which is Windows-only, answers it"))]
#[serde(rename_all = "camelCase")]
pub struct ChangesSummary {
    pub root: Row,
    pub arena: u64,
    /// Files that appeared since and are still there.
    pub created: u64,
    /// Files that existed and were written to.
    pub written: u64,
    /// Files and folders that existed and are gone.
    pub deleted: u64,
    pub since: Option<i64>,
    /// The oldest change the journal holds.
    pub reaches_back_to: Option<i64>,
    /// Whether the journal covers the whole span asked about.
    pub covers: bool,
}

/// One snapshot of the open folder, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRow {
    /// Its file name: what the window asks for it by.
    pub name: String,
    pub kind: Kind,
    /// The moment it is a picture of, as milliseconds since the epoch.
    pub at: Option<i64>,
    /// What it takes on disk.
    pub bytes: u64,
    /// Why it cannot be compared, when it cannot.
    pub unusable: Option<String>,
}

impl From<Snapshot> for SnapshotRow {
    fn from(snapshot: Snapshot) -> Self {
        Self {
            name: snapshot.name,
            kind: snapshot.kind,
            at: millis(snapshot.at),
            bytes: snapshot.bytes,
            unusable: snapshot.unusable,
        }
    }
}

/// The snapshots of the open folder, and what they take together.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshots {
    pub snapshots: Vec<SnapshotRow>,
    pub bytes: u64,
    /// Why there is no picture of last time from this opening, when there is
    /// not one for a reason worth saying.
    pub last_note: Option<String>,
    /// Why a snapshot cannot be taken or compared with now, when it cannot:
    /// the picture on screen is not current yet.
    pub not_now: Option<String>,
}

/// A comparison, as the window shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonSummary {
    pub root: Row,
    pub arena: u64,
    /// The earlier moment, as milliseconds since the epoch.
    pub then: Option<i64>,
    /// The later one.
    pub now: Option<i64>,
    /// Whether the later picture is the folder as it is, rather than a
    /// snapshot.
    pub to_now: bool,
    pub totals: midda_core::Totals,
}

/// One place in the report of a comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceRow {
    /// The entry in the comparison's tree, for the window to go to.
    pub id: u32,
    pub path: String,
    pub is_directory: bool,
    pub kind: PlaceKind,
    pub freed: Size,
    pub grown: Size,
    pub entries: u64,
}

/// The places past the ones listed, added up.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rest {
    pub places: u64,
    pub bytes: u64,
}

/// The report of a comparison: where it came back from, and where it went.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub arena: u64,
    /// The places bytes went from, the most first.
    pub freed: Vec<PlaceRow>,
    pub freed_rest: Rest,
    /// The places bytes came to, the most first.
    pub grown: Vec<PlaceRow>,
    pub grown_rest: Rest,
}

fn poisoned<T>(_: T) -> String {
    "the session is poisoned".to_owned()
}

/// Opens `path`: puts its saved index on screen if there is one, and starts
/// keeping it current.
///
/// # Errors
///
/// When the session lock is poisoned.
#[tauri::command]
pub fn start_scan(path: String, session: tauri::State<'_, Session>) -> Result<(), String> {
    session.close();
    let root = PathBuf::from(path);
    {
        let mut state = session.shared.write().map_err(poisoned)?;
        let arena = state.arena + 1;
        let changes_arena = state.changes_arena + 1;
        let comparison_arena = state.comparison_arena + 1;
        *state = State {
            root: Some(root.clone()),
            arena,
            changes_arena,
            comparison_arena,
            ..State::default()
        };
    }

    let stop = Arc::new(AtomicBool::new(false));
    let reread = Arc::new(AtomicBool::new(false));
    let thread = {
        let shared = Arc::clone(&session.shared);
        let places = Places {
            store: session.store.clone(),
            snapshots: snapshot::folder_for(&session.snapshots, &root),
        };
        let stop = Arc::clone(&stop);
        let reread = Arc::clone(&reread);
        std::thread::Builder::new()
            .name("midda-keeper".into())
            .spawn(move || keep(&shared, &root, &places, &stop, &reread))
            .map_err(|error| format!("could not start reading: {error}"))?
    };
    *session.keeper.lock().map_err(poisoned)? = Some(Keeper { stop, reread, thread });
    Ok(())
}

/// Reads the open folder again in full, keeping the picture on screen until
/// the new one is ready.
///
/// # Errors
///
/// When nothing is open.
#[tauri::command]
pub fn rescan(session: tauri::State<'_, Session>) -> Result<(), String> {
    let keeper = session.keeper.lock().map_err(poisoned)?;
    let keeper = keeper.as_ref().ok_or_else(|| "nothing is open".to_owned())?;
    keeper.reread.store(true, Ordering::Relaxed);
    Ok(())
}

/// Asks the running read to stop.
///
/// # Errors
///
/// When the session lock is poisoned.
#[tauri::command]
pub fn cancel_scan(session: tauri::State<'_, Session>) -> Result<(), String> {
    let state = session.shared.read().map_err(poisoned)?;
    if let Some(reading) = &state.reading {
        reading.progress.cancel();
    }
    Ok(())
}

/// How the read is going, what is on screen, and how current it is.
///
/// # Errors
///
/// When the session lock is poisoned.
#[tauri::command]
pub fn scan_progress(session: tauri::State<'_, Session>) -> Result<SessionState, String> {
    let state = session.shared.read().map_err(poisoned)?;
    let (entries, bytes, records) = state.reading.as_ref().map_or_else(
        || {
            // Nothing running: the picture's own totals, so a window does not
            // flicker to zero the moment a read ends.
            state
                .index
                .as_ref()
                .map_or((0, 0, 0), |index| (index.tree().root().entries, index.tree().root().size.allocated, 0))
        },
        |reading| (reading.progress.entries(), reading.progress.bytes(), reading.progress.records()),
    );
    Ok(SessionState {
        reading: state.reading.is_some(),
        entries,
        bytes,
        records,
        result: state.index.as_ref().map(|index| {
            let tree = index.tree();
            ScanResult {
                root: row(Shown::Index(tree), ROOT),
                cluster_bytes: tree.cluster_bytes(),
                skipped: tree.skipped().len() as u64,
                shared_names: tree.shared().shared_names,
                shared_reclaimed: tree.shared().reclaimed.allocated,
                scanned_by: tree.scanned_by().to_owned(),
                fallback: tree.fallback().map(str::to_owned),
                elapsed_ms: state.took.map_or(0, |took| u64::try_from(took.as_millis()).unwrap_or(u64::MAX)),
                read_at: millis(index.scanned_at()),
            }
        }),
        error: state.failure.clone(),
        freshness: state.freshness.clone(),
        applied_at: state.applied_at.and_then(millis),
        version: state.version,
        arena: state.arena,
    })
}

/// The tree a view is about, with whatever its rows carry beyond the tree.
#[derive(Clone, Copy)]
enum Shown<'a> {
    Index(&'a Tree),
    Changes(&'a Changes),
    Compare(&'a Comparison),
}

impl<'a> Shown<'a> {
    const fn tree(self) -> &'a Tree {
        match self {
            Self::Index(tree) => tree,
            Self::Changes(changes) => &changes.tree,
            Self::Compare(comparison) => comparison.tree(),
        }
    }
}

/// The arena the ids of a view are in now.
const fn arena_of(state: &State, view: View) -> u64 {
    match view {
        View::Index => state.arena,
        View::Changes => state.changes_arena,
        View::Compare => state.comparison_arena,
    }
}

/// The tree a view is about, checked against the arena the window's ids are
/// from.
fn tree_of(state: &State, view: View, arena: Option<u64>) -> Result<Shown<'_>, String> {
    let shown = match view {
        View::Index => state.index.as_ref().map(|index| Shown::Index(index.tree())),
        View::Changes => state.changes.as_ref().map(Shown::Changes),
        View::Compare => state.comparison.as_ref().map(|compared| Shown::Compare(&compared.comparison)),
    };
    // The window's ids are from a tree read before this one: they name other
    // entries now. It finds its place again by path.
    if arena.is_some_and(|arena| arena != arena_of(state, view)) {
        return Err("stale".to_owned());
    }
    shown.ok_or_else(|| "nothing has been read yet".to_owned())
}

fn checked(tree: &Tree, id: u32) -> Result<NodeId, String> {
    if tree.contains(id) {
        Ok(id)
    } else {
        // Deleted while the window looked at it.
        Err("stale".to_owned())
    }
}

/// One page of the children of `id`, ordered by `sort`. Growth, in a
/// comparison, is read on `basis`.
///
/// Paged rather than whole: a folder on a system volume can hold hundreds of
/// thousands of children, and a table that received all of them on every click
/// of a column header would send megabytes across the bridge to redraw thirty
/// rows.
///
/// # Errors
///
/// `stale` when the ids are from a tree read before this one or `id` is gone;
/// otherwise when nothing has been read.
#[tauri::command]
pub fn list_children(view: View, arena: u64, id: u32, sort: Sort, basis: SizeBasis, span: Span, session: tauri::State<'_, Session>) -> Result<Page, String> {
    let state = session.shared.read().map_err(poisoned)?;
    let shown = tree_of(&state, view, Some(arena))?;
    let tree = shown.tree();
    let id = checked(tree, id)?;
    let (ids, total) = match shown {
        Shown::Compare(comparison) => comparison.children_page(id, sort, basis, span),
        Shown::Index(_) | Shown::Changes(_) => order::children_page(tree, id, sort, span),
    };
    Ok(Page {
        rows: ids.into_iter().map(|child| row(shown, child)).collect(),
        offset: span.offset,
        total: u32::try_from(total).unwrap_or(u32::MAX),
    })
}

/// The trail from the root down to `id`, root first.
///
/// # Errors
///
/// As [`list_children`].
#[tauri::command]
pub fn trail_to(view: View, arena: u64, id: u32, session: tauri::State<'_, Session>) -> Result<Vec<Row>, String> {
    let state = session.shared.read().map_err(poisoned)?;
    let shown = tree_of(&state, view, Some(arena))?;
    let id = checked(shown.tree(), id)?;
    Ok(trail(shown, id))
}

/// The trail down to the entry at `path`, or to the deepest folder above it
/// that is still there — and the arena its ids are in.
///
/// How the window finds its place again after the tree changed under it: the
/// folder it was in was deleted, or the whole tree was read again and every
/// id renumbered.
///
/// # Errors
///
/// When nothing has been read.
#[tauri::command]
pub fn trail_to_path(view: View, path: String, session: tauri::State<'_, Session>) -> Result<Trail, String> {
    let state = session.shared.read().map_err(poisoned)?;
    let shown = tree_of(&state, view, None)?;
    let tree = shown.tree();
    let arena = arena_of(&state, view);
    let mut at = ROOT;
    if let Ok(rest) = Path::new(&path).strip_prefix(tree.root_path()) {
        for part in rest.components() {
            match tree.child(at, &part.as_os_str().to_string_lossy()) {
                Some(child) => at = child,
                None => break,
            }
        }
    }
    Ok(Trail { arena, rows: trail(shown, at) })
}

/// The rectangles that draw the children of `id`.
///
/// # Errors
///
/// As [`list_children`].
#[tauri::command]
pub fn treemap(view: View, arena: u64, id: u32, layout: Layout, session: tauri::State<'_, Session>) -> Result<Vec<Tile>, String> {
    let state = session.shared.read().map_err(poisoned)?;
    let shown = tree_of(&state, view, Some(arena))?;
    let id = checked(shown.tree(), id)?;
    Ok(match shown {
        // Drawn by what moved, not by what is there.
        Shown::Compare(comparison) => comparison.tiles(id, layout),
        Shown::Index(tree) => treemap::tiles(tree, id, layout),
        Shown::Changes(changes) => treemap::tiles(&changes.tree, id, layout),
    })
}

/// Works out what was written in the last `hours`, from the change journal.
///
/// # Errors
///
/// When nothing is open, or the journal cannot be read — which, without
/// elevation, it cannot.
#[tauri::command]
pub async fn changes_since(hours: u32, session: tauri::State<'_, Session>) -> Result<ChangesSummary, String> {
    let shared = Arc::clone(&session.shared);
    tauri::async_runtime::spawn_blocking(move || written_since(&shared, hours))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(windows)]
fn written_since(shared: &RwLock<State>, hours: u32) -> Result<ChangesSummary, String> {
    use midda_core::fresh::journal::Journal;

    let root = shared.read().map_err(poisoned)?.root.clone().ok_or_else(|| "nothing is open".to_owned())?;
    let mut journal = Journal::open(&root).map_err(|error| error.to_string())?;
    let records = journal.read_from(journal.oldest()).map_err(|error| error.to_string())?;
    let moment = SystemTime::now() - Duration::from_secs(u64::from(hours) * 3600);

    let mut state = shared.write().map_err(poisoned)?;
    let index = state.index.as_ref().ok_or_else(|| "nothing has been read yet".to_owned())?;
    let changes = midda_core::fresh::since::written_since(index, &records, moment, journal.place());
    state.changes_arena += 1;
    let summary = ChangesSummary {
        root: row(Shown::Changes(&changes), ROOT),
        arena: state.changes_arena,
        created: changes.created,
        written: changes.written,
        deleted: changes.deleted,
        since: millis(changes.since),
        reaches_back_to: changes.reaches_back_to.and_then(millis),
        covers: changes.covers(),
    };
    state.changes = Some(changes);
    Ok(summary)
}

#[cfg(not(windows))]
fn written_since(_shared: &RwLock<State>, _hours: u32) -> Result<ChangesSummary, String> {
    Err("the change journal is an NTFS feature".to_owned())
}

/// The snapshots of the open folder.
///
/// # Errors
///
/// When nothing is open.
#[tauri::command]
pub fn snapshots(session: tauri::State<'_, Session>) -> Result<Snapshots, String> {
    let (folder, root) = session.snapshot_folder()?;
    let listed = snapshot::list(&folder, &root);
    let state = session.shared.read().map_err(poisoned)?;
    Ok(Snapshots {
        bytes: listed.iter().map(|snapshot| snapshot.bytes).sum(),
        snapshots: listed.into_iter().map(SnapshotRow::from).collect(),
        last_note: state.last_note.clone(),
        not_now: not_current(&state).err(),
    })
}

/// Whether the picture on screen is the folder as it is now — as far as
/// anything is keeping it so — rather than the one saved last time and not
/// yet brought up to date. A snapshot of the second, dated now, would be a
/// picture of last time under today's date.
fn not_current(state: &State) -> Result<(), String> {
    if state.index.is_none() {
        return Err("nothing has been read yet".to_owned());
    }
    if state.freshness.saved_at.is_some() || state.freshness.catching_up {
        return Err("the picture on screen is the one saved last time, and is still being brought up to date".to_owned());
    }
    Ok(())
}

/// Takes a snapshot of the folder as it is now.
///
/// # Errors
///
/// When nothing is open, the picture is not current yet, or the snapshot
/// cannot be written.
#[tauri::command]
pub async fn take_snapshot(session: tauri::State<'_, Session>) -> Result<SnapshotRow, String> {
    let (folder, _) = session.snapshot_folder()?;
    let at = SystemTime::now();
    // Written into memory under the lock, put on disk without it: the same
    // split as the index's own save.
    let encoded = {
        let state = session.shared.read().map_err(poisoned)?;
        not_current(&state)?;
        let index = state.index.as_ref().ok_or_else(|| "nothing has been read yet".to_owned())?;
        store::encode(index, at)
    };
    tauri::async_runtime::spawn_blocking(move || snapshot::keep(&encoded, &folder, at))
        .await
        .map_err(|error| error.to_string())?
        .map(SnapshotRow::from)
        .map_err(|error| format!("the snapshot could not be written: {error}"))
}

/// Deletes the snapshot called `name`.
///
/// # Errors
///
/// When nothing is open, or the snapshot cannot be deleted.
#[tauri::command]
pub fn delete_snapshot(name: String, session: tauri::State<'_, Session>) -> Result<(), String> {
    let (folder, _) = session.snapshot_folder()?;
    snapshot::remove(&folder, &name).map_err(|error| format!("the snapshot could not be deleted: {error}"))
}

/// Compares the snapshot called `from` with the snapshot called `to`, or
/// with the folder as it is now when `to` is `None` — whichever of the two
/// is older is "then".
///
/// # Errors
///
/// When nothing is open, a snapshot cannot be read or is of another folder,
/// or the picture is not current yet and `to` is now.
#[tauri::command]
pub async fn compare_snapshots(from: String, to: Option<String>, session: tauri::State<'_, Session>) -> Result<ComparisonSummary, String> {
    let (folder, root) = session.snapshot_folder()?;
    let shared = Arc::clone(&session.shared);
    tauri::async_runtime::spawn_blocking(move || compare_in(&shared, &folder, &root, &from, to.as_deref()))
        .await
        .map_err(|error| error.to_string())?
}

fn compare_in(shared: &RwLock<State>, folder: &Path, root: &Path, from: &str, to: Option<&str>) -> Result<ComparisonSummary, String> {
    let read = |name: &str| -> Result<(Index, SystemTime), String> {
        let saved = snapshot::load(folder, name).map_err(|why| format!("the snapshot could not be read: {why}"))?;
        if !store::same_folder(saved.index.tree().root_path(), root) {
            return Err("the snapshot is a picture of another folder".to_owned());
        }
        Ok((saved.index, saved.saved_at))
    };
    // Read off the disk before any lock is taken: the older picture of a
    // large folder takes a second or two.
    let (first, first_at) = read(from)?;
    let compared = match to {
        Some(name) => {
            let (second, second_at) = read(name)?;
            let (then, now) = if first_at <= second_at {
                ((&first, first_at), (&second, second_at))
            } else {
                ((&second, second_at), (&first, first_at))
            };
            Compared {
                comparison: compare(then.0.tree(), now.0.tree()),
                then: then.1,
                now: now.1,
            }
        }
        None => {
            let state = shared.read().map_err(poisoned)?;
            not_current(&state)?;
            let index = state.index.as_ref().ok_or_else(|| "nothing has been read yet".to_owned())?;
            Compared {
                comparison: compare(first.tree(), index.tree()),
                then: first_at,
                now: SystemTime::now(),
            }
        }
    };
    drop(first);

    let mut state = shared.write().map_err(poisoned)?;
    state.comparison_arena += 1;
    let summary = ComparisonSummary {
        root: row(Shown::Compare(&compared.comparison), ROOT),
        arena: state.comparison_arena,
        then: millis(compared.then),
        now: millis(compared.now),
        to_now: to.is_none(),
        totals: compared.comparison.totals(),
    };
    state.comparison = Some(compared);
    Ok(summary)
}

/// The report of the comparison on screen: the places bytes went from and
/// came to, the most first on `basis`, `limit` of each and the rest added up.
///
/// # Errors
///
/// When no comparison has been made, or `arena` is not the one on screen.
#[tauri::command]
pub fn comparison_report(arena: u64, basis: SizeBasis, limit: u32, session: tauri::State<'_, Session>) -> Result<Report, String> {
    let state = session.shared.read().map_err(poisoned)?;
    let Shown::Compare(comparison) = tree_of(&state, View::Compare, Some(arena))? else {
        return Err("no comparison has been made".to_owned());
    };
    let places = comparison.places();
    let tree = comparison.tree();
    let side = |pick: fn(&midda_core::Place) -> Size| {
        let mut listed: Vec<_> = places.iter().filter(|place| basis.of(pick(place)) > 0).collect();
        listed.sort_by(|a, b| basis.of(pick(b)).cmp(&basis.of(pick(a))).then_with(|| a.id.cmp(&b.id)));
        let limit = limit as usize;
        let rest = Rest {
            places: listed.len().saturating_sub(limit) as u64,
            bytes: listed.iter().skip(limit).map(|place| basis.of(pick(place))).sum(),
        };
        let rows = listed
            .into_iter()
            .take(limit)
            .map(|place| PlaceRow {
                id: place.id,
                path: tree.path_of(place.id).to_string_lossy().into_owned(),
                is_directory: tree.node(place.id).is_directory(),
                kind: place.kind,
                freed: place.freed,
                grown: place.grown,
                entries: place.entries,
            })
            .collect();
        (rows, rest)
    };
    let (freed, freed_rest) = side(|place| place.freed);
    let (grown, grown_rest) = side(|place| place.grown);
    Ok(Report {
        arena,
        freed,
        freed_rest,
        grown,
        grown_rest,
    })
}

/// The trail from the root down to `id`, root first.
fn trail(shown: Shown<'_>, id: NodeId) -> Vec<Row> {
    let tree = shown.tree();
    let mut trail = Vec::new();
    let mut at = id;
    loop {
        trail.push(row(shown, at));
        if at == ROOT {
            break;
        }
        at = tree.node(at).parent;
    }
    trail.reverse();
    trail
}

/// Turns a node into a row.
fn row(shown: Shown<'_>, id: NodeId) -> Row {
    let tree = shown.tree();
    let node = tree.node(id);
    Row {
        id,
        name: node.name.clone(),
        allocated: node.size.allocated,
        logical: node.size.logical,
        is_directory: node.is_directory(),
        entries: node.entries,
        subtree_modified: node.subtree_modified.and_then(millis),
        path: tree.path_of(id).to_string_lossy().into_owned(),
        traits: node.traits.bits(),
        links: node.links,
        change: match shown {
            Shown::Changes(changes) => changes.mark(id),
            Shown::Index(_) | Shown::Compare(_) => None,
        },
        compared: match shown {
            Shown::Compare(comparison) => Some(ComparedRow {
                before: comparison.before(id),
                moved: comparison.moved(id),
                mark: comparison.mark(id),
            }),
            Shown::Index(_) | Shown::Changes(_) => None,
        },
    }
}

/// A time as JavaScript wants it: milliseconds since the Unix epoch.
///
/// A time before 1970 is possible on a filesystem and JavaScript handles a
/// negative one fine, so it is not discarded — only a value too large for an
/// `i64` of milliseconds is, and that is a clock, not a file.
fn millis(time: SystemTime) -> Option<i64> {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis()).ok().map(|millis| -millis),
    }
}

/// Where the news of changes comes from.
enum Source {
    #[cfg(windows)]
    Journal(midda_core::fresh::journal::Journal),
    #[cfg(windows)]
    Watcher(midda_core::fresh::watch::Watcher),
    /// Nothing: the reason is the note the window shows.
    Nothing(String),
}

impl Source {
    /// The journal when this process may read it; a watcher when it may not;
    /// nothing when neither can be had.
    fn open(root: &Path) -> Self {
        #[cfg(windows)]
        {
            use midda_core::fresh::journal::Journal;
            use midda_core::fresh::watch::Watcher;

            if let Ok(journal) = Journal::open(root) {
                return Self::Journal(journal);
            }
            match Watcher::start(root) {
                Ok(watcher) => Self::Watcher(watcher),
                Err(error) => Self::Nothing(error.to_string()),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = root;
            Self::Nothing("watching for changes is not built for this platform".to_owned())
        }
    }

    const fn mode(&self) -> Mode {
        match self {
            #[cfg(windows)]
            Self::Journal(_) => Mode::Journal,
            #[cfg(windows)]
            Self::Watcher(_) => Mode::Watching,
            Self::Nothing(_) => Mode::Still,
        }
    }

    fn note(&self) -> Option<String> {
        match self {
            Self::Nothing(why) => Some(why.clone()),
            #[cfg(windows)]
            Self::Journal(_) | Self::Watcher(_) => None,
        }
    }
}

/// What the keeper's source said since it was last asked.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the journal and the watcher, which are Windows-only, lose track or end")
)]
enum News {
    /// These paths changed.
    Paths(Vec<PathBuf>),
    /// The source cannot say what changed: the tree has to be read again.
    Lost(String),
    /// The source stopped for good.
    Ended(String),
}

impl Source {
    /// What changed since `from` — the journal's position the index is up to
    /// — and the position the journal is at now, when it is one.
    ///
    /// Reads the disk, and holds nothing of the session's while it does.
    fn news(&mut self, from: Option<JournalPosition>) -> (News, Option<JournalPosition>) {
        match self {
            #[cfg(windows)]
            Self::Journal(journal) => {
                let Some(from) = from else {
                    return (News::Lost("the index does not know where in the change journal it is".into()), None);
                };
                match journal.read_from(from) {
                    Ok(records) => (News::Paths(journal.paths(&records)), Some(journal.position())),
                    Err(midda_core::fresh::journal::Failure::Gap(gap)) => (News::Lost(gap.to_string()), None),
                    Err(error) => (News::Ended(error.to_string()), None),
                }
            }
            #[cfg(windows)]
            Self::Watcher(watcher) => {
                let _ = from;
                let drained = watcher.drain();
                let news = if let Some(why) = drained.ended {
                    News::Ended(format!("the watcher stopped: {why}"))
                } else if drained.lost {
                    News::Lost("changes came faster than the watcher could follow".into())
                } else {
                    News::Paths(drained.paths)
                };
                (news, None)
            }
            Self::Nothing(_) => {
                let _ = from;
                (News::Paths(Vec::new()), None)
            }
        }
    }

    /// Where the journal is now, for a read about to start: everything after
    /// it is applied once the read is done.
    fn position(&self) -> Option<JournalPosition> {
        #[cfg(windows)]
        if let Self::Journal(journal) = self {
            return Some(journal.position());
        }
        None
    }

    /// Whether a saved index that was up to `journal` can be brought up to
    /// date from here, or has to be read again — and why.
    fn catches_up_from(&self, journal: Option<JournalPosition>) -> Due {
        #[cfg(windows)]
        if let Self::Journal(reader) = self {
            return match journal.map(|at| reader.reaches(at)) {
                Some(Ok(())) => Due::Nothing,
                Some(Err(gap)) => Due::Read(Some(format!("the saved index was read again: {gap}"))),
                None => Due::Read(Some("the saved index was made without the change journal, and was read again".to_owned())),
            };
        }
        let _ = journal;
        Due::Read(None)
    }
}

/// Whether the keeper is to read the folder in full, and what to say about
/// why.
enum Due {
    Nothing,
    Read(Option<String>),
}

/// Where the keeper keeps what it saves: the index of the open folder, and
/// the folder's snapshots.
struct Places {
    store: PathBuf,
    snapshots: PathBuf,
}

/// The keeper: opens `root`, and keeps what is on screen current until told
/// to stop.
fn keep(shared: &RwLock<State>, root: &Path, places: &Places, stop: &AtomicBool, reread: &AtomicBool) {
    let store = places.store.as_path();
    let mut source = Source::open(root);
    let mut saved_at = Instant::now();
    let mut dirty = false;
    let mut saver = Saver::default();

    // The saved index first, if there is one: on screen at once.
    let mut due = match store::load(store, root) {
        Ok(saved) => {
            let journal = saved.index.journal();
            let saved_millis = millis(saved.saved_at);
            // Kept as the picture of last time before anything brings it up
            // to date: what "what grew since last time" is measured from.
            let last_note = snapshot::keep_last(&places.snapshots, &store::file_for(store, root))
                .err()
                .map(|error| format!("the picture of last time could not be kept: {error}"));
            if let Ok(mut state) = shared.write() {
                state.last_note = last_note;
                state.index = Some(saved.index);
                state.arena += 1;
                state.version += 1;
                state.freshness = Freshness {
                    mode: source.mode(),
                    saved_at: saved_millis,
                    catching_up: false,
                    note: None,
                };
            }
            source.catches_up_from(journal)
        }
        Err(Unusable::Missing) => Due::Read(None),
        Err(why) => Due::Read(Some(format!("the saved index was not used: {why}"))),
    };

    if matches!(due, Due::Nothing) {
        // Catch up from where the saved index left off.
        set(shared, |state| state.freshness.catching_up = true);
        let outcome = advance(shared, &mut source);
        set(shared, |state| {
            state.freshness.catching_up = false;
            state.freshness.saved_at = None;
        });
        match outcome {
            // Caught up with nothing to apply is still a new position to save.
            Ok(_) => dirty = true,
            Err(why) => due = Due::Read(Some(why)),
        }
    }

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if reread.swap(false, Ordering::Relaxed) && matches!(due, Due::Nothing) {
            due = Due::Read(None);
        }
        if let Due::Read(why) = std::mem::replace(&mut due, Due::Nothing) {
            // A source that stopped is opened again for a read asked for:
            // the folder may be back, the read may have been stopped by hand.
            if matches!(source, Source::Nothing(_)) {
                source = Source::open(root);
            }
            match read_in_full(shared, root, &mut source, why) {
                Read::Done => {
                    saver.save(shared, store);
                    saved_at = Instant::now();
                    dirty = false;
                }
                Read::Again(why) => {
                    saver.save(shared, store);
                    saved_at = Instant::now();
                    dirty = false;
                    due = Due::Read(Some(why));
                }
                Read::Stopped => {}
                Read::Failed => break,
            }
            continue;
        }

        match advance(shared, &mut source) {
            Ok(changed) => dirty |= changed,
            Err(why) => {
                due = Due::Read(Some(why));
                continue;
            }
        }
        if dirty && saved_at.elapsed() >= SAVE_EVERY {
            saver.save(shared, store);
            saved_at = Instant::now();
            dirty = false;
        }
        std::thread::sleep(TICK);
    }

    if dirty {
        saver.save(shared, store);
    }
    // The window is closing: the last save is finished before it does.
    saver.finish();
}

/// Applies what the source has to say. `Ok(true)` when the picture changed;
/// `Err` with the reason when the tree has to be read again.
fn advance(shared: &RwLock<State>, source: &mut Source) -> Result<bool, String> {
    let Some(from) = shared.read().ok().and_then(|state| state.index.as_ref().map(Index::journal)) else {
        return Ok(false);
    };
    let (news, at) = source.news(from);
    let paths = match news {
        News::Paths(paths) => paths,
        News::Lost(why) => return Err(why),
        News::Ended(why) => {
            *source = Source::Nothing(why.clone());
            set(shared, |state| {
                state.freshness.mode = Mode::Still;
                state.freshness.note = Some(why);
            });
            return Ok(false);
        }
    };
    if paths.is_empty() {
        if at.is_some() && at != from {
            set(shared, |state| {
                if let Some(index) = state.index.as_mut() {
                    index.follow(at);
                }
            });
        }
        return Ok(false);
    }
    // Looking is reading the disk: done under a read lock, so the window goes
    // on drawing meanwhile. Only the change itself holds the tree.
    let observed = {
        let Ok(state) = shared.read() else {
            return Ok(false);
        };
        let Some(index) = state.index.as_ref() else {
            return Ok(false);
        };
        index.observe(paths)
    };
    let Ok(mut state) = shared.write() else {
        return Ok(false);
    };
    let Some(index) = state.index.as_mut() else {
        return Ok(false);
    };
    let applied = index.apply(observed);
    if at.is_some() {
        index.follow(at);
    }
    state.version += 1;
    if applied.any() {
        state.applied_at = Some(SystemTime::now());
    }
    Ok(true)
}

enum Read {
    Done,
    /// Done, but the source lost track of changes made during the read: the
    /// picture is up, and is to be read again.
    Again(String),
    Stopped,
    Failed,
}

/// Reads the folder in full — under the picture on screen, if there is one —
/// and puts the new picture up with every change made during the read applied.
fn read_in_full(shared: &RwLock<State>, root: &Path, source: &mut Source, why: Option<String>) -> Read {
    let progress = Progress::new();
    let had_index = set(shared, |state| {
        state.reading = Some(Reading {
            progress: Arc::clone(&progress),
        });
        state.failure = None;
        if why.is_some() {
            state.freshness.note.clone_from(&why);
        }
        state.index.is_some()
    })
    .unwrap_or(false);

    // Everything the source reports from here on is applied to the new tree:
    // the journal by position, the watcher by what it keeps meanwhile.
    let from = source.position();
    let started = Instant::now();
    let outcome = midda_core::scan(root, &progress);
    let took = started.elapsed();

    match outcome {
        Ok(tree) => {
            let mut index = Index::new(tree, SystemTime::now());
            index.follow(from);
            let (news, at) = source.news(from);
            if at.is_some() {
                index.follow(at);
            }
            let (lost, again) = match news {
                News::Paths(paths) => {
                    let observed = index.observe(paths);
                    index.apply(observed);
                    (None, None)
                }
                News::Lost(why) => (Some(why.clone()), Some(why)),
                News::Ended(why) => (Some(why), None),
            };
            set(shared, |state| {
                state.index = Some(index);
                state.reading = None;
                state.took = Some(took);
                state.arena += 1;
                state.version += 1;
                state.freshness = Freshness {
                    mode: source.mode(),
                    saved_at: None,
                    catching_up: false,
                    note: lost.or_else(|| source.note()),
                };
            });
            again.map_or(Read::Done, Read::Again)
        }
        Err(midda_core::Error::Cancelled) => {
            set(shared, |state| {
                state.reading = None;
                if had_index {
                    // The picture stays, and says it was not read again.
                    state.freshness.mode = Mode::Still;
                    state.freshness.note = Some("reading the folder again was stopped".into());
                } else {
                    state.failure = Some("the scan was stopped".into());
                }
            });
            if had_index {
                *source = Source::Nothing("reading the folder again was stopped".into());
                Read::Stopped
            } else {
                Read::Failed
            }
        }
        Err(error) => {
            set(shared, |state| {
                state.reading = None;
                if had_index {
                    state.freshness.mode = Mode::Still;
                    state.freshness.note = Some(format!("the folder could not be read again: {error}"));
                } else {
                    state.failure = Some(error.to_string());
                }
            });
            if had_index { Read::Stopped } else { Read::Failed }
        }
    }
}

/// Saves the index on screen without holding up the keeper.
///
/// The index is written into memory under the lock — a fraction of a second
/// — and put on disk by a thread of its own, which can take seconds for a
/// large one. One write at a time: a save asked for while the last is still
/// on its way waits for it, so files never land out of order. A save that
/// fails costs only the next start's head start, and is not worth stopping
/// for.
#[derive(Default)]
struct Saver {
    writing: Option<JoinHandle<()>>,
}

impl Saver {
    fn save(&mut self, shared: &RwLock<State>, store: &Path) {
        let Some(encoded) = shared
            .read()
            .ok()
            .and_then(|state| state.index.as_ref().map(|index| store::encode(index, SystemTime::now())))
        else {
            return;
        };
        self.finish();
        let store = store.to_path_buf();
        self.writing = std::thread::Builder::new()
            .name("midda-save".into())
            .spawn(move || {
                let _ = encoded.write(&store);
            })
            .ok();
    }

    /// Waits for the write on its way, if there is one.
    fn finish(&mut self) {
        if let Some(writing) = self.writing.take() {
            let _ = writing.join();
        }
    }
}

/// Changes the state, if it is not poisoned.
fn set<T>(shared: &RwLock<State>, change: impl FnOnce(&mut State) -> T) -> Option<T> {
    shared.write().ok().map(|mut state| change(&mut state))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use midda_core::{Mark, Scanner, WalkScanner};

    use super::*;

    fn write(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the parent");
        }
        std::fs::write(path, vec![b'a'; bytes]).expect("write a fixture file");
    }

    fn index_of(root: &Path) -> Index {
        Index::new(WalkScanner::new().scan(root, &Progress::default()).expect("scan"), SystemTime::now())
    }

    fn opened(index: Index) -> RwLock<State> {
        RwLock::new(State {
            root: Some(index.tree().root_path().to_path_buf()),
            index: Some(index),
            ..State::default()
        })
    }

    #[test]
    fn a_picture_not_yet_brought_up_to_date_is_not_now() {
        let dir = tempfile::tempdir().expect("a folder");
        write(&dir.path().join("a.bin"), 10);
        let shared = opened(index_of(dir.path()));
        assert!(not_current(&shared.read().expect("read")).is_ok());

        // Last time's index on screen, not yet caught up: a snapshot of it
        // dated now would be last time under today's date.
        set(&shared, |state| state.freshness.saved_at = Some(0));
        assert!(not_current(&shared.read().expect("read")).is_err());
        set(&shared, |state| {
            state.freshness.saved_at = None;
            state.freshness.catching_up = true;
        });
        assert!(not_current(&shared.read().expect("read")).is_err());
        assert!(not_current(&State::default()).is_err(), "nothing read is nothing to compare");
    }

    #[test]
    fn last_time_compared_with_now_is_what_changed_while_the_folder_was_closed() {
        let dir = tempfile::tempdir().expect("a folder");
        let data = tempfile::tempdir().expect("a data folder");
        let root = dir.path();
        write(&root.join("build/app.exe"), 90_000);
        write(&root.join("src/main.rs"), 1_000);

        // Closed: the index is saved. Opened again: it is kept as last time.
        let store_dir = data.path().join("index");
        let saved = store::save(&index_of(root), &store_dir, SystemTime::now() - Duration::from_secs(3_600)).expect("save");
        let folder = snapshot::folder_for(&data.path().join("snapshots"), root);
        snapshot::keep_last(&folder, &saved).expect("keep last time");

        std::fs::remove_dir_all(root.join("build")).expect("clean the build");
        write(&root.join("downloads/setup.exe"), 40_000);
        let shared = opened(index_of(root));

        let summary = compare_in(&shared, &folder, root, "last.midx", None).expect("compare");
        assert!(summary.to_now);
        assert!(summary.then < summary.now);
        assert!(summary.totals.freed.logical >= 90_000);
        assert!(summary.totals.grown.logical >= 40_000);
        let state = shared.read().expect("read");
        assert_eq!(state.comparison_arena, summary.arena);
        let comparison = &state.comparison.as_ref().expect("kept for the window").comparison;
        let build = comparison.tree().child(ROOT, "build").expect("what went is in it");
        assert_eq!(comparison.mark(build), Mark::Gone);
        assert!(comparison.tree().child(ROOT, "src").is_none(), "what did not change is not");
    }

    #[test]
    fn two_snapshots_compare_older_to_newer_whichever_is_named_first() {
        let dir = tempfile::tempdir().expect("a folder");
        let data = tempfile::tempdir().expect("a data folder");
        let root = dir.path();
        write(&root.join("a.bin"), 1_000);
        let folder = snapshot::folder_for(data.path(), root);
        let monday = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let early = snapshot::keep(&store::encode(&index_of(root), monday), &folder, monday).expect("take");
        write(&root.join("b.bin"), 5_000);
        let tuesday = monday + Duration::from_secs(86_400);
        let late = snapshot::keep(&store::encode(&index_of(root), tuesday), &folder, tuesday).expect("take");

        let shared = opened(index_of(root));
        let summary = compare_in(&shared, &folder, root, &late.name, Some(&early.name)).expect("compare");
        assert_eq!((summary.then, summary.now), (millis(monday), millis(tuesday)));
        assert!(!summary.to_now);
        assert_eq!(summary.totals.new, 1, "b.bin came between the two");
        assert_eq!(summary.totals.gone, 0);
    }
}
