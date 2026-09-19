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
}

/** What a scan looks like from outside. Mirrors `ScanState`. */
export interface ScanState {
  running: boolean
  entries: number
  bytes: number
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
