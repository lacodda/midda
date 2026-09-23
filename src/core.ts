/**
 * The door to midda-core. Every shape the window receives is declared here, and
 * every call goes through here — a component that reached for `invoke` directly
 * would be a second place that knows what a byte means.
 */

import { invoke } from '@tauri-apps/api/core'

/** Which of the two sizes the window is showing. */
export type SizeBasis = 'allocated' | 'logical'

/** What a column is ordered by. Mirrors `SortKey` in midda-core. */
export type SortKey = 'allocated' | 'logical' | 'name' | 'modified' | 'entries'

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

/** One row of the table. Mirrors `Row` in src-tauri/src/scan.rs. */
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
}

/** One page of an ordered list. Mirrors `Page`. */
export interface Page {
  rows: Row[]
  /** Where these rows start in the whole list. */
  offset: number
  /** How many rows the list holds, drawn or not. */
  total: number
}

/** A finished scan. Mirrors `ScanResult`. */
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
  /** How long the scan took. */
  elapsedMs: number
}

/** What a scan looks like from outside. Mirrors `ScanState`. */
export interface ScanState {
  running: boolean
  entries: number
  bytes: number
  /** MFT records read so far; zero on the walk. */
  records: number
  result: ScanResult | null
  error: string | null
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

export function startScan(path: string): Promise<void> {
  return invoke<void>('start_scan', { path })
}

export function scanProgress(): Promise<ScanState> {
  return invoke<ScanState>('scan_progress')
}

export function cancelScan(): Promise<void> {
  return invoke<void>('cancel_scan')
}

export function listChildren(id: number, sort: Sort, span: Span): Promise<Page> {
  return invoke<Page>('list_children', { id, sort, span })
}

export function trailTo(id: number): Promise<Row[]> {
  return invoke<Row[]>('trail_to', { id })
}

export function treemap(id: number, layout: Layout): Promise<Tile[]> {
  return invoke<Tile[]>('treemap', { id, layout })
}

/** Picks the size a basis names out of a row. */
export function sizeOf(row: Row, basis: SizeBasis): number {
  return basis === 'allocated' ? row.allocated : row.logical
}

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
