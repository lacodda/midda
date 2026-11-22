import { useEffect, useRef, useState } from 'react'

import { Badge } from '@/components/ui/badge'
import { TableSortButton } from '@/components/ui/table'
import { ariaSort } from '@/components/ui/table-sort'
import { VirtualGrid, VirtualGridCell, VirtualGridColumn } from '@/components/ui/virtual-grid'
import {
  growthOf,
  hasTrait,
  isSharedName,
  isStale,
  listChildren,
  pick,
  sizeOf,
  TRAIT,
  type Page,
  type Row,
  type SizeBasis,
  type Sort,
  type SortKey,
  type View,
} from '@/core'
import { explainSize, formatAge, formatBytes, formatCount, formatShare, formatSigned, MARK_WORDS } from '@/format'

/**
 * How tall one row is. Fixed, because virtualization is arithmetic over a known
 * height — measuring rows means rendering to find out, which means a second
 * pass and a scrollbar that changes length as the reader travels.
 */
const ROW_HEIGHT = 28

/** How many rows to fetch either side of the visible window. */
const OVERSCAN = 12

/**
 * The columns, and the one place their widths are written down.
 *
 * A `<table>` sizes its columns from what is in the DOM, and a virtualized body
 * holds one screenful — so the widths would shift on every scroll. One grid
 * template, shared by the heading and every row, is what keeps them in line:
 * dowel's VirtualGrid puts it on both. Written whole, as classes, so the
 * stylesheet has them.
 */
const GRID = 'grid-cols-[minmax(0,1fr)_5rem_4.5rem_7rem_6rem_6rem]'

/** The columns of a comparison: what each entry held then, holds now, and
 * the difference, all on the size being read. */
const COMPARED_GRID = 'grid-cols-[minmax(0,1fr)_5rem_6.5rem_6.5rem_7rem]'

interface Column {
  /** What clicking this header sorts by. A column with no key of its own is
   * not sortable and is drawn as a plain heading. */
  key: SortKey | null
  label: string
  numeric: boolean
}

const COLUMNS: Column[] = [
  { key: 'name', label: 'Name', numeric: false },
  { key: 'entries', label: 'Items', numeric: true },
  // The share is a share *of the column being read*, so sorting by it and
  // sorting by that size are the same order. Giving it the `allocated` key
  // would put the sort arrow on two headings at once and tell a screen reader
  // that two columns are sorted, which is not a thing a table can be.
  { key: null, label: 'Share', numeric: true },
  { key: 'allocated', label: 'On disk', numeric: true },
  { key: 'logical', label: 'Size', numeric: true },
  { key: 'modified', label: 'Changed', numeric: true },
]

/**
 * The columns of a comparison, on the size being read.
 *
 * "Before" has no key: an order by what something held then is an order by
 * a picture that is gone, and the question a comparison answers is what
 * grew. "Now" sorts by the size being read, as a size column does anywhere.
 * "Items" counts what changed inside, not what is inside.
 */
function comparedColumns(basis: SizeBasis): Column[] {
  return [
    { key: 'name', label: 'Name', numeric: false },
    { key: 'entries', label: 'Items', numeric: true },
    { key: null, label: 'Before', numeric: true },
    { key: basis, label: 'Now', numeric: true },
    { key: 'growth', label: 'Change', numeric: true },
  ]
}

interface EntryTableProps {
  /** Which tree the rows come from. */
  view: View
  /** Which reading of that tree the ids are from. */
  arena: number
  /** Moves whenever the tree changed: the rows on screen are asked for again,
   * where they are, without losing the place. */
  version: number
  /** The folder whose children are shown. */
  parentId: number
  /** That folder's total on the current basis, for the share column — in a
   * comparison, how much moved under it, which the bars are shares of. */
  total: number
  sort: Sort
  basis: SizeBasis
  /** The entry the picture beside this has highlighted, if any. */
  selectedId: number | null
  onSortChange: (key: SortKey) => void
  onDescend: (row: Row) => void
  /** A row was chosen without being entered — the picture follows. */
  onSelect: (row: Row | null) => void
}

/**
 * The children of one folder, sorted and virtualized.
 *
 * Rows arrive a page at a time from the core rather than all at once: a folder
 * on a system volume can hold hundreds of thousands, and the core is where the
 * comparison rules live — including the one that keeps rows with no timestamp
 * at the bottom whichever way the date column points.
 *
 * A new folder or a new order makes every row held meaningless, and the honest
 * way to say that in React is a new component: `Rows` gives this a key of the
 * folder and the sort, so changing either remounts it empty and scrolled to the
 * top. Clearing the state in an effect instead would be derived state kept by
 * hand — a cascading render, and one more place for the scroll position to
 * survive a change it should not have survived.
 */
export function EntryTable({ view, arena, version, parentId, total, sort, basis, selectedId, onSortChange, onDescend, onSelect }: EntryTableProps) {
  /** The rows drawn now, by index: what the grid says is in view. */
  const [range, setRange] = useState({ start: 0, end: 0 })
  const [count, setCount] = useState(0)
  /** The rows that have arrived, by their index in the ordered list. */
  const [rows, setRows] = useState<Map<number, Row>>(new Map())
  const [failure, setFailure] = useState<string | null>(null)

  // Which request is current. A reader who clicks two headers quickly has two
  // pages in flight, and the slower one must not land on top of the newer.
  const generation = useRef(0)
  const compared = view === 'compare'
  const grid = compared ? COMPARED_GRID : GRID
  const columns = compared ? comparedColumns(basis) : COLUMNS
  // Which version of the tree the rows held are from. A change to the tree
  // asks for the rows on screen again; until they land, the old ones stay
  // drawn rather than blinking out, and when they land they replace every
  // row held - the ones off screen are as old as the ones on it.
  const heldVersion = useRef(version)

  // Fetch whatever part of the rows in view has not arrived. The first pass
  // runs with `count` at zero, which asks for one page and learns the real
  // total from it.
  useEffect(() => {
    const from = count === 0 ? 0 : range.start
    const to = count === 0 ? OVERSCAN * 4 : range.end
    if (to <= from && count !== 0) return

    const outdated = heldVersion.current !== version
    let missing = outdated
    for (let index = from; index < to && !missing; index += 1) {
      if (!rows.has(index)) missing = true
    }
    if (!missing && count !== 0) return

    // Pages of the old tree still in flight are worthless once it changed;
    // pages of the same tree for another stretch of the list are not.
    if (outdated) generation.current += 1
    const mine = generation.current
    void listChildren(view, arena, parentId, sort, basis, { offset: from, limit: Math.max(to - from, 1) })
      .then((page: Page) => {
        // A page from a folder or an order the reader has already left.
        if (mine !== generation.current) return
        setCount(page.total)
        const replacing = heldVersion.current !== version
        heldVersion.current = version
        setRows((held) => {
          const next = replacing ? new Map<number, Row>() : new Map(held)
          page.rows.forEach((row, at) => next.set(page.offset + at, row))
          return next
        })
      })
      .catch((cause: unknown) => {
        if (mine !== generation.current) return
        // The tree was read again, or this folder deleted: the window is
        // finding its place again, and this table is about to be replaced.
        if (isStale(cause)) return
        setFailure(String(cause))
      })
  }, [view, arena, version, parentId, sort, basis, range, count, rows])

  if (failure !== null) {
    return <p className="p-4 text-sm text-bad">{failure}</p>
  }

  // The picture chooses by entry, the grid by place: the chosen entry is
  // chosen here once its row has arrived, and brought into view then.
  let selected: number | null = null
  if (selectedId !== null) {
    for (const [index, row] of rows) {
      if (row.id === selectedId) {
        selected = index
        break
      }
    }
  }
  const sorted = { column: sort.key, direction: sort.direction === 'ascending' ? 'asc' : 'desc' } as const

  const header = columns.map((column) =>
    column.key === null ? (
      <VirtualGridColumn key={column.label} className="px-3 text-right">
        {column.label}
      </VirtualGridColumn>
    ) : (
      <VirtualGridColumn key={column.label} aria-sort={ariaSort(sorted, column.key)} className={column.numeric ? 'px-3 text-right' : 'px-3'}>
        <TableSortButton
          column={column.key}
          sort={sorted}
          onSortChange={(key) => onSortChange(key as SortKey)}
          // A button takes no case from the heading it is in (the stylesheet's
          // reset), and the grid's headings are set in capitals.
          className={column.numeric ? 'w-[calc(100%+0.5rem)] justify-end uppercase' : 'w-[calc(100%+0.5rem)] uppercase'}
        >
          {column.label}
        </TableSortButton>
      </VirtualGridColumn>
    ),
  )

  const renderRow = (index: number) => {
    const row = rows.get(index)
    // A row that has not arrived: drawn empty at its height, so the list
    // does not jump when it lands.
    if (!row) return null

    // In a comparison the bar is the row's share of what moved in the
    // folder, as its tile in the picture is; in the view of what is
    // untouched, of what of the folder is.
    const size = row.compared !== null ? pick(row.compared.moved, basis) : row.untouched !== null ? pick(row.untouched, basis) : sizeOf(row, basis)
    // The two size columns of the view of what is untouched are what of the
    // row is: the folder by its old bytes alone.
    const shown = row.untouched ?? { allocated: row.allocated, logical: row.logical }
    // Why this row's two numbers differ, when they do. A second name for
    // shared bytes is the one worth marking in the list itself rather than
    // only in a tooltip: it is the row whose zero would otherwise read as a
    // scanner that gave up.
    const explanation = explainSize(row.traits, row.links)
    // Marked when this row's own size would otherwise be a mystery: a second
    // name, or a folder whose contents are named elsewhere and counted there.
    // Both draw a zero the reader would otherwise read as a scanner that gave
    // up.
    const shared = isSharedName(row.traits) || hasTrait(row.traits, TRAIT.holdsShared)
    return (
      <>
        <VirtualGridCell className="flex items-center gap-2 px-3" title={explanation === null ? row.path : `${row.path}\n${explanation}`}>
          {/* The bar is the row's share of the folder, so a glance down the
              column is the shape of what is here. */}
          <span className="h-1 w-10 shrink-0 overflow-hidden rounded-full bg-soft" aria-hidden>
            <span className="block h-full bg-accent" style={{ width: total > 0 ? `${(size / total) * 100}%` : '0%' }} />
          </span>
          <span className={row.isDirectory ? 'truncate font-medium' : 'truncate text-dim'}>
            {row.name}
            {row.isDirectory && <span className="text-dim">/</span>}
          </span>
          {/* A word on the row rather than only in the tooltip: the reader is
              scanning a column of sizes for something to delete, and a zero
              with no reason beside it is the one row they will act on
              wrongly. Only the second name is marked here — the name holding
              the bytes is an ordinary large row, and labelling it too would
              put a badge on half the list. */}
          {shared && (
            <Badge variant="soft" className="shrink-0 px-1.5 py-0 text-2xs leading-4" title={explanation ?? undefined}>
              linked
            </Badge>
          )}
          {/* In the view of what changed, the word that keeps a size honest:
              a file written to holds this much now, and grew by an amount
              nothing recorded. */}
          {row.change !== null && (
            <Badge
              variant="soft"
              className="shrink-0 px-1.5 py-0 text-2xs leading-4"
              title={row.change === 'created' ? 'Did not exist at the start of the span' : 'Existed before, and was written to since'}
            >
              {row.change === 'created' ? 'new' : 'written'}
            </Badge>
          )}
          {/* In a comparison, what happened to the entry as a whole. A file
              that only changed size carries no word: the change column is the
              word. */}
          {row.compared !== null && MARK_WORDS[row.compared.mark] !== null && (
            <Badge variant="soft" className="shrink-0 px-1.5 py-0 text-2xs leading-4" title={MARK_TITLES[row.compared.mark]}>
              {MARK_WORDS[row.compared.mark]}
            </Badge>
          )}
        </VirtualGridCell>
        <VirtualGridCell className="tabular px-3 text-right text-xs text-dim">{row.isDirectory ? formatCount(row.entries) : ''}</VirtualGridCell>
        {row.compared !== null && <ComparedCells row={row} basis={basis} />}
        {row.compared === null && (
          <>
            <VirtualGridCell className="tabular px-3 text-right text-xs text-dim">{formatShare(size, total)}</VirtualGridCell>
            <VirtualGridCell className="tabular px-3 text-right">{formatBytes(shown.allocated)}</VirtualGridCell>
            <VirtualGridCell className="tabular px-3 text-right text-dim">{formatBytes(shown.logical)}</VirtualGridCell>
            <VirtualGridCell className="tabular px-3 text-right text-xs text-dim">{formatAge(row.subtreeModified)}</VirtualGridCell>
          </>
        )}
      </>
    )
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <VirtualGrid
        label="Entries"
        count={count}
        rowHeight={ROW_HEIGHT}
        columns={grid}
        header={header}
        renderRow={renderRow}
        rowKey={(index) => rows.get(index)?.id ?? `pending-${index}`}
        selected={selected}
        /* One click highlights, and the picture beside this highlights with
         * it; a second on a folder, or Enter, goes in. A single click that
         * descended would make the two views impossible to point at the same
         * thing, because pointing at it would have left it. */
        onSelectedChange={(index) => onSelect(rows.get(index) ?? null)}
        onActivate={(index) => {
          const row = rows.get(index)
          if (row) onDescend(row)
        }}
        onRangeChange={(from, to) => setRange({ start: from, end: to })}
        overscan={OVERSCAN}
        className={count === 0 ? 'shrink-0' : 'min-h-0 flex-1'}
      />
      {count === 0 && (
        <p className="px-3 py-6 text-sm text-dim">
          {compared ? 'Nothing in this folder changed size.' : view === 'old' ? 'Nothing in this folder has gone a year untouched.' : 'This folder is empty.'}
        </p>
      )}
    </div>
  )
}

/** What each mark on a row of a comparison means, for its tooltip. */
const MARK_TITLES = {
  new: 'Was not there in the earlier picture',
  gone: 'Is not there any more',
  replaced: 'A file stands where a folder stood, or a folder where a file did',
  changed: '',
} as const

/**
 * The three numbers of a comparison's row: before, now, and the change.
 *
 * The change is coloured the way the picture beside it is - growth in the
 * colour that means trouble, what was freed in the one that means relief -
 * so a glance down the column and a glance at the picture agree.
 */
function ComparedCells({ row, basis }: { row: Row; basis: SizeBasis }) {
  const growth = growthOf(row, basis) ?? 0
  const before = row.compared === null ? 0 : pick(row.compared.before, basis)
  return (
    <>
      <VirtualGridCell className="tabular px-3 text-right text-dim">{formatBytes(before)}</VirtualGridCell>
      <VirtualGridCell className="tabular px-3 text-right">{formatBytes(sizeOf(row, basis))}</VirtualGridCell>
      <VirtualGridCell className={`tabular px-3 text-right font-medium ${growth > 0 ? 'text-bad' : growth < 0 ? 'text-good' : 'text-dim'}`}>
        {formatSigned(growth)}
      </VirtualGridCell>
    </>
  )
}
