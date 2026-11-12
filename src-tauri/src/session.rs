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
//! date. Of either tree the window can ask who is eating the disk: the few
//! folders that account for its growth, and the pace (ADR 0011) — with each
//! folder a rule knows to be one thing named whole.
//!
//! Of the folder as it is the window can ask what can go: the folders the
//! rules find, grouped, each with how it comes back and how much deleting it
//! would give back — worked out again only when the tree has changed. A
//! cache whose own tool cleans it is cleaned by that tool, here, once the
//! tool has said it is the cache found, and the folder is read again so the
//! window can say how much came back (ADR 0012). What only a file can say —
//! where a shortcut points, what an installer installs — is read away from
//! the tree, once per change of the file (ADR 0016).
//!
//! And it can ask what fills it: every byte given to one consumer by the
//! places the rules files name, the places that hold the most of each, and
//! why the volume is as full as it is, as a paragraph — worked out once per
//! version of the tree, the first time it is asked (ADR 0014). The picture's
//! colours read the same rules: what they found is what a tool makes again.
//!
//! Of the virtual disks in it - WSL's, Docker's, a Hyper-V machine's - it can
//! ask what is inside each and what compacting it gives back, read from the
//! files once per version of the tree; and of Docker's engine, what it keeps
//! that nothing uses (ADR 0015).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use midda_core::checks::{self, Check, Kept, Machine, Seen};
use midda_core::compare::{Comparison, Mark, PlaceKind};
use midda_core::compress::{self, Compressed, Verdict, Why};
use midda_core::consumers::story::{self, Story};
use midda_core::consumers::{self, Classified, Consumer, Summary};
use midda_core::delete::{self, Removed, Way};
use midda_core::deletions::{Deletion, Log};
use midda_core::fresh::since::{Change, Changes};
use midda_core::fresh::store::{self, Unusable};
use midda_core::growth::{self, Growers};
use midda_core::layers::{self, Layer, Untouched};
use midda_core::order::{self, Sort, Span};
use midda_core::platform::Space;
use midda_core::rules::cleanup::{Ran, Reach};
use midda_core::rules::{self, Finding, Goes, Group, Risk, RuleSet, Survey};
use midda_core::snapshot::{self, Kind, Snapshot};
use midda_core::treemap::{self, Layout, Tile};
use midda_core::vdisk::{self, Looked, Owner, docker};
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
    /// What the rules found in the folder as it is, as of the tree it was
    /// worked out from.
    found: Arc<Mutex<Option<Found>>>,
    /// What the checks saw of files, by path: kept between versions of the
    /// tree, since most versions change none of them.
    seen: Arc<Mutex<Kept>>,
    /// What was deleted, and how each comes back.
    deletions: Arc<Log>,
    /// Where the reader's own rules files are (`crate::own`).
    rules_folder: PathBuf,
}

/// What the rules say of one version of the open folder's tree: worked out
/// once per version, and what fills it only once it is asked.
struct Found {
    /// The set the rules were, which each finding names its rule in.
    rules: Arc<RuleSet>,
    arena: u64,
    version: u64,
    survey: Survey,
    /// The folders the rules claim whole: what the picture colours as a
    /// tool's.
    units: HashSet<NodeId>,
    /// What fills it, once asked: every byte's consumer, the summary on
    /// disk and by size, and why it is full.
    classified: Option<Arc<Classified>>,
    summaries: [Option<Arc<Summary>>; 2],
    story: Option<Arc<Story>>,
    /// The virtual disks in it, read inside, once asked.
    disks: Option<Arc<Vec<Looked>>>,
    /// Every finding, the ones a check waited on settled, once asked.
    settled: Option<Arc<Vec<Settled>>>,
    /// What of each entry has been untouched for a year, once asked.
    untouched: Option<Arc<Untouched>>,
}

impl Session {
    /// A session keeping what it saves under `data` - indexes in `index`,
    /// snapshots in `snapshots` - and reading the reader's own rules from
    /// `rules` under `config`.
    #[must_use]
    pub fn new(data: &Path, config: &Path) -> Self {
        let rules_folder = config.join("rules");
        let (rules, own) = crate::own::load(&rules_folder);
        Self {
            shared: Arc::new(RwLock::new(State {
                rules: crate::own::Rules(rules),
                own,
                ..State::default()
            })),
            keeper: Mutex::new(None),
            store: data.join("index"),
            snapshots: data.join("snapshots"),
            found: Arc::new(Mutex::new(None)),
            seen: Arc::new(Mutex::new(Kept::default())),
            deletions: Arc::new(Log::at(data.join("deletions.jsonl"))),
            rules_folder,
        }
    }

    /// Where the reader's own rules files are.
    pub(crate) fn rules_folder(&self) -> &Path {
        &self.rules_folder
    }

    /// The set in effect, and how each of the reader's files read.
    pub(crate) fn own_rules(&self) -> Result<(Arc<RuleSet>, Vec<crate::own::RulesFileRow>), String> {
        let state = self.shared.read().map_err(poisoned)?;
        Ok((Arc::clone(&state.rules.0), state.own.clone()))
    }

    /// Puts `rules` in effect. What is on screen changes with them - the
    /// findings, the colours, what fills it - so the version moves, and the
    /// window asks again.
    pub(crate) fn put_rules(&self, rules: Arc<RuleSet>, own: Vec<crate::own::RulesFileRow>) -> Result<(), String> {
        let mut state = self.shared.write().map_err(poisoned)?;
        state.rules = crate::own::Rules(rules);
        state.own = own;
        state.version += 1;
        Ok(())
    }

    /// Where the entry `id` of the folder as it is is.
    pub(crate) fn entry_path(&self, arena: u64, id: u32) -> Result<PathBuf, String> {
        let state = self.shared.read().map_err(poisoned)?;
        let tree = tree_of(&state, View::Index, Some(arena))?.tree();
        Ok(tree.path_of(checked(tree, id)?))
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
    /// The folders of the tree above that are one thing, as the folder as it
    /// was then says.
    changes_units: HashSet<NodeId>,
    /// Moves when the answer above is replaced.
    changes_arena: u64,
    /// Two pictures of the folder compared, once asked.
    comparison: Option<Compared>,
    /// Moves when the comparison above is replaced.
    comparison_arena: u64,
    /// Why the picture of last time was not kept when the folder was opened,
    /// when it was not.
    last_note: Option<String>,
    /// A compression running now.
    compressing: Option<Compressing>,
    /// A deletion running now.
    deleting: Option<Deleting>,
    /// The rules the folder is surveyed with: the reader's own and midda's.
    rules: crate::own::Rules,
    /// The reader's own files, and how each read.
    own: Vec<crate::own::RulesFileRow>,
}

/// A compression running: what, how far, and out of how much.
struct Compressing {
    path: PathBuf,
    progress: Arc<Progress>,
    /// What the tree said the entry reads as when it started.
    of: u64,
}

/// A deletion running: how far, out of how much, and the entry in hand.
struct Deleting {
    progress: Arc<Progress>,
    /// What the entries picked read as, in all.
    of: u64,
    /// How many were picked, and how many are done.
    items: u64,
    done: u64,
    path: PathBuf,
}

/// A comparison, and the two moments it is between.
struct Compared {
    comparison: Comparison,
    then: SystemTime,
    now: SystemTime,
    /// What the volume had free when the later picture is now: what a pace
    /// is set against. Not asked for two snapshots - their pace was then.
    free: Option<u64>,
    /// The folders of the comparison that are one thing in the later
    /// picture, and in the earlier one: what grew is named by the first,
    /// what was freed by the second, which still held what went.
    units_now: HashSet<NodeId>,
    units_then: HashSet<NodeId>,
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
    /// In the view of what is untouched: what of it nothing has written to
    /// for a year. `null` everywhere else.
    pub untouched: Option<Size>,
    /// When a file beneath this was last opened, as milliseconds since the
    /// epoch - only where the volume keeps that; `null` everywhere else.
    pub accessed: Option<i64>,
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
    /// The folder as it is, by what of it nothing has written to for a year:
    /// the same tree and ids as `Index`, its sizes the old bytes only.
    Old,
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
    /// A compression running now, and how far it is.
    pub compressing: Option<CompressingRow>,
    /// A deletion running now, and how far it is.
    pub deleting: Option<DeletingRow>,
}

/// How far a compression is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressingRow {
    pub path: String,
    /// Files gone through so far, and what they read as.
    pub files: u64,
    pub bytes: u64,
    /// What the whole reads as.
    pub of: u64,
}

/// How far a deletion is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletingRow {
    /// The entry in hand.
    pub path: String,
    /// Entries picked, and done.
    pub items: u64,
    pub done: u64,
    /// Files gone through so far, and what they read as - and what the
    /// whole reads as.
    pub files: u64,
    pub bytes: u64,
    pub of: u64,
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
    /// Whether the volume keeps the time files were last opened: where it
    /// does not, no row says when anything was.
    pub access_kept: bool,
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

/// One line of who is eating the disk, as the window shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowerRow {
    /// The folder in the view's tree, for the window to go to.
    pub id: u32,
    pub path: String,
    /// What the whole folder grew by, the lines inside it included. In the
    /// list of what was freed, what it freed.
    pub growth: i64,
    /// What this line accounts for apart from the lines inside it.
    pub own: i64,
    /// How many lines are named inside it.
    pub inside: u32,
    /// The line is the files directly in the folder compared, not a folder.
    pub loose: bool,
    /// How many entries came, went or changed under it.
    pub entries: u64,
    /// In a comparison, how the folder came to be in it; `null` in the view
    /// of what was written.
    pub mark: Option<Mark>,
}

/// One side of who is eating the disk: the lines, and what they leave.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowerSide {
    /// The most first.
    pub growers: Vec<GrowerRow>,
    /// The lines past the ones listed, added up.
    pub more: Rest,
    /// What grew in pieces too small to name, on balance.
    pub rest: i64,
    /// What a folder had to account for on its own to be named.
    pub threshold: u64,
}

/// How fast the folder grew, and how long the volume's free space lasts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaceRow {
    /// What the folder grew by in a day, on the size shown.
    pub per_day: i64,
    /// Days until the volume's free space is gone at this pace - on disk,
    /// whatever size is shown, because free space is clusters. `null` when
    /// the folder is not growing on disk or the free space is not known.
    pub full_in_days: Option<f64>,
}

/// Who is eating the disk, as the window shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Top {
    pub arena: u64,
    /// The folders that grew - or, in the view of what was written, that the
    /// most was written to.
    pub grew: GrowerSide,
    /// The folders that shrank, in a comparison; `null` in the view of what
    /// was written, which cannot know.
    pub freed: Option<GrowerSide>,
    /// How long the span is, in milliseconds.
    pub span_ms: Option<u64>,
    /// The pace, when the view is a comparison and its span is long enough
    /// to read one.
    pub pace: Option<PaceRow>,
    /// What the volume had free when the comparison was made.
    pub free: Option<u64>,
}

fn poisoned<T>(_: T) -> String {
    "the session is poisoned".to_owned()
}

/// What the session holds once `root` is opened: nothing of the folder
/// before, every id handed out for it stale - and the reader's rules as they
/// were, since they are the window's, not the folder's.
fn open_anew(state: &mut State, root: &Path) {
    let arena = state.arena + 1;
    let changes_arena = state.changes_arena + 1;
    let comparison_arena = state.comparison_arena + 1;
    let rules = std::mem::take(&mut state.rules);
    let own = std::mem::take(&mut state.own);
    *state = State {
        root: Some(root.to_path_buf()),
        arena,
        changes_arena,
        comparison_arena,
        rules,
        own,
        ..State::default()
    };
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
    open_anew(&mut *session.shared.write().map_err(poisoned)?, &root);

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
                access_kept: tree.access_kept(),
            }
        }),
        error: state.failure.clone(),
        freshness: state.freshness.clone(),
        applied_at: state.applied_at.and_then(millis),
        version: state.version,
        arena: state.arena,
        compressing: state.compressing.as_ref().map(|compressing| CompressingRow {
            path: compressing.path.to_string_lossy().into_owned(),
            files: compressing.progress.entries(),
            bytes: compressing.progress.bytes(),
            of: compressing.of,
        }),
        deleting: state.deleting.as_ref().map(|deleting| DeletingRow {
            path: deleting.path.to_string_lossy().into_owned(),
            items: deleting.items,
            done: deleting.done,
            files: deleting.progress.entries(),
            bytes: deleting.progress.bytes(),
            of: deleting.of,
        }),
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
        View::Index | View::Old => state.arena,
        View::Changes => state.changes_arena,
        View::Compare => state.comparison_arena,
    }
}

/// The tree a view is about, checked against the arena the window's ids are
/// from.
fn tree_of(state: &State, view: View, arena: Option<u64>) -> Result<Shown<'_>, String> {
    let shown = match view {
        View::Index | View::Old => state.index.as_ref().map(|index| Shown::Index(index.tree())),
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
    let untouched = untouched_for(&state, &session.found, view, tree)?;
    let (ids, total) = match (shown, &untouched) {
        (Shown::Compare(comparison), _) => comparison.children_page(id, sort, basis, span),
        (_, Some(untouched)) => untouched.children_page(tree, id, sort, basis, span),
        (Shown::Index(_) | Shown::Changes(_), None) => order::children_page(tree, id, sort, span),
    };
    let mut rows: Vec<Row> = ids.into_iter().map(|child| row(shown, child)).collect();
    untouched_rows(&mut rows, untouched.as_deref());
    Ok(Page {
        rows,
        offset: span.offset,
        total: u32::try_from(total).unwrap_or(u32::MAX),
    })
}

/// What of each entry is untouched, when `view` is the view of it: worked
/// out once per version of the tree on screen.
fn untouched_for(state: &State, found: &Mutex<Option<Found>>, view: View, tree: &Tree) -> Result<Option<Arc<Untouched>>, String> {
    if view != View::Old {
        return Ok(None);
    }
    with_found(state, found, tree, |found| {
        Some(Arc::clone(
            found.untouched.get_or_insert_with(|| Arc::new(Untouched::of(tree, SystemTime::now()))),
        ))
    })
}

/// Rows of the view of what is untouched carry what of each is.
fn untouched_rows(rows: &mut [Row], untouched: Option<&Untouched>) {
    if let Some(untouched) = untouched {
        for row in rows {
            row.untouched = Some(untouched.size(row.id));
        }
    }
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
    let mut rows = trail(shown, id);
    untouched_rows(&mut rows, untouched_for(&state, &session.found, view, shown.tree())?.as_deref());
    Ok(rows)
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
    let mut rows = trail(shown, at);
    untouched_rows(&mut rows, untouched_for(&state, &session.found, view, tree)?.as_deref());
    Ok(Trail { arena, rows })
}

/// The rectangles that draw the children of `id`.
///
/// Off the window's thread: the folder as it is is coloured by what the
/// rules find in it, and when the tree has changed since they last looked,
/// they look again.
///
/// # Errors
///
/// As [`list_children`].
#[tauri::command]
pub async fn treemap(view: View, arena: u64, id: u32, layout: Layout, session: tauri::State<'_, Session>) -> Result<Vec<Tile>, String> {
    let shared = Arc::clone(&session.shared);
    let found = Arc::clone(&session.found);
    tauri::async_runtime::spawn_blocking(move || tiles_of(&shared, &found, view, arena, id, layout))
        .await
        .map_err(|error| error.to_string())?
}

fn tiles_of(shared: &RwLock<State>, found: &Mutex<Option<Found>>, view: View, arena: u64, id: u32, layout: Layout) -> Result<Vec<Tile>, String> {
    let state = shared.read().map_err(poisoned)?;
    let shown = tree_of(&state, view, Some(arena))?;
    let id = checked(shown.tree(), id)?;
    let rules = Arc::clone(&state.rules.0);
    let now = SystemTime::now();
    Ok(match shown {
        // Drawn by what moved, not by what is there.
        Shown::Compare(comparison) => comparison.tiles(id, layout),
        // Drawn by what of it is old, coloured by how old.
        Shown::Index(tree) if view == View::Old => untouched_for(&state, found, view, tree)?
            .map(|untouched| untouched.tiles(tree, id, layout))
            .unwrap_or_default(),
        Shown::Index(tree) => {
            let (rules, units) = with_found(&state, found, tree, |found| (Arc::clone(&found.rules), found.units.clone()))?;
            treemap::tiles(tree, id, layout, &rules, &units, now)
        }
        Shown::Changes(changes) => treemap::tiles(&changes.tree, id, layout, &rules, &state.changes_units, now),
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
    let units = rules::units_in(&changes.tree, index.tree(), &state.rules.0.units(index.tree()));
    state.changes_units = units;
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
    // Asked here, off the window's thread: a network volume can be slow to
    // say how full it is.
    let free = if to.is_none() {
        midda_core::platform::space_of(root).map(|space| space.free)
    } else {
        None
    };
    let compared = match to {
        Some(name) => {
            let (second, second_at) = read(name)?;
            let (then, now) = if first_at <= second_at {
                ((&first, first_at), (&second, second_at))
            } else {
                ((&second, second_at), (&first, first_at))
            };
            let comparison = compare(then.0.tree(), now.0.tree());
            let rules = Arc::clone(&shared.read().map_err(poisoned)?.rules.0);
            Compared {
                units_now: rules::units_in(comparison.tree(), now.0.tree(), &rules.units(now.0.tree())),
                units_then: rules::units_in(comparison.tree(), then.0.tree(), &rules.units(then.0.tree())),
                comparison,
                then: then.1,
                now: now.1,
                free: None,
            }
        }
        None => {
            let state = shared.read().map_err(poisoned)?;
            not_current(&state)?;
            let index = state.index.as_ref().ok_or_else(|| "nothing has been read yet".to_owned())?;
            let comparison = compare(first.tree(), index.tree());
            Compared {
                units_now: rules::units_in(comparison.tree(), index.tree(), &state.rules.0.units(index.tree())),
                units_then: rules::units_in(comparison.tree(), first.tree(), &state.rules.0.units(first.tree())),
                comparison,
                then: first_at,
                now: SystemTime::now(),
                free,
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

/// Who is eating the disk in the comparison or the view of what was written
/// on screen: the folders that account for the growth on `basis`, `limit` of
/// them each way and the rest added up, and the pace.
///
/// # Errors
///
/// When the view holds nothing yet, is the folder as it is, or `arena` is not
/// the one on screen.
#[tauri::command]
pub fn top_growth(view: View, arena: u64, basis: SizeBasis, limit: u32, session: tauri::State<'_, Session>) -> Result<Top, String> {
    let state = session.shared.read().map_err(poisoned)?;
    top_of(&state, view, arena, basis, limit as usize)
}

fn top_of(state: &State, view: View, arena: u64, basis: SizeBasis, limit: usize) -> Result<Top, String> {
    let shown = tree_of(state, view, Some(arena))?;
    let tree = shown.tree();
    let side = |growers: Growers, mark: &dyn Fn(NodeId) -> Option<Mark>| GrowerSide {
        more: Rest {
            places: growers.growers.len().saturating_sub(limit) as u64,
            bytes: growers.growers.iter().skip(limit).map(|grower| grower.own.unsigned_abs()).sum(),
        },
        rest: growers.rest,
        threshold: growers.threshold,
        growers: growers
            .growers
            .into_iter()
            .take(limit)
            .map(|grower| GrowerRow {
                id: grower.id,
                path: tree.path_of(grower.id).to_string_lossy().into_owned(),
                growth: grower.growth,
                own: grower.own,
                inside: grower.inside,
                loose: grower.loose,
                // What changed under it: a folder that came or went whole is
                // one of those entries itself, one that is still there is
                // not - it only holds them.
                entries: if grower.loose {
                    tree.root().children.iter().filter(|&&child| !tree.node(child).is_directory()).count() as u64
                } else if matches!(mark(grower.id), Some(Mark::New | Mark::Gone | Mark::Replaced)) {
                    tree.node(grower.id).entries
                } else {
                    tree.node(grower.id).entries.saturating_sub(1)
                },
                mark: if grower.loose { None } else { mark(grower.id) },
            })
            .collect(),
    };
    let millis_of = |span: Duration| u64::try_from(span.as_millis()).unwrap_or(u64::MAX);
    match shown {
        Shown::Compare(comparison) => {
            let compared = state.comparison.as_ref();
            let span = compared.and_then(|compared| compared.now.duration_since(compared.then).ok());
            let free = compared.and_then(|compared| compared.free);
            let pace = span.and_then(|span| {
                let shown = growth::pace(comparison.growth(ROOT, basis), span, None)?;
                let on_disk = growth::pace(comparison.growth(ROOT, SizeBasis::Allocated), span, free)?;
                Some(PaceRow {
                    per_day: shown.per_day,
                    full_in_days: on_disk.full_in.map(|full| full.as_secs_f64() / growth::PACE_SPAN.as_secs_f64()),
                })
            });
            let (now, then) = compared.map_or((None, None), |compared| (Some(&compared.units_now), Some(&compared.units_then)));
            Ok(Top {
                arena,
                grew: side(comparison.growers(basis, among(now)), &|id| Some(comparison.mark(id))),
                freed: Some(side(comparison.shrinkers(basis, among(then)), &|id| Some(comparison.mark(id)))),
                span_ms: span.map(millis_of),
                pace,
                free,
            })
        }
        Shown::Changes(changes) => Ok(Top {
            arena,
            grew: side(changes.growers(basis, |id| state.changes_units.contains(&id)), &|_| None),
            freed: None,
            span_ms: SystemTime::now().duration_since(changes.since).ok().map(millis_of),
            pace: None,
            free: None,
        }),
        // The folder as it is has no second moment to have grown between.
        Shown::Index(_) => Err("growth is read from a comparison or from what was written lately".to_owned()),
    }
}

/// One finding, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingRow {
    /// The entry, in the tree of the folder as it is: for a folder that is
    /// emptied, the folder.
    pub id: u32,
    pub path: String,
    /// Which rule found it, and what the rule calls what it found.
    pub rule: String,
    pub name: String,
    pub risk: Risk,
    /// What brings it back, when one command does.
    pub command: Option<String>,
    /// What it is, how it comes back and what is lost.
    pub explain: String,
    /// What else the reader is told of this one: what its check saw, or how
    /// much of the folder goes.
    pub detail: Option<String>,
    /// Whether it is a file, a folder, or what is in a folder that goes.
    pub goes: Goes,
    pub folder: bool,
    /// What the tree counts for what goes.
    pub holds: Size,
    /// What deleting it would give back: what it holds less the files with
    /// a name somewhere else.
    pub frees: Size,
    /// How many entries go.
    pub entries: u64,
    /// When anything that goes was last written.
    pub subtree_modified: Option<i64>,
    /// The cache's own cleanup, when it has one.
    pub cleanup: Option<CleanupRow>,
    /// How it goes unless the reader says otherwise: for good when it comes
    /// back as it was, into the Recycle Bin when it may not.
    pub way: Way,
    /// Why it cannot go to the Recycle Bin, when it cannot: it is in the bin
    /// already, or its volume has none.
    pub bin_refused: Option<String>,
}

/// A cache's own cleanup, as the window offers it before asking the tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupRow {
    /// The command with the first of the tools: `pnpm store prune`.
    pub line: String,
    /// What it does.
    pub explain: String,
}

/// One group of findings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupRow {
    pub group: Group,
    /// The most freed first, up to the limit asked.
    pub rows: Vec<FindingRow>,
    /// How many findings are in the group, listed or not.
    pub count: u64,
    pub holds: Size,
    pub frees: Size,
    /// The findings past the ones listed: how many, and what they free.
    pub more: u64,
    pub more_frees: Size,
}

/// What can go in the folder as it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingsSummary {
    pub arena: u64,
    /// The version of the tree these are of.
    pub version: u64,
    /// Dependencies, build output, caches — those with anything in them.
    pub groups: Vec<GroupRow>,
    pub count: u64,
    pub holds: Size,
    pub frees: Size,
    /// What the findings that come back as they were free.
    pub safe: Size,
    /// What the findings to look at first free.
    pub caution: Size,
}

/// What a cleanup did, and how much came back.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleaned {
    pub ran: Ran,
    /// What the folder held before, and holds now, read again whole.
    pub before: Size,
    pub after: Size,
}

/// What the rules find in the folder as it is: each group's findings, the
/// most freed on `basis` first, `limit` of each — every one of `whole` —
/// and what each group and the whole add up to.
///
/// # Errors
///
/// `stale` when `arena` is not the tree on screen; otherwise when nothing
/// has been read.
#[tauri::command]
pub async fn findings(arena: u64, basis: SizeBasis, limit: u32, whole: Option<Group>, session: tauri::State<'_, Session>) -> Result<FindingsSummary, String> {
    let shared = Arc::clone(&session.shared);
    let found = Arc::clone(&session.found);
    let seen = Arc::clone(&session.seen);
    tauri::async_runtime::spawn_blocking(move || findings_of(&shared, &found, &seen, arena, basis, limit as usize, whole))
        .await
        .map_err(|error| error.to_string())?
}

/// A finding, with what its check saw - or what its rule tells of it - when
/// there is anything.
type Settled = (Finding, Option<Seen>);

/// Every finding of one version of the tree, and the set of rules they are
/// findings of: a finding names its rule by its place in the set, so the two
/// travel together.
#[derive(Clone)]
struct SettledSet {
    rules: Arc<RuleSet>,
    settled: Arc<Vec<Settled>>,
}

/// Every finding of the tree on screen, the checked ones settled — once per
/// version of the tree. The files are read with no lock on the tree held, as
/// the virtual disks are; what a check saw of a file is kept between
/// versions, and asked again only of a file that changed.
fn current_settled(shared: &RwLock<State>, found: &Mutex<Option<Found>>, seen: &Mutex<Kept>, arena: u64) -> Result<SettledSet, String> {
    let (version, rules, findings, waiting, told) = {
        let state = shared.read().map_err(poisoned)?;
        let tree = tree_of(&state, View::Index, Some(arena))?.tree();
        let held = with_found(&state, found, tree, |found| match &found.settled {
            Some(settled) => Ok(SettledSet {
                rules: Arc::clone(&found.rules),
                settled: Arc::clone(settled),
            }),
            None => Err((
                Arc::clone(&found.rules),
                found.survey.findings.clone(),
                found.survey.waiting.clone(),
                found.survey.told.clone(),
            )),
        })?;
        match held {
            Ok(settled) => return Ok(settled),
            Err((rules, findings, waiting, told)) => (state.version, rules, findings, waiting, told),
        }
    };
    let mut machine: Option<Machine> = None;
    let mut kept = seen.lock().map_err(poisoned)?;
    let checked = kept.settle(&rules, &waiting, &mut |entry| match &entry.check {
        // The programs installed are read once, by the first installer that
        // is asked, for every one after it.
        Check::Installer => checks::look(&entry.path, &entry.check, machine.get_or_insert_with(Machine::read)),
        check => checks::look(&entry.path, check, &Machine::default()),
    });
    let mut said: std::collections::HashMap<NodeId, Seen> = kept.tell(&told, &mut |entry| checks::tell(&entry.path, entry.about)).into_iter().collect();
    drop(kept);
    let settled: Vec<Settled> = findings
        .into_iter()
        .map(|finding| {
            let told = said.remove(&finding.id);
            (finding, told)
        })
        .chain(checked.into_iter().map(|(finding, seen)| (finding, Some(seen))))
        .collect();
    let settled = Arc::new(settled);
    if let Some(held) = found
        .lock()
        .map_err(poisoned)?
        .as_mut()
        .filter(|held| held.arena == arena && held.version == version && Arc::ptr_eq(&held.rules, &rules))
    {
        held.settled = Some(Arc::clone(&settled));
    }
    Ok(SettledSet { rules, settled })
}

/// What the rules say of the tree on screen, worked out again only when it
/// has changed since they last were, handed to `read`.
fn with_found<T>(state: &State, found: &Mutex<Option<Found>>, tree: &Tree, read: impl FnOnce(&mut Found) -> T) -> Result<T, String> {
    let mut found = found.lock().map_err(poisoned)?;
    let current = found
        .as_ref()
        .is_some_and(|held| held.arena == state.arena && held.version == state.version && Arc::ptr_eq(&held.rules, &state.rules.0));
    if !current {
        let rules = Arc::clone(&state.rules.0);
        let survey = rules.survey(tree);
        *found = Some(Found {
            rules,
            arena: state.arena,
            version: state.version,
            units: survey.units(),
            survey,
            classified: None,
            summaries: [None, None],
            story: None,
            disks: None,
            settled: None,
            untouched: None,
        });
    }
    Ok(read(found.as_mut().expect("worked out above")))
}

/// The findings of the tree on screen, and the set they are findings of.
fn current_findings(state: &State, found: &Mutex<Option<Found>>, tree: &Tree) -> Result<(Arc<RuleSet>, Vec<Finding>), String> {
    with_found(state, found, tree, |found| (Arc::clone(&found.rules), found.survey.findings.clone()))
}

/// What fills a version of the tree, as one ask of it needs it.
struct Filled {
    rules: Arc<RuleSet>,
    classified: Arc<Classified>,
    summary: Arc<Summary>,
    story: Arc<Story>,
}

/// Every byte of a version of the tree given to its consumer, worked out the
/// first time it is asked.
fn classified_in(found: &mut Found, tree: &Tree) -> Arc<Classified> {
    let survey = &found.survey;
    Arc::clone(
        found
            .classified
            .get_or_insert_with(|| Arc::new(consumers::classify(&found.rules, tree, survey))),
    )
}

/// What fills the tree on screen: every byte given to its consumer, the
/// summary on `basis` and why it is full — each worked out the first time it
/// is asked of a version of the tree, the story with `disks` read inside.
/// The window asks again whenever the tree changes, and asks the same version
/// as often as it redraws.
fn current_fills(state: &State, found: &Mutex<Option<Found>>, tree: &Tree, basis: SizeBasis, space: Option<Space>, disks: &[Looked]) -> Result<Filled, String> {
    with_found(state, found, tree, |found| {
        let rules = Arc::clone(&found.rules);
        let classified = classified_in(found, tree);
        let mut summary = |basis: SizeBasis| {
            let slot = match basis {
                SizeBasis::Allocated => 0,
                SizeBasis::Logical => 1,
            };
            Arc::clone(found.summaries[slot].get_or_insert_with(|| Arc::new(classified.summary(tree, basis))))
        };
        let on_disk = summary(SizeBasis::Allocated);
        let shown = summary(basis);
        let told = Arc::clone(
            found
                .story
                .get_or_insert_with(|| Arc::new(story::tell_from(&rules, tree, &classified, &on_disk, space, disks))),
        );
        Filled {
            rules,
            classified,
            summary: shown,
            story: told,
        }
    })
}

fn findings_of(
    shared: &RwLock<State>,
    found: &Mutex<Option<Found>>,
    seen: &Mutex<Kept>,
    arena: u64,
    basis: SizeBasis,
    limit: usize,
    whole: Option<Group>,
) -> Result<FindingsSummary, String> {
    let SettledSet { rules, settled } = current_settled(shared, found, seen, arena)?;
    let rules = rules.as_ref();
    let state = shared.read().map_err(poisoned)?;
    let tree = tree_of(&state, View::Index, Some(arena))?.tree();
    let mut findings: Vec<&Settled> = settled.iter().collect();
    // The core orders on disk; a reader looking at sizes gets their order.
    findings.sort_by(|(a, _), (b, _)| basis.of(b.frees).cmp(&basis.of(a.frees)).then_with(|| tree.scan_order(a.id, b.id)));

    let mut summary = FindingsSummary {
        arena,
        version: state.version,
        groups: Vec::new(),
        count: 0,
        holds: Size::zero(),
        frees: Size::zero(),
        safe: Size::zero(),
        caution: Size::zero(),
    };
    for group in Group::ALL {
        let mut row = GroupRow {
            group,
            rows: Vec::new(),
            count: 0,
            holds: Size::zero(),
            frees: Size::zero(),
            more: 0,
            more_frees: Size::zero(),
        };
        let listed = if whole == Some(group) { usize::MAX } else { limit };
        for (finding, seen) in findings.iter().copied().filter(|(finding, _)| finding.group == group) {
            row.count += 1;
            row.holds.add(finding.holds);
            row.frees.add(finding.frees);
            match rules.restore_of(finding).risk {
                Risk::Safe => summary.safe.add(finding.frees),
                Risk::Caution => summary.caution.add(finding.frees),
            }
            if row.rows.len() < listed {
                row.rows.push(finding_row(tree, rules, finding, seen.as_ref()));
            } else {
                row.more += 1;
                row.more_frees.add(finding.frees);
            }
        }
        if row.count > 0 {
            summary.count += row.count;
            summary.holds.add(row.holds);
            summary.frees.add(row.frees);
            summary.groups.push(row);
        }
    }
    Ok(summary)
}

/// A place that holds a sizeable part of one consumer, as the window lists
/// it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillLine {
    /// The entry to open: the place the line is, its folder, or the root for
    /// the root's own files.
    pub id: u32,
    pub path: String,
    /// What a place calls it, when one does.
    pub name: Option<String>,
    /// What it holds of the consumer, on the size shown.
    pub bytes: u64,
    /// The line is the files directly in the root.
    pub loose: bool,
}

/// One consumer, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillShare {
    pub consumer: Consumer,
    /// What a reader calls it, and what is in it.
    pub label: String,
    pub about: String,
    pub size: Size,
    /// What of it a tool makes again: what its findings free.
    pub frees: Size,
    /// What compacting its virtual disks gives back, when that is worth
    /// saying: only the virtual disks have it.
    pub compacts: Option<u64>,
    /// The places that hold the most of it, the most first, up to the limit
    /// asked.
    pub lines: Vec<FillLine>,
    /// How many places past the ones listed.
    pub more: u64,
    /// What no place names, in pieces under `threshold` each.
    pub rest: u64,
    pub threshold: u64,
}

/// The size of a volume and what it has free.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceRow {
    pub total: u64,
    pub free: u64,
}

/// What fills the folder as it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fills {
    pub arena: u64,
    /// The version of the tree this is of.
    pub version: u64,
    /// What the folder holds: what the shares add up to.
    pub total: Size,
    /// The volume's size and free space, when the folder is a whole volume.
    pub space: Option<SpaceRow>,
    /// Each consumer that holds anything, the most first.
    pub shares: Vec<FillShare>,
    /// Why it is as full as it is, as a paragraph - on disk, whatever size is
    /// shown.
    pub story: String,
    /// The paragraph and every consumer's places, as Markdown: what an
    /// assistant is given.
    pub brief: String,
}

/// What fills the folder as it is: each consumer, the most on `basis` first,
/// with `limit` of its places - every one of `whole`'s - and why the volume is
/// as full as it is.
///
/// # Errors
///
/// `stale` when `arena` is not the tree on screen; otherwise when nothing
/// has been read.
#[tauri::command]
pub async fn fills(arena: u64, basis: SizeBasis, limit: u32, whole: Option<Consumer>, session: tauri::State<'_, Session>) -> Result<Fills, String> {
    let shared = Arc::clone(&session.shared);
    let found = Arc::clone(&session.found);
    tauri::async_runtime::spawn_blocking(move || fills_of(&shared, &found, arena, basis, limit as usize, whole))
        .await
        .map_err(|error| error.to_string())?
}

fn fills_of(
    shared: &RwLock<State>,
    found: &Mutex<Option<Found>>,
    arena: u64,
    basis: SizeBasis,
    limit: usize,
    whole: Option<Consumer>,
) -> Result<Fills, String> {
    // Asked before the tree is held: a network volume can be slow to say how
    // full it is, and the keeper should not wait on it.
    let root = shared.read().map_err(poisoned)?.root.clone();
    let space = root.as_deref().filter(|root| story::is_volume(root)).and_then(midda_core::platform::space_of);
    let disks = current_disks(shared, found, arena)?;

    let state = shared.read().map_err(poisoned)?;
    let tree = tree_of(&state, View::Index, Some(arena))?.tree();
    let Filled {
        rules,
        classified,
        summary,
        story: told,
    } = current_fills(&state, found, tree, basis, space, &disks)?;
    let rules = rules.as_ref();
    let shares = summary
        .shares
        .iter()
        .map(|share| {
            let listed = if whole == Some(share.consumer) { usize::MAX } else { limit };
            FillShare {
                consumer: share.consumer,
                label: share.consumer.label().to_owned(),
                about: share.consumer.about().to_owned(),
                size: share.size,
                frees: share.frees,
                compacts: (share.consumer == Consumer::VirtualDisks)
                    .then(|| vdisk::gives_back(&disks, share.size.allocated))
                    .flatten(),
                more: share.lines.len().saturating_sub(listed) as u64,
                lines: share
                    .lines
                    .iter()
                    .take(listed)
                    .map(|line| {
                        let entry = Classified::entry_of(line);
                        FillLine {
                            id: entry,
                            path: tree.path_of(entry).to_string_lossy().into_owned(),
                            name: classified.name_of(rules, tree, share.consumer, line),
                            bytes: line.whole,
                            loose: line.loose,
                        }
                    })
                    .collect(),
                rest: share.rest,
                threshold: share.threshold,
            }
        })
        .collect();
    Ok(Fills {
        arena,
        version: state.version,
        total: summary.total,
        space: space.map(|space| SpaceRow {
            total: space.total,
            free: space.free,
        }),
        shares,
        story: told.text.clone(),
        brief: told.brief.clone(),
    })
}

/// The virtual disks of the tree on screen, read inside — once per version
/// of the tree. The files are read with no lock held: the keeper is not to
/// wait on a disk being read, and a version that moved on meanwhile is read
/// again the next time it is asked.
fn current_disks(shared: &RwLock<State>, found: &Mutex<Option<Found>>, arena: u64) -> Result<Arc<Vec<Looked>>, String> {
    let (version, disks) = {
        let state = shared.read().map_err(poisoned)?;
        let tree = tree_of(&state, View::Index, Some(arena))?.tree();
        let held = with_found(&state, found, tree, |found| match &found.disks {
            Some(looked) => Ok(Arc::clone(looked)),
            None => Err(vdisk::find(&Arc::clone(&found.rules), tree, &classified_in(found, tree))),
        })?;
        match held {
            Ok(looked) => return Ok(looked),
            Err(disks) => (state.version, disks),
        }
    };
    let looked = Arc::new(vdisk::look_all(disks));
    if let Some(held) = found
        .lock()
        .map_err(poisoned)?
        .as_mut()
        .filter(|held| held.arena == arena && held.version == version)
    {
        held.disks = Some(Arc::clone(&looked));
    }
    Ok(looked)
}

/// A virtual disk, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskRow {
    /// The file, in the tree of the folder as it is.
    pub id: u32,
    pub path: String,
    /// What it is: `Docker's disk`.
    pub name: String,
    pub owner: Option<Owner>,
    /// `VHDX`, when midda knows the kind of file.
    pub format: Option<String>,
    /// What the file takes.
    pub file: Size,
    /// What is inside, as measured.
    pub inside: String,
    /// Why the file is larger, and what compacting does.
    pub explain: String,
    /// What compacting gives back now, when that was measured.
    pub gives_back: Option<u64>,
    /// Whether that is worth compacting for.
    pub worth: bool,
    /// What has to stop first.
    pub first: Option<String>,
    /// What compacts it, for an administrator PowerShell.
    pub script: Option<String>,
}

/// The virtual disks of the folder as it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Disks {
    pub arena: u64,
    /// The version of the tree these are of.
    pub version: u64,
    /// The largest first.
    pub disks: Vec<DiskRow>,
    /// What compacting the ones worth it gives back.
    pub gives_back: u64,
}

/// The virtual disks of the folder as it is, each read inside: what it
/// holds, what compacting it gives back and how.
///
/// # Errors
///
/// `stale` when `arena` is not the tree on screen; otherwise when nothing
/// has been read.
#[tauri::command]
pub async fn disks(arena: u64, session: tauri::State<'_, Session>) -> Result<Disks, String> {
    let shared = Arc::clone(&session.shared);
    let found = Arc::clone(&session.found);
    tauri::async_runtime::spawn_blocking(move || disks_of(&shared, &found, arena))
        .await
        .map_err(|error| error.to_string())?
}

fn disks_of(shared: &RwLock<State>, found: &Mutex<Option<Found>>, arena: u64) -> Result<Disks, String> {
    let looked = current_disks(shared, found, arena)?;
    let version = shared.read().map_err(poisoned)?.version;
    Ok(Disks {
        arena,
        version,
        gives_back: vdisk::worth_compacting(&looked),
        disks: looked
            .iter()
            .map(|Looked { disk, look, told }| DiskRow {
                id: disk.id,
                path: disk.path.to_string_lossy().into_owned(),
                name: told.name.clone(),
                owner: disk.owner,
                format: look.format.map(|format| format.name().to_owned()),
                file: disk.size,
                inside: told.inside.clone(),
                explain: told.explain.clone(),
                gives_back: told.gives_back,
                worth: told.worth,
                first: told.first.clone(),
                script: told.script.clone(),
            })
            .collect(),
    })
}

/// One kind of thing Docker keeps that nothing uses, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneRow {
    pub what: String,
    pub count: u64,
    pub bytes: u64,
    pub risk: Risk,
    pub command: String,
    pub explain: String,
}

/// What Docker keeps, as its engine says.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerRow {
    /// What it keeps in all: images, containers, volumes, build cache.
    pub size: u64,
    /// What of it nothing uses, the safest first.
    pub prunes: Vec<PruneRow>,
    /// What a prune is followed by.
    pub after: String,
}

/// What Docker keeps and what of it nothing uses, asked of its engine over
/// the pipe or socket the `docker` command talks to.
///
/// # Errors
///
/// When Docker is not running, refuses this account, or does not answer in
/// time - said as it is.
#[tauri::command]
pub async fn docker_usage() -> Result<DockerRow, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let usage = docker::ask(&docker::Endpoint::of_environment()?, docker::WITHIN)?;
        Ok(DockerRow {
            size: usage.size(),
            prunes: vdisk::prunes(&usage)
                .into_iter()
                .map(|prune| PruneRow {
                    what: prune.what.to_owned(),
                    count: prune.count,
                    bytes: prune.bytes,
                    risk: prune.risk,
                    command: prune.command.to_owned(),
                    explain: prune.explain.to_owned(),
                })
                .collect(),
            after: vdisk::AFTER_PRUNE.to_owned(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn finding_row(tree: &Tree, rules: &RuleSet, finding: &Finding, seen: Option<&Seen>) -> FindingRow {
    let rule = rules.rule_of(finding);
    let restore = rules.restore_of(finding);
    let path = tree.path_of(finding.id);
    let bin_refused = delete::bin_refused(&path);
    FindingRow {
        id: finding.id,
        way: Way::of_entry(restore.risk, bin_refused.is_none()),
        bin_refused,
        path: path.to_string_lossy().into_owned(),
        rule: rule.id.clone(),
        name: rule.name.clone(),
        risk: restore.risk,
        command: restore.command.clone(),
        explain: restore.explain.clone(),
        detail: rules.detail(finding, seen),
        goes: finding.goes,
        folder: tree.node(finding.id).is_directory(),
        holds: finding.holds,
        frees: finding.frees,
        entries: finding.entries,
        subtree_modified: finding.newest.and_then(millis),
        cleanup: rule.cleanup.as_ref().map(|cleanup| CleanupRow {
            line: cleanup.line(),
            explain: cleanup.explain.clone(),
        }),
    }
}

/// The folder of the finding `id` and its cleanup, from the findings of the
/// tree on screen.
fn cleanup_of(session: &Session, arena: u64, id: u32) -> Result<(PathBuf, rules::Cleanup), String> {
    let state = session.shared.read().map_err(poisoned)?;
    let tree = tree_of(&state, View::Index, Some(arena))?.tree();
    let id = checked(tree, id)?;
    let (rules, findings) = current_findings(&state, &session.found, tree)?;
    let finding = findings
        .iter()
        .find(|finding| finding.id == id)
        .ok_or_else(|| "no rule names this folder now".to_owned())?;
    let cleanup = rules
        .rule_of(finding)
        .cleanup
        .clone()
        .ok_or_else(|| "this folder has no cleanup of its own".to_owned())?;
    Ok((tree.path_of(id), cleanup))
}

/// Whether the cleanup of the finding `id` would run: its tool is on this
/// machine and says the folder it cleans is this one. Asks the tool, which
/// only prints.
///
/// # Errors
///
/// `stale` when `arena` is not the tree on screen or the folder is gone;
/// otherwise when the folder is no finding with a cleanup.
#[tauri::command]
pub async fn cleanup_reach(arena: u64, id: u32, session: tauri::State<'_, Session>) -> Result<Reach, String> {
    let (folder, cleanup) = cleanup_of(&session, arena, id)?;
    tauri::async_runtime::spawn_blocking(move || cleanup.reach(&folder, std::env::var_os("PATH").as_deref()))
        .await
        .map_err(|error| error.to_string())
}

/// Runs the cleanup of the finding `id` — after asking its tool once more
/// that this is the folder it cleans — and reads the folder again whole, so
/// what came back is measured rather than assumed.
///
/// # Errors
///
/// As [`cleanup_reach`], and when the tool is not there, cleans another
/// folder, or could not be started. A tool that ran and failed is not an
/// error: it says so in what it ran.
#[tauri::command]
pub async fn run_cleanup(arena: u64, id: u32, session: tauri::State<'_, Session>) -> Result<Cleaned, String> {
    let (folder, cleanup) = cleanup_of(&session, arena, id)?;
    let shared = Arc::clone(&session.shared);
    tauri::async_runtime::spawn_blocking(move || clean(&shared, &folder, &cleanup, std::env::var_os("PATH").as_deref()))
        .await
        .map_err(|error| error.to_string())?
}

/// Runs `cleanup` on `folder`, with its tool looked for on `search`, and
/// reads the folder again whole.
fn clean(shared: &RwLock<State>, folder: &Path, cleanup: &rules::Cleanup, search: Option<&std::ffi::OsStr>) -> Result<Cleaned, String> {
    let before = size_at(shared, folder)?;
    let ran = cleanup.run(folder, search)?;
    // Read again whole: the journal or the watcher would report the files
    // one by one, later; the reader is waiting now.
    let observed = {
        let state = shared.read().map_err(poisoned)?;
        state.index.as_ref().map(|index| index.read_again(folder))
    };
    if let Some(observed) = observed {
        let mut state = shared.write().map_err(poisoned)?;
        if let Some(index) = state.index.as_mut() {
            index.apply(observed);
            state.version += 1;
            state.applied_at = Some(SystemTime::now());
        }
    }
    let after = size_at(shared, folder)?;
    Ok(Cleaned { ran, before, after })
}

/// What the folder at `path` holds in the tree on screen: nothing, when the
/// tree does not hold it.
fn size_at(shared: &RwLock<State>, path: &Path) -> Result<Size, String> {
    let state = shared.read().map_err(poisoned)?;
    let index = state.index.as_ref().ok_or_else(|| "nothing has been read yet".to_owned())?;
    let tree = index.tree();
    let Some(names) = index.below_root(path) else {
        return Ok(Size::zero());
    };
    let mut at = ROOT;
    for name in &names {
        match tree.child(at, name) {
            Some(child) => at = child,
            None => return Ok(Size::zero()),
        }
    }
    Ok(tree.node(at).size)
}

/// How old the layers of the folder are, as the window shows them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayersSummary {
    pub arena: u64,
    /// The version of the tree these are of.
    pub version: u64,
    /// Its bytes by how long ago each file was written, the youngest first.
    pub strata: Vec<Layer>,
    /// The largest fossils, each inside no larger one.
    pub fossils: Vec<FossilRow>,
    /// How many fossils there are, listed or not, and what they take.
    pub count: u64,
    pub size: Size,
    pub more: u64,
    pub more_size: Size,
    /// Whether the volume keeps the time files were last opened.
    pub access_kept: bool,
    /// Why nothing on this volume can be compressed, when nothing can.
    pub compress_refused: Option<String>,
}

/// One fossil, as the window lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FossilRow {
    pub id: u32,
    pub path: String,
    pub is_directory: bool,
    pub size: Size,
    pub entries: u64,
    /// When anything in it was last written, and when a file in it was last
    /// opened, as milliseconds since the epoch.
    pub written: Option<i64>,
    pub opened: Option<i64>,
    /// What compressing it would read, when it can be compressed.
    pub compresses: Option<Size>,
    /// Why it cannot be, when it cannot.
    pub not_compressed: Option<Why>,
    /// What a rule calls it, when a rule finds it whole: an old folder a tool
    /// makes again is better deleted than compressed.
    pub finding: Option<String>,
}

/// What compressing an entry did, and how much came back.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressedRow {
    pub done: Compressed,
    /// What it held before, and holds now, read again whole.
    pub before: Size,
    pub after: Size,
}

/// How old the layers of the folder as it is are: the strata, and the
/// `limit` largest fossils on `basis`, each with whether it can be
/// compressed.
///
/// # Errors
///
/// `stale` when `arena` is not the tree on screen; otherwise when nothing
/// has been read.
#[tauri::command]
pub async fn layers(arena: u64, basis: SizeBasis, limit: u32, session: tauri::State<'_, Session>) -> Result<LayersSummary, String> {
    let shared = Arc::clone(&session.shared);
    let found = Arc::clone(&session.found);
    tauri::async_runtime::spawn_blocking(move || layers_of(&shared, &found, arena, basis, limit as usize, SystemTime::now()))
        .await
        .map_err(|error| error.to_string())?
}

fn layers_of(
    shared: &RwLock<State>,
    found: &Mutex<Option<Found>>,
    arena: u64,
    basis: SizeBasis,
    limit: usize,
    now: SystemTime,
) -> Result<LayersSummary, String> {
    let state = shared.read().map_err(poisoned)?;
    let tree = tree_of(&state, View::Index, Some(arena))?.tree();
    let fossils = layers::fossils(tree, ROOT, now, basis, limit);
    let refused = compress::refused_on(tree.root_path());
    let (classified, findings, rules) = with_found(&state, found, tree, |found| {
        (classified_in(found, tree), found.survey.findings.clone(), Arc::clone(&found.rules))
    })?;
    let rules = rules.as_ref();
    let rows = fossils
        .listed
        .iter()
        .map(|fossil| {
            let (compresses, not_compressed) = match compress::verdict(tree, fossil.id, &classified, refused.as_deref()) {
                Verdict::Can { reads } => (Some(reads), None),
                Verdict::Not(why) => (None, Some(why)),
            };
            FossilRow {
                id: fossil.id,
                path: tree.path_of(fossil.id).to_string_lossy().into_owned(),
                is_directory: tree.node(fossil.id).is_directory(),
                size: fossil.size,
                entries: fossil.entries,
                written: fossil.written.and_then(millis),
                opened: fossil.opened.and_then(millis),
                compresses,
                not_compressed,
                finding: findings
                    .iter()
                    .find(|finding| finding.id == fossil.id)
                    .map(|finding| rules.rule_of(finding).name.clone()),
            }
        })
        .collect();
    Ok(LayersSummary {
        arena,
        version: state.version,
        strata: layers::strata(tree, ROOT, now),
        fossils: rows,
        count: fossils.count,
        size: fossils.size,
        more: fossils.more,
        more_size: fossils.more_size,
        access_kept: tree.access_kept(),
        compress_refused: refused,
    })
}

/// Compresses the entry `id` of the folder as it is with NTFS compression,
/// and reads it again whole, so what came back is measured rather than
/// assumed. One at a time; the window follows it in `scan_progress`.
///
/// # Errors
///
/// `stale` when `arena` is not the tree on screen or the entry is gone;
/// when a compression is running already, the entry is not one that can be
/// compressed, or the volume cannot compress.
#[tauri::command]
pub async fn compress_entry(arena: u64, id: u32, session: tauri::State<'_, Session>) -> Result<CompressedRow, String> {
    let shared = Arc::clone(&session.shared);
    let found = Arc::clone(&session.found);
    tauri::async_runtime::spawn_blocking(move || compress_in(&shared, &found, arena, id))
        .await
        .map_err(|error| error.to_string())?
}

/// Asks the compression running to stop after the file it is on.
///
/// # Errors
///
/// When the session lock is poisoned.
#[tauri::command]
pub fn cancel_compress(session: tauri::State<'_, Session>) -> Result<(), String> {
    let state = session.shared.read().map_err(poisoned)?;
    if let Some(compressing) = &state.compressing {
        compressing.progress.cancel();
    }
    Ok(())
}

fn compress_in(shared: &RwLock<State>, found: &Mutex<Option<Found>>, arena: u64, id: u32) -> Result<CompressedRow, String> {
    let progress = Progress::new();
    let (path, before) = {
        let mut state = shared.write().map_err(poisoned)?;
        if state.compressing.is_some() {
            return Err("a compression is running already".to_owned());
        }
        let tree = tree_of(&state, View::Index, Some(arena))?.tree();
        let id = checked(tree, id)?;
        let classified = with_found(&state, found, tree, |found| classified_in(found, tree))?;
        let refused = compress::refused_on(tree.root_path());
        if let Verdict::Not(why) = compress::verdict(tree, id, &classified, refused.as_deref()) {
            return Err(match why {
                Why::Volume(why) => why,
                Why::Windows => "Windows' own files are left as Windows keeps them".to_owned(),
                Why::Compressed => "it is compressed already".to_owned(),
                Why::Nothing => "nothing in it would compress".to_owned(),
            });
        }
        let path = tree.path_of(id);
        let before = tree.node(id).size;
        state.compressing = Some(Compressing {
            path: path.clone(),
            progress: Arc::clone(&progress),
            of: before.logical,
        });
        (path, before)
    };
    let done = compress::compress(&path, &progress);
    // Read again whole, as a cleanup is: the reader is waiting to see what
    // came back, and the journal would say it file by file, later.
    let observed = {
        let state = shared.read().map_err(poisoned)?;
        state.index.as_ref().map(|index| index.read_again(&path))
    };
    {
        let mut state = shared.write().map_err(poisoned)?;
        state.compressing = None;
        if let Some(observed) = observed
            && let Some(index) = state.index.as_mut()
        {
            index.apply(observed);
            state.version += 1;
            state.applied_at = Some(SystemTime::now());
        }
    }
    let done = done?;
    let after = size_at(shared, &path)?;
    Ok(CompressedRow { done, before, after })
}

/// One finding the reader picked, and how it goes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    pub id: u32,
    pub way: Way,
}

/// What deleting one finding did, measured.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedItem {
    pub path: String,
    /// What the rule calls it.
    pub name: String,
    pub way: Way,
    pub removed: Removed,
    /// What it held before, and holds now, read again.
    pub before: Size,
    pub after: Size,
}

/// What deleting the findings picked did.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedRow {
    /// The most first, in the order they went.
    pub items: Vec<DeletedItem>,
    /// The reader stopped it before the end.
    pub stopped: bool,
    /// What went for good, and what was moved to the Recycle Bin - measured,
    /// before less after.
    pub freed: Size,
    pub binned: Size,
    /// Why the log of deletions could not be written, when it could not:
    /// what was deleted is gone all the same, and the reader is told.
    pub unlogged: Option<String>,
}

/// One finding picked, worked out before anything goes.
struct Going {
    finding: Finding,
    path: PathBuf,
    way: Way,
    /// What goes, and what each reads as.
    entries: Vec<(PathBuf, u64)>,
    name: String,
    rule: String,
    risk: Risk,
    command: Option<String>,
    explain: String,
}

/// Deletes the findings picked, each its way, the most freed first: what is
/// picked for good, for good ([`delete::remove`]); what is picked for the
/// Recycle Bin, into it, and never for good instead ([`delete::recycle`]).
/// Each is read again as soon as it has gone, so the window and the log say
/// what went as measured, and each is written to the log of deletions with
/// how it comes back before the next goes. One deletion at a time, and none
/// while a compression runs; the window follows it in `scan_progress`.
///
/// # Errors
///
/// `stale` when `arena` is not the tree on screen, the findings were worked
/// out for another version of it, or an entry is gone; when nothing is
/// picked, a pick is no finding now, a pick for the bin cannot go there, or
/// something else is running.
#[tauri::command]
pub async fn delete_findings(arena: u64, picks: Vec<Pick>, session: tauri::State<'_, Session>) -> Result<DeletedRow, String> {
    let shared = Arc::clone(&session.shared);
    let found = Arc::clone(&session.found);
    let seen = Arc::clone(&session.seen);
    let log = Arc::clone(&session.deletions);
    tauri::async_runtime::spawn_blocking(move || delete_in(&shared, &found, &seen, &log, arena, &picks, SystemTime::now()))
        .await
        .map_err(|error| error.to_string())?
}

/// Asks the deletion running to stop after the file it is on.
///
/// # Errors
///
/// When the session lock is poisoned.
#[tauri::command]
pub fn cancel_delete(session: tauri::State<'_, Session>) -> Result<(), String> {
    let state = session.shared.read().map_err(poisoned)?;
    if let Some(deleting) = &state.deleting {
        deleting.progress.cancel();
    }
    Ok(())
}

fn delete_in(
    shared: &RwLock<State>,
    found: &Mutex<Option<Found>>,
    seen: &Mutex<Kept>,
    log: &Log,
    arena: u64,
    picks: &[Pick],
    now: SystemTime,
) -> Result<DeletedRow, String> {
    if picks.is_empty() {
        return Err("nothing is picked".to_owned());
    }
    let SettledSet { rules, settled } = current_settled(shared, found, seen, arena)?;
    let progress = Progress::new();
    let plan = {
        let mut state = shared.write().map_err(poisoned)?;
        if state.deleting.is_some() {
            return Err("a deletion is running already".to_owned());
        }
        if state.compressing.is_some() {
            return Err("a compression is running; delete once it is done".to_owned());
        }
        let tree = tree_of(&state, View::Index, Some(arena))?.tree();
        // The findings the window picked from are those of the tree as it is
        // now, or the ids mean other entries.
        let current = found.lock().map_err(poisoned)?.as_ref().is_some_and(|held| {
            held.arena == state.arena
                && held.version == state.version
                && Arc::ptr_eq(&held.rules, &rules)
                && held.settled.as_ref().is_some_and(|held| Arc::ptr_eq(held, &settled))
        });
        if !current {
            return Err("stale".to_owned());
        }
        let plan = plan_of(tree, &rules, &settled, picks, now)?;
        state.deleting = Some(Deleting {
            progress: Arc::clone(&progress),
            of: plan.iter().map(|going| going.finding.holds.logical).sum(),
            items: plan.len() as u64,
            done: 0,
            path: plan.first().map(|going| going.path.clone()).unwrap_or_default(),
        });
        plan
    };
    let outcome = take_all(shared, log, &plan, &progress);
    if let Ok(mut state) = shared.write() {
        state.deleting = None;
    }
    outcome
}

/// What each pick takes, worked out from `tree` before anything goes: the
/// picks in the order of what they free, the most first, each once.
fn plan_of(tree: &Tree, rules: &RuleSet, settled: &[Settled], picks: &[Pick], now: SystemTime) -> Result<Vec<Going>, String> {
    let mut plan: Vec<Going> = Vec::new();
    for pick in picks {
        let id = checked(tree, pick.id)?;
        if plan.iter().any(|going| going.finding.id == id) {
            continue;
        }
        let path = tree.path_of(id);
        let (finding, _) = settled
            .iter()
            .find(|(finding, _)| finding.id == id)
            .ok_or_else(|| format!("no rule names {} now", path.display()))?;
        if pick.way == Way::RecycleBin
            && let Some(why) = delete::bin_refused(&path)
        {
            return Err(format!("{} cannot go to the Recycle Bin: {why}", path.display()));
        }
        let rule = rules.rule_of(finding);
        let restore = rules.restore_of(finding);
        plan.push(Going {
            finding: *finding,
            entries: rules
                .going(tree, finding, now)
                .into_iter()
                .map(|entry| (tree.path_of(entry), tree.node(entry).size.logical))
                .collect(),
            path,
            way: pick.way,
            name: rule.name.clone(),
            rule: rule.id.clone(),
            risk: restore.risk,
            command: restore.command.clone(),
            explain: restore.explain.clone(),
        });
    }
    plan.sort_by_key(|going| std::cmp::Reverse(going.finding.frees.allocated));
    Ok(plan)
}

/// Takes each of `plan` its way, reads it again, and writes it to `log`.
fn take_all(shared: &RwLock<State>, log: &Log, plan: &[Going], progress: &Progress) -> Result<DeletedRow, String> {
    let mut row = DeletedRow {
        items: Vec::new(),
        stopped: false,
        freed: Size::zero(),
        binned: Size::zero(),
        unlogged: None,
    };
    for going in plan {
        if progress.cancelled() {
            row.stopped = true;
            break;
        }
        if let Some(deleting) = shared.write().map_err(poisoned)?.deleting.as_mut() {
            deleting.path.clone_from(&going.path);
        }
        let before = size_at(shared, &going.path)?;
        let removed = delete::take(&going.entries, going.way, progress);
        // Read again at once, as a cleanup is: the reader is waiting to see
        // what went, and the log says what went as measured. An entry that
        // went whole is gone from the tree; a folder emptied is read anew.
        let observed = {
            let state = shared.read().map_err(poisoned)?;
            state.index.as_ref().map(|index| index.read_again(&going.path))
        };
        {
            let mut state = shared.write().map_err(poisoned)?;
            if let Some(observed) = observed
                && let Some(index) = state.index.as_mut()
            {
                index.apply(observed);
                state.version += 1;
                state.applied_at = Some(SystemTime::now());
            }
            if let Some(deleting) = state.deleting.as_mut() {
                deleting.done += 1;
            }
        }
        let after = size_at(shared, &going.path)?;
        let gone = Size {
            logical: before.logical.saturating_sub(after.logical),
            allocated: before.allocated.saturating_sub(after.allocated),
        };
        match going.way {
            Way::ForGood => row.freed.add(gone),
            Way::RecycleBin => row.binned.add(gone),
        }
        if removed.any() {
            let written = log.write(&Deletion {
                // When this one went: a long deletion spans minutes.
                at: millis(SystemTime::now()).unwrap_or_default(),
                path: going.path.to_string_lossy().into_owned(),
                name: going.name.clone(),
                rule: going.rule.clone(),
                risk: going.risk,
                way: going.way,
                goes: going.finding.goes,
                held: before.allocated,
                gone: gone.allocated,
                left: removed.left.iter().map(|kept| kept.entries).sum(),
                command: going.command.clone(),
                // The command runs beside what it makes again.
                run_in: going
                    .command
                    .as_ref()
                    .and_then(|_| going.path.parent())
                    .map(|folder| folder.to_string_lossy().into_owned()),
                explain: going.explain.clone(),
            });
            if let Err(error) = written
                && row.unlogged.is_none()
            {
                row.unlogged = Some(error.to_string());
            }
        }
        let stopped = removed.stopped;
        row.items.push(DeletedItem {
            path: going.path.to_string_lossy().into_owned(),
            name: going.name.clone(),
            way: going.way,
            removed,
            before,
            after,
        });
        if stopped {
            row.stopped = true;
            break;
        }
    }
    Ok(row)
}

/// The log of deletions, the latest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionLog {
    /// Up to the limit asked.
    pub rows: Vec<Deletion>,
    /// How many are written down in all, and what they gave back for good.
    pub count: u64,
    pub freed: u64,
    /// Where the log is, for a reader who wants the file.
    pub file: String,
}

/// What midda has deleted, the latest first, `limit` of it, each with how it
/// comes back.
///
/// # Errors
///
/// When the log is there and cannot be read.
#[tauri::command]
pub async fn deletion_log(limit: u32, session: tauri::State<'_, Session>) -> Result<DeletionLog, String> {
    let log = Arc::clone(&session.deletions);
    tauri::async_runtime::spawn_blocking(move || {
        let all = log.read().map_err(|error| format!("the log of deletions could not be read: {error}"))?;
        Ok(DeletionLog {
            count: all.len() as u64,
            freed: all.iter().filter(|deletion| deletion.way == Way::ForGood).map(|deletion| deletion.gone).sum(),
            rows: all.into_iter().take(limit as usize).collect(),
            file: log.path().to_string_lossy().into_owned(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Whether a folder is one of `units`: none are, when there are none.
fn among(units: Option<&HashSet<NodeId>>) -> impl Fn(NodeId) -> bool + '_ {
    move |id| units.is_some_and(|units| units.contains(&id))
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
        untouched: None,
        accessed: node.accessed.and_then(millis),
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

    /// A fresh session's place for findings.
    fn nothing_found() -> Mutex<Option<Found>> {
        Mutex::new(None)
    }

    /// And for what its checks saw.
    fn nothing_seen() -> Mutex<Kept> {
        Mutex::new(Kept::default())
    }

    #[test]
    fn what_can_go_is_grouped_counted_and_worked_out_once_per_change() {
        let dir = tempfile::tempdir().expect("a folder");
        let root = dir.path();
        write(&root.join("app/package.json"), 10);
        write(&root.join("app/pnpm-lock.yaml"), 10);
        write(&root.join("app/node_modules/a/index.js"), 30_000);
        write(&root.join("loose/package.json"), 10);
        write(&root.join("loose/node_modules/b/index.js"), 20_000);
        for name in ["one", "two", "three"] {
            write(&root.join(format!("{name}/Cargo.toml")), 10);
            write(&root.join(format!("{name}/target/debug/app.exe")), 50_000);
        }
        write(&root.join("site/package.json"), 10);
        write(&root.join("site/dist/index.html"), 4_000);
        let shared = opened(index_of(root));
        let found = nothing_found();

        let summary = findings_of(&shared, &found, &nothing_seen(), 0, SizeBasis::Logical, 2, None).expect("findings");
        let groups: Vec<_> = summary.groups.iter().map(|group| (group.group, group.count)).collect();
        assert_eq!(groups, [(Group::Dependencies, 2), (Group::Build, 4)], "no caches here, so no group for them");
        assert_eq!(summary.count, 6);
        assert_eq!(summary.frees.logical, 30_000 + 20_000 + 150_000 + 4_000);
        // A node_modules without a lockfile and a dist are the two to look
        // at first; the rest come back as they were.
        assert_eq!(summary.caution.logical, 20_000 + 4_000);
        assert_eq!(summary.safe.logical, 30_000 + 150_000);

        let build = &summary.groups[1];
        assert_eq!(build.rows.len(), 2, "two listed");
        assert_eq!((build.more, build.more_frees.logical), (2, 50_000 + 4_000), "and what the rest come to");
        assert_eq!(build.rows[0].command.as_deref(), Some("cargo build"));
        let dependencies = &summary.groups[0];
        assert!(dependencies.rows[0].path.ends_with("node_modules") && dependencies.rows[0].risk == Risk::Safe);
        assert_eq!(dependencies.rows[1].risk, Risk::Caution);

        let whole = findings_of(&shared, &found, &nothing_seen(), 0, SizeBasis::Logical, 2, Some(Group::Build)).expect("findings");
        assert_eq!(
            (whole.groups[1].rows.len(), whole.groups[1].more),
            (4, 0),
            "a group asked for whole is listed whole"
        );

        // The same tree is not searched again; a changed one is.
        let held = |found: &Mutex<Option<Found>>| found.lock().expect("lock").as_ref().map(|held| held.version);
        assert_eq!(held(&found), Some(0));
        set(&shared, |state| state.version = 7);
        findings_of(&shared, &found, &nothing_seen(), 0, SizeBasis::Logical, 2, None).expect("findings");
        assert_eq!(held(&found), Some(7));
        assert_eq!(
            findings_of(&shared, &found, &nothing_seen(), 99, SizeBasis::Logical, 2, None).expect_err("stale"),
            "stale"
        );
    }

    #[test]
    fn picked_findings_go_their_way_are_measured_and_written_down() {
        let dir = tempfile::tempdir().expect("a folder");
        let root = dir.path();
        write(&root.join("app/package.json"), 10);
        write(&root.join("app/pnpm-lock.yaml"), 10);
        write(&root.join("app/node_modules/a/index.js"), 30_000);
        write(&root.join("one/Cargo.toml"), 10);
        write(&root.join("one/target/debug/app.exe"), 50_000);
        write(&root.join("two/Cargo.toml"), 10);
        write(&root.join("two/target/debug/app.exe"), 40_000);
        let shared = opened(index_of(root));
        let found = nothing_found();
        let seen = nothing_seen();
        let log = Log::at(root.join("data").join("deletions.jsonl"));
        let id = |path: &str| {
            let state = shared.read().expect("read");
            let tree = state.index.as_ref().expect("an index").tree();
            path.split('/').fold(ROOT, |at, name| tree.child(at, name).expect("in the tree"))
        };

        let summary = findings_of(&shared, &found, &seen, 0, SizeBasis::Logical, 10, None).expect("findings");
        let modules = &summary.groups[0].rows[0];
        assert_eq!(modules.way, Way::ForGood, "what comes back as it was goes for good unless said otherwise");

        let (app, one) = (id("app/node_modules"), id("one/target"));
        let package = id("app/package.json");
        let version = shared.read().expect("read").version;
        let now = SystemTime::now();
        assert_eq!(delete_in(&shared, &found, &seen, &log, 0, &[], now).expect_err("nothing"), "nothing is picked");
        assert_eq!(
            delete_in(&shared, &found, &seen, &log, 99, &[Pick { id: app, way: Way::ForGood }], now).expect_err("another arena"),
            "stale"
        );
        assert!(
            delete_in(
                &shared,
                &found,
                &seen,
                &log,
                0,
                &[Pick {
                    id: package,
                    way: Way::ForGood
                }],
                now
            )
            .expect_err("no finding")
            .starts_with("no rule names"),
            "only what a rule names can go from here"
        );
        assert!(root.join("app/package.json").exists());

        let picks = [
            Pick { id: one, way: Way::ForGood },
            Pick { id: app, way: Way::ForGood },
            Pick { id: app, way: Way::ForGood },
        ];
        let deleted = delete_in(&shared, &found, &seen, &log, 0, &picks, now).expect("deleted");
        assert_eq!(deleted.items.len(), 2, "a pick given twice goes once");
        assert_eq!(deleted.items[0].name, "Cargo build output", "the most freed first");
        assert_eq!(deleted.freed.logical, 80_000, "measured, before less after");
        assert_eq!(deleted.binned, Size::zero());
        assert!(deleted.items.iter().all(|item| item.removed.whole() && item.after == Size::zero()));
        assert!(!root.join("app/node_modules").exists() && !root.join("one/target").exists());
        assert!(root.join("two/target/debug/app.exe").exists(), "what was not picked stays");
        assert!(root.join("app/pnpm-lock.yaml").exists(), "and what it comes back from");

        let state = shared.read().expect("read");
        assert!(state.version > version, "the window is told the tree changed");
        assert!(state.deleting.is_none());
        assert_eq!(state.index.as_ref().expect("an index").tree().root().size.logical, 40_000 + 40);
        drop(state);

        // Written down, the latest first, each with how it comes back and
        // where to run it.
        let written = log.read().expect("the log");
        assert_eq!(written.len(), 2);
        let modules = written.iter().find(|deletion| deletion.name == "node_modules").expect("the modules");
        assert_eq!(modules.command.as_deref(), Some("pnpm install"));
        assert_eq!(modules.run_in.as_deref(), Some(root.join("app").to_string_lossy().as_ref()));
        assert_eq!((modules.way, modules.risk, modules.left), (Way::ForGood, Risk::Safe, 0));
        assert!(modules.gone > 0 && modules.gone == modules.held);

        // The ids of a tree that changed are not taken for its entries now.
        assert_eq!(
            delete_in(&shared, &found, &seen, &log, 0, &[Pick { id: app, way: Way::ForGood }], now).expect_err("gone"),
            "stale"
        );
    }

    #[test]
    fn what_is_in_the_bin_is_not_picked_for_the_bin() {
        let dir = tempfile::tempdir().expect("a folder");
        let root = dir.path();
        write(&root.join("$Recycle.Bin/one/Cargo.toml"), 10);
        write(&root.join("$Recycle.Bin/one/target/app.exe"), 1_000);
        let shared = opened(index_of(root));
        let found = nothing_found();
        let seen = nothing_seen();
        let summary = findings_of(&shared, &found, &seen, 0, SizeBasis::Logical, 10, None).expect("findings");
        let target = &summary.groups[0].rows[0];
        assert!(target.bin_refused.is_some(), "the window is told it cannot go there");
        let log = Log::at(root.join("deletions.jsonl"));
        let refused = delete_in(
            &shared,
            &found,
            &seen,
            &log,
            0,
            &[Pick {
                id: target.id,
                way: Way::RecycleBin,
            }],
            SystemTime::now(),
        )
        .expect_err("refused");
        assert!(refused.contains("cannot go to the Recycle Bin"), "{refused}");
        assert!(root.join("$Recycle.Bin/one/target/app.exe").exists());
        assert!(shared.read().expect("read").deleting.is_none());
    }

    #[test]
    fn the_virtual_disks_are_read_inside_once_per_change() {
        let dir = tempfile::tempdir().expect("a folder");
        let root = dir.path();
        // Docker's disk by where it is - but not a disk file inside - and a
        // VirtualBox disk by its own signature.
        write(&root.join("Docker/wsl/disk/docker_data.vhdx"), 9_000);
        let mut vdi = vec![0_u8; 4_000];
        vdi[0x40..0x44].copy_from_slice(&[0x7F, 0x10, 0xDA, 0xBE]);
        std::fs::write(root.join("box.vdi"), vdi).expect("write the disk");
        let shared = opened(index_of(root));
        let found = nothing_found();

        let disks = disks_of(&shared, &found, 0).expect("the disks");
        let rows: Vec<(&str, Option<Owner>, Option<&str>)> = disks
            .disks
            .iter()
            .map(|disk| (disk.name.as_str(), disk.owner, disk.format.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [("Docker's disk", Some(Owner::Docker), None), ("A virtual disk", None, Some("VDI"))],
            "the largest first"
        );
        assert_eq!(disks.disks[0].inside, "midda cannot see inside it: it is not a disk file midda knows.");
        assert_eq!(disks.disks[0].gives_back, None);
        assert!(
            disks.disks[1]
                .script
                .as_deref()
                .is_some_and(|script| script.starts_with("VBoxManage modifymedium disk"))
        );
        assert_eq!(disks.gives_back, 0, "nothing was measured");

        let fills = fills_of(&shared, &found, 0, SizeBasis::Logical, 5, None).expect("what fills it");
        let virtual_disks = fills
            .shares
            .iter()
            .find(|share| share.consumer == Consumer::VirtualDisks)
            .expect("the disks' share");
        assert_eq!(virtual_disks.compacts, None, "nothing measured, nothing said");
        assert!(fills.story.contains("midda cannot see inside them."), "{}", fills.story);

        // The files are not read again for the same tree; for a changed one
        // they are.
        std::fs::remove_file(root.join("box.vdi")).expect("delete the disk");
        assert_eq!(disks_of(&shared, &found, 0).expect("the disks").disks[1].format.as_deref(), Some("VDI"));
        set(&shared, |state| state.version = 3);
        let again = disks_of(&shared, &found, 0).expect("the disks");
        assert!(again.disks[1].inside.contains("could not be opened"), "{:?}", again.disks[1]);
        assert_eq!(disks_of(&shared, &found, 99).expect_err("stale"), "stale");
    }

    #[test]
    fn a_build_folder_is_one_line_of_what_grew_and_of_what_was_freed() {
        let dir = tempfile::tempdir().expect("a folder");
        let data = tempfile::tempdir().expect("a data folder");
        let root = dir.path();
        write(&root.join("rust/Cargo.toml"), 10);
        write(&root.join("web/package.json"), 10);
        write(&root.join("web/package-lock.json"), 10);
        write(&root.join("web/node_modules/big/a.js"), 60_000);
        write(&root.join("web/node_modules/huge/b.js"), 80_000);
        let saved = store::save(&index_of(root), &data.path().join("index"), SystemTime::now() - Duration::from_secs(3_600)).expect("save");
        let folder = snapshot::folder_for(&data.path().join("snapshots"), root);
        snapshot::keep_last(&folder, &saved).expect("keep last time");

        // A build wrote two sizeable halves; the dependencies were deleted.
        write(&root.join("rust/target/debug/app.exe"), 70_000);
        write(&root.join("rust/target/release/app.exe"), 50_000);
        std::fs::remove_dir_all(root.join("web/node_modules")).expect("delete the dependencies");
        let shared = opened(index_of(root));
        let summary = compare_in(&shared, &folder, root, "last.midx", None).expect("compare");

        let state = shared.read().expect("read");
        let top = top_of(&state, View::Compare, summary.arena, SizeBasis::Logical, 10).expect("the top");
        let ends = |side: &GrowerSide| side.growers.iter().map(|grower| grower.path.replace('\\', "/")).collect::<Vec<_>>();
        let grew = ends(&top.grew);
        assert_eq!(grew.len(), 1, "{grew:?}");
        assert!(grew[0].ends_with("rust/target"), "the build is one thing, not its debug and release: {grew:?}");
        // What went is known to be one thing by the picture that still held it.
        let freed = ends(top.freed.as_ref().expect("what was freed"));
        assert_eq!(freed.len(), 1, "{freed:?}");
        assert!(freed[0].ends_with("web/node_modules"), "{freed:?}");
    }

    #[test]
    fn a_cleanup_reads_its_folder_again_and_says_what_came_back() {
        let dir = tempfile::tempdir().expect("a folder");
        let root = dir.path();
        write(&root.join("cache/blob"), 70_000);
        write(&root.join("cache/keep"), 1_000);
        let shared = opened(index_of(root));
        let cache = root.join("cache");

        // A tool that says it cleans `cache` and deletes its blob.
        let tools = tempfile::tempdir().expect("a folder for the tool");
        let shown = cache.display();
        if cfg!(windows) {
            let script = format!("@echo off\r\nif \"%~1\"==\"where\" (\r\n  echo {shown}\r\n  exit /b 0\r\n)\r\ndel /q \"{shown}\\blob\"\r\n");
            std::fs::write(tools.path().join("cachetool.cmd"), script).expect("write the tool");
        } else {
            let script = format!("#!/bin/sh\nif [ \"$1\" = where ]; then echo '{shown}'; exit 0; fi\nrm -f '{shown}/blob'\n");
            let path = tools.path().join("cachetool");
            std::fs::write(&path, script).expect("write the tool");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("make it runnable");
            }
        }
        let rules = RuleSet::parse(
            "[[rule]]\nid = \"c\"\nname = \"c\"\ngroup = \"caches\"\npaths = [\"cache\"]\n\
             [[rule.restore]]\nrisk = \"safe\"\nexplain = \"A cache.\"\n\
             [rule.cleanup]\ntools = [[\"cachetool\"]]\nrun = [\"clean\"]\nlocate = [\"where\"]\nexplain = \"It cleans.\"\n",
        )
        .expect("the rule reads");
        let cleanup = rules.rules()[0].cleanup.as_ref().expect("a cleanup");
        let search = std::env::join_paths([tools.path()]).expect("a search path");
        let version = shared.read().expect("read").version;

        let cleaned = clean(&shared, &cache, cleanup, Some(&search)).expect("it ran");
        assert!(cleaned.ran.succeeded, "{:?}", cleaned.ran);
        assert_eq!((cleaned.before.logical, cleaned.after.logical), (71_000, 1_000), "measured, not assumed");
        let state = shared.read().expect("read");
        assert!(state.version > version, "the window is told the tree changed");
        assert_eq!(state.index.as_ref().expect("an index").tree().root().size.logical, 1_000);
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
    fn who_ate_the_space_since_three_days_ago_and_at_what_pace() {
        let dir = tempfile::tempdir().expect("a folder");
        let data = tempfile::tempdir().expect("a data folder");
        let root = dir.path();
        write(&root.join("build/app.exe"), 90_000);
        write(&root.join("src/main.rs"), 1_000);
        let store_dir = data.path().join("index");
        let three_days = Duration::from_secs(3 * 86_400);
        let saved = store::save(&index_of(root), &store_dir, SystemTime::now() - three_days).expect("save");
        let folder = snapshot::folder_for(&data.path().join("snapshots"), root);
        snapshot::keep_last(&folder, &saved).expect("keep last time");

        // Since then: a download came, the source grew a file, the build went.
        write(&root.join("downloads/setup.exe"), 400_000);
        write(&root.join("downloads/notes.txt"), 2_000);
        write(&root.join("src/lib.rs"), 300_000);
        std::fs::remove_dir_all(root.join("build")).expect("clean the build");
        let shared = opened(index_of(root));
        let summary = compare_in(&shared, &folder, root, "last.midx", None).expect("compare");

        let state = shared.read().expect("read");
        let top = top_of(&state, View::Compare, summary.arena, SizeBasis::Logical, 10).expect("the top of a comparison");
        let names: Vec<_> = top
            .grew
            .growers
            .iter()
            .map(|grower| Path::new(&grower.path).file_name().map(|name| name.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(
            names,
            [Some("downloads".to_owned()), Some("src".to_owned())],
            "one line a folder, not one a file"
        );
        assert_eq!(top.grew.growers[0].mark, Some(Mark::New));
        assert_eq!(top.grew.growers[0].entries, 3, "a folder that came is one of what came, with its two files");
        assert_eq!(top.grew.growers[1].mark, Some(Mark::Changed));
        assert_eq!(top.grew.growers[1].entries, 1, "a folder that stayed only holds what changed: one file");
        let freed = top.freed.as_ref().expect("a comparison says what went");
        assert!(freed.growers[0].path.ends_with("build"));
        assert_eq!(freed.growers[0].own, 90_000);

        // Three days apart is a pace: a third of the net a day.
        let pace = top.pace.expect("three days is long enough for a pace");
        let net = 402_000 + 300_000 - 90_000;
        assert!((pace.per_day - net / 3).abs() <= 1, "{} a day", pace.per_day);
        let span = top.span_ms.expect("a span");
        assert!(span >= 3 * 86_400_000);
        if cfg!(windows) {
            assert!(top.free.is_some(), "the volume says what it has free");
            assert!(pace.full_in_days.is_some_and(|days| days > 0.0));
        }

        assert!(
            top_of(&state, View::Compare, summary.arena + 1, SizeBasis::Logical, 10).is_err(),
            "ids from another comparison"
        );
    }

    #[test]
    fn a_comparison_an_hour_apart_has_no_pace() {
        let dir = tempfile::tempdir().expect("a folder");
        let data = tempfile::tempdir().expect("a data folder");
        let root = dir.path();
        write(&root.join("a.bin"), 1_000);
        let saved = store::save(&index_of(root), &data.path().join("index"), SystemTime::now() - Duration::from_secs(3_600)).expect("save");
        let folder = snapshot::folder_for(&data.path().join("snapshots"), root);
        snapshot::keep_last(&folder, &saved).expect("keep last time");
        write(&root.join("cache/blob.bin"), 900_000);
        let shared = opened(index_of(root));
        let summary = compare_in(&shared, &folder, root, "last.midx", None).expect("compare");
        let top = top_of(&shared.read().expect("read"), View::Compare, summary.arena, SizeBasis::Logical, 10).expect("top");
        assert_eq!(top.grew.growers.len(), 1, "the growth is still named");
        assert!(top.pace.is_none(), "an hour times twenty-four is not a pace");
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

    /// Writes `bytes` of text at `path`, last written `days` ago.
    fn written_ago(path: &Path, bytes: usize, days: u64) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the parent");
        }
        std::fs::write(path, "an old line of text, said again. ".repeat(bytes / 33 + 1)).expect("write");
        let then = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_times(std::fs::FileTimes::new().set_modified(then)))
            .expect("date the file");
    }

    /// Dates the folder at `path` as last written `days` ago: a folder is
    /// written whenever an entry in it is added, so it is dated after them.
    fn folder_ago(path: &Path, days: u64) {
        let then = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        let mut options = std::fs::File::options();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_WRITE_ATTRIBUTES, and FILE_FLAG_BACKUP_SEMANTICS, without
            // which Windows opens no folder.
            options.access_mode(0x100).custom_flags(0x0200_0000);
        }
        #[cfg(not(windows))]
        options.read(true);
        options
            .open(path)
            .and_then(|folder| folder.set_times(std::fs::FileTimes::new().set_modified(then)))
            .expect("date the folder");
    }

    #[test]
    fn the_layers_of_a_folder_are_its_strata_and_its_fossils_and_the_old_view_counts_the_same() {
        let dir = tempfile::tempdir().expect("a folder");
        let root = dir.path();
        written_ago(&root.join("old-project/src/main.rs"), 40_000, 900);
        written_ago(&root.join("old-project/notes.txt"), 10_000, 1000);
        written_ago(&root.join("live/today.txt"), 5_000, 0);
        written_ago(&root.join("live/archive/2020.txt"), 20_000, 2000);
        folder_ago(&root.join("old-project/src"), 900);
        folder_ago(&root.join("old-project"), 900);
        folder_ago(&root.join("live/archive"), 2000);
        let shared = opened(index_of(root));
        let found = nothing_found();

        let summary = layers_of(&shared, &found, 0, SizeBasis::Logical, 10, SystemTime::now()).expect("the layers");
        let paths: Vec<String> = summary
            .fossils
            .iter()
            .map(|fossil| fossil.path.replace('\\', "/").rsplit('/').next().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(paths, ["old-project", "archive"], "the old project whole; of the live folder, its archive");
        let old: u64 = summary
            .strata
            .iter()
            .filter(|layer| layer.stratum.is_fossil())
            .map(|layer| layer.size.logical)
            .sum();
        assert_eq!(old, summary.size.logical, "the old strata and the fossils are one count");

        // The view of what is untouched: the same tree, by its old bytes.
        let state = shared.read().expect("read");
        let tree = state.index.as_ref().expect("an index").tree();
        let untouched = untouched_for(&state, &found, View::Old, tree).expect("worked out").expect("the old view");
        assert_eq!(untouched.size(ROOT).logical, summary.size.logical);
        let sort = Sort {
            key: midda_core::SortKey::Logical,
            direction: midda_core::Direction::Descending,
        };
        let live = tree.child(ROOT, "live").expect("live");
        let (page, total) = untouched.children_page(tree, live, sort, SizeBasis::Logical, Span::default());
        assert_eq!((total, tree.node(page[0]).name.as_str()), (1, "archive"), "today's file is not in the old view");
        assert!(untouched_for(&state, &found, View::Index, tree).expect("asked").is_none());
    }

    #[test]
    #[cfg(windows)]
    fn a_fossil_compressed_is_read_again_and_says_what_came_back() {
        // Not in the system's temporary folder: a scan inside it is Windows'
        // by the places above it, as on a scan of the volume, and Windows'
        // files are not offered for compression. The build's own folder is
        // nobody's.
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target");
        std::fs::create_dir_all(&target).expect("the build's folder");
        let dir = tempfile::tempdir_in(&target).expect("a folder");
        let root = dir.path();
        if compress::refused_on(root).is_some() {
            return;
        }
        written_ago(&root.join("cold/a.txt"), 300_000, 800);
        written_ago(&root.join("cold/b.txt"), 200_000, 900);
        folder_ago(&root.join("cold"), 800);
        let shared = opened(index_of(root));
        let found = nothing_found();
        let cold = {
            let state = shared.read().expect("read");
            state.index.as_ref().expect("an index").tree().child(ROOT, "cold").expect("cold")
        };
        let version = shared.read().expect("read").version;

        let compressed = compress_in(&shared, &found, 0, cold).expect("compressed");
        assert_eq!(compressed.done.files, 2);
        assert!(
            compressed.after.allocated < compressed.before.allocated / 2,
            "text compresses: {:?} -> {:?}",
            compressed.before,
            compressed.after
        );
        assert_eq!(compressed.after.logical, compressed.before.logical, "nothing was deleted");
        let state = shared.read().expect("read");
        assert!(state.version > version, "the window is told the tree changed");
        assert!(state.compressing.is_none());
        drop(state);

        // Still a fossil - its age did not move - and now there is nothing
        // in it left to compress.
        let summary = layers_of(&shared, &nothing_found(), 0, SizeBasis::Allocated, 10, SystemTime::now()).expect("the layers");
        assert_eq!(summary.fossils.len(), 1);
        assert_eq!(summary.fossils[0].not_compressed, Some(Why::Compressed));
        // Read again whole, the folder is a new entry of the tree: the window
        // finds it again by its path, as after a cleanup.
        assert_eq!(compress_in(&shared, &found, 0, cold).expect_err("the old id"), "stale");
        let again = {
            let state = shared.read().expect("read");
            state.index.as_ref().expect("an index").tree().child(ROOT, "cold").expect("cold")
        };
        assert_eq!(compress_in(&shared, &found, 0, again).expect_err("nothing left"), "it is compressed already");
    }

    #[test]
    fn opening_another_folder_keeps_the_readers_rules() {
        let own = vec![("mine.toml".to_owned(), "[[keep]]\npaths = [\"x\"]\n".to_owned())];
        let (rules, refused) = RuleSet::with_own(&own, &midda_core::rules::pattern::Known::in_profiles());
        assert!(refused.is_empty());
        let mut state = State {
            rules: crate::own::Rules(Arc::new(rules)),
            own: vec![crate::own::RulesFileRow {
                name: "mine.toml".to_owned(),
                rules: 0,
                places: 0,
                keeps: 1,
                refused: None,
            }],
            ..State::default()
        };
        let before = Arc::clone(&state.rules.0);
        open_anew(&mut state, Path::new("C:/other"));
        assert!(Arc::ptr_eq(&state.rules.0, &before), "the same set reads the next folder");
        assert_eq!(state.own.len(), 1);
        assert_eq!((state.arena, state.root.as_deref()), (1, Some(Path::new("C:/other"))));
    }
}
