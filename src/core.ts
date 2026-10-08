/**
 * The door to midda-core. Every shape the window receives is declared here, and
 * every call goes through here — a component that reached for `invoke` directly
 * would be a second place that knows what a byte means.
 */

import { invoke } from '@tauri-apps/api/core'

/** Which of the two sizes the window is showing. */
export type SizeBasis = 'allocated' | 'logical'

/** What a column is ordered by. Mirrors `SortKey` in midda-core. `growth`
 * is known only in a comparison; in any other tree every row is unknown. */
export type SortKey = 'allocated' | 'logical' | 'name' | 'modified' | 'entries' | 'growth'

/** Which way a column points. Mirrors `Direction`. */
export type Direction = 'ascending' | 'descending'

/** A column and the way it points. Mirrors `Sort`. */
export interface Sort {
  key: SortKey
  direction: Direction
}

/** The slice of an ordered list the window is drawing. Mirrors `Span`. */
export interface Span {
  offset: number
  limit: number
}

/**
 * Why an entry's two sizes differ. Mirrors `Traits` in midda-core.
 *
 * Bits rather than an enum, because a file can be several of these at once: a
 * cloud placeholder is sparse as well, and a sparse file can have a second
 * name. The values are the core's and must not be reordered — they cross the
 * bridge as a number.
 */
export const TRAIT = {
  /** The bytes live in the cloud; the file is a stub until something opens it. */
  placeholder: 1 << 0,
  /** NTFS is storing the contents compressed. */
  compressed: 1 << 1,
  /** The file has holes: ranges that read as zeroes and occupy nothing. */
  sparse: 1 << 2,
  /** The same bytes are reachable under another name on this volume. */
  linked: 1 << 3,
  /** This is the name those shared bytes were counted under. */
  countedHere: 1 << 4,
  /** Somewhere beneath this directory is a name for bytes counted elsewhere. */
  holdsShared: 1 << 5,
} as const

/** Whether `traits` carries `trait`. */
export function hasTrait(traits: number, bit: number): boolean {
  return (traits & bit) === bit
}

/**
 * Whether this entry is a second name for bytes counted elsewhere.
 *
 * The row that shows zero is the one wanting an explanation, not the row that
 * shows the bytes — so the polarity here is the whole point of the function.
 */
export function isSharedName(traits: number): boolean {
  return hasTrait(traits, TRAIT.linked) && !hasTrait(traits, TRAIT.countedHere)
}

/** How a file came to be in the view of what changed. Mirrors `Change`. */
export type Change = 'created' | 'written'

/**
 * Which tree a question is about: the folder as it is, what was written in
 * it lately, or two pictures of it compared. Mirrors `View` in
 * src-tauri/src/session.rs.
 */
export type View = 'index' | 'changes' | 'compare'

/** Both sizes of something. Mirrors `Size` in midda-core. */
export interface Size {
  logical: number
  allocated: number
}

/** How an entry came to be in a comparison. Mirrors `Mark`. */
export type Mark = 'new' | 'gone' | 'changed' | 'replaced'

/** What a row of a comparison carries: the other moment. Mirrors `ComparedRow`. */
export interface ComparedRow {
  /** What it held then; what it holds now is the row's own size. */
  before: Size
  /** How many bytes moved under it: grown and freed, added. */
  moved: Size
  mark: Mark
}

/** One row of the table. Mirrors `Row` in src-tauri/src/session.rs. */
export interface Row {
  id: number
  name: string
  /** The bytes the volume gives up if this goes away. midda's default. */
  allocated: number
  /** The bytes a reader would get out of it. */
  logical: number
  isDirectory: boolean
  entries: number
  /** Milliseconds since the Unix epoch, or `null` when the filesystem would not say. */
  subtreeModified: number | null
  path: string
  /** Why the two sizes differ, as the bits of `TRAIT`. */
  traits: number
  /** How many names these bytes have, or `null` when it could not be asked. */
  links: number | null
  /** In the view of what changed, whether the file is new or was written to. */
  change: Change | null
  /** In a comparison, what it held then and how much moved under it. */
  compared: ComparedRow | null
}

/** One page of an ordered list. Mirrors `Page`. */
export interface Page {
  rows: Row[]
  /** Where these rows start in the whole list. */
  offset: number
  /** How many rows the list holds, drawn or not. */
  total: number
}

/** The way down to one entry, and the ids it was given in. Mirrors `Trail`. */
export interface Trail {
  arena: number
  rows: Row[]
}

/** A picture on screen. Mirrors `ScanResult`. */
export interface ScanResult {
  root: Row
  clusterBytes: number | null
  skipped: number
  /** How many entries were second names for bytes counted elsewhere. */
  sharedNames: number
  /** The bytes those second names would have been counted as, and are not. */
  sharedReclaimed: number
  /** Which scanner read this: `walk` or `mft`. */
  scannedBy: string
  /** Why the MFT reader gave way to the walk, when it did. */
  fallback: string | null
  /** How long the last full read took; zero for an index read back from disk. */
  elapsedMs: number
  /** When the folder was last read in full, as milliseconds since the epoch. */
  readAt: number | null
}

/** How the picture on screen is kept current. Mirrors `Mode`. */
export type Mode = 'none' | 'journal' | 'watching' | 'still'

/** What the window says about how current its picture is. Mirrors `Freshness`. */
export interface Freshness {
  mode: Mode
  /** The picture is the index saved at this moment, not yet brought up to date. */
  savedAt: number | null
  /** The change journal is being read from where the saved index left off. */
  catchingUp: boolean
  /** Why the picture is as current as it is, when that needs saying. */
  note: string | null
}

/** The open folder from outside. Mirrors `SessionState`. */
export interface SessionState {
  /** A full read is running — the first, or one under the picture on screen. */
  reading: boolean
  entries: number
  bytes: number
  /** MFT records read so far; zero on the walk. */
  records: number
  result: ScanResult | null
  error: string | null
  freshness: Freshness
  /** When a change was last applied. */
  appliedAt: number | null
  /** Moves with every change to what is on screen. */
  version: number
  /** Moves when the tree was read anew and every id held before means something else. */
  arena: number
}

/** What was written lately. Mirrors `ChangesSummary`. */
export interface ChangesSummary {
  root: Row
  arena: number
  created: number
  written: number
  deleted: number
  since: number | null
  /** The oldest change the journal holds. */
  reachesBackTo: number | null
  /** Whether the journal covers the whole span asked about. */
  covers: boolean
}

/** A picture of the folder kept for later. Mirrors `Kind` in midda-core's
 * snapshot module: the folder as it was last closed, or one the reader took. */
export type SnapshotKind = 'last' | 'taken'

/** One snapshot of the open folder. Mirrors `SnapshotRow`. */
export interface SnapshotRow {
  /** Its file name, which is what it is asked for by. */
  name: string
  kind: SnapshotKind
  /** The moment it is a picture of. */
  at: number | null
  /** What it takes on disk. */
  bytes: number
  /** Why it cannot be compared, when it cannot. */
  unusable: string | null
}

/** The snapshots of the open folder. Mirrors `Snapshots`. */
export interface Snapshots {
  snapshots: SnapshotRow[]
  bytes: number
  /** Why there is no picture of last time from this opening, when it matters. */
  lastNote: string | null
  /** Why nothing can be taken or compared with now yet, when it cannot. */
  notNow: string | null
}

/** What a comparison adds up to. Mirrors `Totals`. */
export interface Totals {
  before: Size
  now: Size
  /** Everything written in between and still there. */
  grown: Size
  /** Everything that went in between: after a cleanup, what came back. */
  freed: Size
  new: number
  gone: number
  changed: number
}

/** A comparison on screen. Mirrors `ComparisonSummary`. */
export interface ComparisonSummary {
  root: Row
  arena: number
  /** The earlier moment. */
  then: number | null
  /** The later one. */
  now: number | null
  /** Whether the later picture is the folder as it is, not a snapshot. */
  toNow: boolean
  totals: Totals
}

/** What kind of place a report line is. Mirrors `PlaceKind`. */
export type PlaceKind = 'gone' | 'new' | 'replaced' | 'files'

/** One place in a report. Mirrors `PlaceRow`. */
export interface PlaceRow {
  id: number
  path: string
  isDirectory: boolean
  kind: PlaceKind
  freed: Size
  grown: Size
  entries: number
}

/** The places past the ones listed, added up. Mirrors `Rest`. */
export interface Rest {
  places: number
  bytes: number
}

/** The report of a comparison. Mirrors `Report`. */
export interface Report {
  arena: number
  freed: PlaceRow[]
  freedRest: Rest
  grown: PlaceRow[]
  grownRest: Rest
}

/** A rectangle in fractions of the box being drawn. Mirrors `Rect`. */
export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/** What a tile is made of, as far as the colour is concerned. Mirrors `Category`. */
export type Category = 'build' | 'source' | 'media' | 'other'

/** What a tile held at two moments, on the basis drawn. Mirrors `Delta`. */
export interface Delta {
  before: number
  now: number
}

/** One rectangle of a laid-out treemap. Mirrors `Tile`. */
export interface Tile {
  /** The node, or `null` for the tile that gathers what was too small to draw. */
  id: number | null
  name: string
  /** The bytes this tile's area is proportional to. */
  bytes: number
  rect: Rect
  isDirectory: boolean
  category: Category
  entries: number
  /** The full path, for the tooltip. Empty for the gathered remainder, which
   * stands for many paths and so has none. */
  path: string
  /** In a comparison, what it held then and holds now; there the area is
   * how much moved, and this says which way. */
  change: Delta | null
}

/** What the window asks the core to lay out. Mirrors `Layout`. */
export interface Layout {
  basis: SizeBasis
  /** How wide the box is relative to its height, so "square" means square on
   * screen rather than square in fraction space. */
  aspect: number
  /** The smallest fraction of the box a tile may take before it is gathered. */
  minArea: number
  maxTiles: number
}

/** A drive the user can point a scan at. Mirrors `Volume`. */
export interface Volume {
  path: string
  label: string
  total: number | null
  free: number | null
}

export function listVolumes(): Promise<Volume[]> {
  return invoke<Volume[]>('list_volumes')
}

/**
 * Where scans stand with the fast scanner. Mirrors `Acceleration` in
 * midda-core: `active` reads the MFT, `needs-elevation` could after a restart
 * as administrator, `unsupported` never can (not NTFS, not Windows).
 */
export type Acceleration = 'active' | 'needs-elevation' | 'unsupported'

/** Where `path` — or, with none, the system drive — stands with the fast scanner. */
export function acceleration(path: string | null): Promise<Acceleration> {
  return invoke<{ state: Acceleration }>('acceleration', { path }).then((answer) => answer.state)
}

/**
 * Restarts midda as an administrator, scanning `path`. Resolves only if the
 * restart failed to happen — on success this window closes — so a rejection
 * carries the reason to show.
 */
export function accelerate(path: string | null): Promise<void> {
  return invoke<void>('accelerate', { path })
}

/** The folder this window was started to scan, once; `null` after that. */
export function launchRequest(): Promise<string | null> {
  return invoke<string | null>('launch_request')
}

/** Opens `path`: its saved index at once if there is one, then kept current. */
export function startScan(path: string): Promise<void> {
  return invoke<void>('start_scan', { path })
}

export function scanProgress(): Promise<SessionState> {
  return invoke<SessionState>('scan_progress')
}

export function cancelScan(): Promise<void> {
  return invoke<void>('cancel_scan')
}

/** Reads the open folder again in full, keeping the picture until the new one is ready. */
export function rescan(): Promise<void> {
  return invoke<void>('rescan')
}

/**
 * Whether a refusal means the ids asked about are from a tree read before this
 * one, or name an entry since deleted. Not a failure to show: the window finds
 * its place again by path.
 */
export function isStale(cause: unknown): boolean {
  return cause === 'stale'
}

/** One page of the children of `id`. `basis` is what growth is read on, in
 * a comparison. */
export function listChildren(view: View, arena: number, id: number, sort: Sort, basis: SizeBasis, span: Span): Promise<Page> {
  return invoke<Page>('list_children', { view, arena, id, sort, basis, span })
}

export function trailTo(view: View, arena: number, id: number): Promise<Row[]> {
  return invoke<Row[]>('trail_to', { view, arena, id })
}

/** The way down to `path`, or to the deepest folder above it still there. */
export function trailToPath(view: View, path: string): Promise<Trail> {
  return invoke<Trail>('trail_to_path', { view, path })
}

export function treemap(view: View, arena: number, id: number, layout: Layout): Promise<Tile[]> {
  return invoke<Tile[]>('treemap', { view, arena, id, layout })
}

/** What was written in the last `hours`, from the change journal. */
export function changesSince(hours: number): Promise<ChangesSummary> {
  return invoke<ChangesSummary>('changes_since', { hours })
}

/** The snapshots of the open folder. */
export function listSnapshots(): Promise<Snapshots> {
  return invoke<Snapshots>('snapshots')
}

/** Takes a snapshot of the open folder as it is now. */
export function takeSnapshot(): Promise<SnapshotRow> {
  return invoke<SnapshotRow>('take_snapshot')
}

export function deleteSnapshot(name: string): Promise<void> {
  return invoke<void>('delete_snapshot', { name })
}

/** Compares the snapshot `from` with `to`, or with the folder as it is now
 * when `to` is `null`. Whichever is older is "then". */
export function compareSnapshots(from: string, to: string | null): Promise<ComparisonSummary> {
  return invoke<ComparisonSummary>('compare_snapshots', { from, to })
}

/** The report of the comparison in `arena`: `limit` places each way. */
export function comparisonReport(arena: number, basis: SizeBasis, limit: number): Promise<Report> {
  return invoke<Report>('comparison_report', { arena, basis, limit })
}

/** Picks the size a basis names out of a row. */
export function sizeOf(row: Row, basis: SizeBasis): number {
  return basis === 'allocated' ? row.allocated : row.logical
}

/** Picks the size a basis names out of a pair of sizes. */
export function pick(size: Size, basis: SizeBasis): number {
  return basis === 'allocated' ? size.allocated : size.logical
}

/** How much a row of a comparison grew on `basis` — negative for one that
 * shrank or went — or `null` outside a comparison. */
export function growthOf(row: Row, basis: SizeBasis): number | null {
  return row.compared === null ? null : sizeOf(row, basis) - pick(row.compared.before, basis)
}

/** The order a comparison opens in: what grew most, first. */
export const GROWTH_SORT: Sort = { key: 'growth', direction: 'descending' }

/** The order a freshly opened folder is shown in: biggest on disk first. */
export const DEFAULT_SORT: Sort = { key: 'allocated', direction: 'descending' }

/**
 * The sort a click on `key` produces, given what is sorted now.
 *
 * Mirrors `Sort::toggled` in the core, and is checked against it by
 * `src/sort.test.ts`: the window decides this locally so a header click does
 * not wait on a round trip, and two copies of a rule drift unless something
 * holds them together.
 */
export function toggleSort(sort: Sort, key: SortKey): Sort {
  if (sort.key === key) {
    return { key, direction: sort.direction === 'ascending' ? 'descending' : 'ascending' }
  }
  return { key, direction: naturalDirection(key) }
}

/** The direction a column is first shown in when it is picked. */
export function naturalDirection(key: SortKey): Direction {
  // "Which of these is big" and "what changed lately" are largest-first
  // questions; the alphabet runs one way.
  return key === 'name' ? 'ascending' : 'descending'
}
