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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use midda_core::fresh::since::{self, Change, Changes};
use midda_core::fresh::store::{self, Unusable};
use midda_core::order::{self, Sort, Span};
use midda_core::treemap::{self, Layout, Tile};
use midda_core::{Index, JournalPosition, NodeId, Progress, ROOT, Tree};
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
}

impl Session {
    /// A session saving its indexes under `store`.
    #[must_use]
    pub fn new(store: PathBuf) -> Self {
        Self {
            shared: Arc::new(RwLock::new(State::default())),
            keeper: Mutex::new(None),
            store,
        }
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
}

struct Reading {
    progress: Arc<Progress>,
}

/// How the index on screen is kept current.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
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
        *state = State {
            root: Some(root.clone()),
            arena,
            changes_arena,
            ..State::default()
        };
    }

    let stop = Arc::new(AtomicBool::new(false));
    let reread = Arc::new(AtomicBool::new(false));
    let thread = {
        let shared = Arc::clone(&session.shared);
        let store = session.store.clone();
        let stop = Arc::clone(&stop);
        let reread = Arc::clone(&reread);
        std::thread::Builder::new()
            .name("midda-keeper".into())
            .spawn(move || keep(&shared, &root, &store, &stop, &reread))
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
                root: row(tree, ROOT, None),
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

/// The tree a view is about, checked against the arena the window's ids are
/// from.
fn tree_of(state: &State, view: View, arena: Option<u64>) -> Result<(&Tree, Option<&Changes>), String> {
    let (tree, changes, current) = match view {
        View::Index => (state.index.as_ref().map(Index::tree), None, state.arena),
        View::Changes => (state.changes.as_ref().map(|changes| &changes.tree), state.changes.as_ref(), state.changes_arena),
    };
    // The window's ids are from a tree read before this one: they name other
    // entries now. It finds its place again by path.
    if arena.is_some_and(|arena| arena != current) {
        return Err("stale".to_owned());
    }
    tree.map(|tree| (tree, changes)).ok_or_else(|| "nothing has been read yet".to_owned())
}

fn checked(tree: &Tree, id: u32) -> Result<NodeId, String> {
    if tree.contains(id) {
        Ok(id)
    } else {
        // Deleted while the window looked at it.
        Err("stale".to_owned())
    }
}

/// One page of the children of `id`, ordered by `sort`.
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
pub fn list_children(view: View, arena: u64, id: u32, sort: Sort, span: Span, session: tauri::State<'_, Session>) -> Result<Page, String> {
    let state = session.shared.read().map_err(poisoned)?;
    let (tree, changes) = tree_of(&state, view, Some(arena))?;
    let id = checked(tree, id)?;
    let (ids, total) = order::children_page(tree, id, sort, span);
    Ok(Page {
        rows: ids.into_iter().map(|child| row(tree, child, changes)).collect(),
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
    let (tree, changes) = tree_of(&state, view, Some(arena))?;
    let id = checked(tree, id)?;
    Ok(trail(tree, id, changes))
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
    let (tree, changes) = tree_of(&state, view, None)?;
    let arena = match view {
        View::Index => state.arena,
        View::Changes => state.changes_arena,
    };
    let mut at = ROOT;
    if let Ok(rest) = Path::new(&path).strip_prefix(tree.root_path()) {
        for part in rest.components() {
            match tree.child(at, &part.as_os_str().to_string_lossy()) {
                Some(child) => at = child,
                None => break,
            }
        }
    }
    Ok(Trail {
        arena,
        rows: trail(tree, at, changes),
    })
}

/// The rectangles that draw the children of `id`.
///
/// # Errors
///
/// As [`list_children`].
#[tauri::command]
pub fn treemap(view: View, arena: u64, id: u32, layout: Layout, session: tauri::State<'_, Session>) -> Result<Vec<Tile>, String> {
    let state = session.shared.read().map_err(poisoned)?;
    let (tree, _) = tree_of(&state, view, Some(arena))?;
    let id = checked(tree, id)?;
    Ok(treemap::tiles(tree, id, layout))
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
    let changes = since::written_since(index.tree(), &records, moment, journal.place());
    state.changes_arena += 1;
    let summary = ChangesSummary {
        root: row(&changes.tree, ROOT, Some(&changes)),
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
    let _ = since::written_since;
    Err("the change journal is an NTFS feature".to_owned())
}

/// The trail from the root down to `id`, root first.
fn trail(tree: &Tree, id: NodeId, changes: Option<&Changes>) -> Vec<Row> {
    let mut trail = Vec::new();
    let mut at = id;
    loop {
        trail.push(row(tree, at, changes));
        if at == ROOT {
            break;
        }
        at = tree.node(at).parent;
    }
    trail.reverse();
    trail
}

/// Turns a node into a row.
fn row(tree: &Tree, id: NodeId, changes: Option<&Changes>) -> Row {
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
        change: changes.and_then(|changes| changes.mark(id)),
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
            _ => None,
        }
    }
}

/// What the keeper's source said since it was last asked.
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
        match self {
            #[cfg(windows)]
            Self::Journal(journal) => Some(journal.position()),
            _ => None,
        }
    }
}

/// Whether the keeper is to read the folder in full, and what to say about
/// why.
enum Due {
    Nothing,
    Read(Option<String>),
}

/// The keeper: opens `root`, and keeps what is on screen current until told
/// to stop.
fn keep(shared: &RwLock<State>, root: &Path, store: &Path, stop: &AtomicBool, reread: &AtomicBool) {
    let mut source = Source::open(root);
    let mut saved_at = Instant::now();
    let mut dirty = false;

    // The saved index first, if there is one: on screen at once.
    let mut due = match store::load(store, root) {
        Ok(saved) => {
            let journal = saved.index.journal();
            let saved_millis = millis(saved.saved_at);
            if let Ok(mut state) = shared.write() {
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
            match (&source, journal) {
                #[cfg(windows)]
                (Source::Journal(reader), Some(at)) => match reader.reaches(at) {
                    Ok(()) => Due::Nothing,
                    Err(gap) => Due::Read(Some(format!("the saved index was read again: {gap}"))),
                },
                #[cfg(windows)]
                (Source::Journal(_), None) => Due::Read(Some("the saved index was made without the change journal, and was read again".to_owned())),
                _ => Due::Read(None),
            }
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
                    save(shared, store);
                    saved_at = Instant::now();
                    dirty = false;
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
            save(shared, store);
            saved_at = Instant::now();
            dirty = false;
        }
        std::thread::sleep(TICK);
    }

    if dirty {
        save(shared, store);
    }
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
            let lost = match news {
                News::Paths(paths) => {
                    let observed = index.observe(paths);
                    index.apply(observed);
                    None
                }
                News::Lost(why) | News::Ended(why) => Some(why),
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
            Read::Done
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

/// Saves the index on screen. A save that fails costs only the next start's
/// head start, and is not worth stopping for.
fn save(shared: &RwLock<State>, store: &Path) {
    if let Ok(state) = shared.read()
        && let Some(index) = state.index.as_ref()
    {
        let _ = store::save(index, store, SystemTime::now());
    }
}

/// Changes the state, if it is not poisoned.
fn set<T>(shared: &RwLock<State>, change: impl FnOnce(&mut State) -> T) -> Option<T> {
    shared.write().ok().map(|mut state| change(&mut state))
}
