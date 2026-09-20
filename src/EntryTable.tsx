import { useCallback, useEffect, useRef, useState } from 'react'

import { Badge } from '@/components/ui/badge'
import { TableSortHeader } from '@/components/ui/table'
import { windowFor } from '@/components/ui/virtual-list'
import { hasTrait, isSharedName, listChildren, sizeOf, TRAIT, type Page, type Row, type SizeBasis, type Sort, type SortKey } from '@/core'
import { explainSize, formatAge, formatBytes, formatCount, formatShare } from '@/format'

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
 * template, shared by the header row and every body row, is what keeps them in
 * line. It is also why the body rows are a list of grids rather than a
 * `<tbody>`: the browser cannot be asked to lay out rows it cannot see.
 */
const GRID = 'minmax(0,1fr) 5rem 4.5rem 7rem 6rem 6rem'

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

interface EntryTableProps {
  /** The folder whose children are shown. */
  parentId: number
  /** That folder's total on the current basis, for the share column. */
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
export function EntryTable({ parentId, total, sort, basis, selectedId, onSortChange, onDescend, onSelect }: EntryTableProps) {
  const viewport = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(0)
  const [count, setCount] = useState(0)
  /** The rows that have arrived, by their index in the ordered list. */
  const [rows, setRows] = useState<Map<number, Row>>(new Map())
  const [failure, setFailure] = useState<string | null>(null)

  // Which request is current. A reader who clicks two headers quickly has two
  // pages in flight, and the slower one must not land on top of the newer.
  const generation = useRef(0)

  const measure = useCallback(() => {
    const element = viewport.current
    if (!element) return
    setViewportHeight(element.clientHeight)
    setScrollTop(element.scrollTop)
  }, [])

  useEffect(() => {
    const element = viewport.current
    if (!element) return
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [measure])

  const { start, end, totalHeight, offsetTop } = windowFor({
    count,
    rowHeight: ROW_HEIGHT,
    viewportHeight,
    scrollTop,
    overscan: OVERSCAN,
  })

  // Fetch whatever part of the visible window has not arrived. The first pass
  // runs with `count` at zero, which asks for one page and learns the real
  // total from it.
  useEffect(() => {
    const from = count === 0 ? 0 : start
    const to = count === 0 ? OVERSCAN * 4 : end
    if (to <= from && count !== 0) return

    let missing = false
    for (let index = from; index < to; index += 1) {
      if (!rows.has(index)) {
        missing = true
        break
      }
    }
    if (!missing && count !== 0) return

    const mine = generation.current
    void listChildren(parentId, sort, { offset: from, limit: Math.max(to - from, 1) })
      .then((page: Page) => {
        // A page from a folder or an order the reader has already left.
        if (mine !== generation.current) return
        setCount(page.total)
        setRows((held) => {
          const next = new Map(held)
          page.rows.forEach((row, at) => next.set(page.offset + at, row))
          return next
        })
      })
      .catch((cause: unknown) => {
        if (mine !== generation.current) return
        setFailure(String(cause))
      })
  }, [parentId, sort, start, end, count, rows])

  if (failure !== null) {
    return <p className="p-4 text-sm text-bad">{failure}</p>
  }

  const drawn: (Row | undefined)[] = []
  for (let index = start; index < end; index += 1) drawn.push(rows.get(index))

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <table className="w-full table-fixed border-collapse text-left text-sm">
        <thead className="border-b border-line text-xs text-dim">
          <tr style={{ display: 'grid', gridTemplateColumns: GRID }} className="h-8 items-center">
            {COLUMNS.map((column) =>
              column.key === null ? (
                <th
                  key={column.label}
                  scope="col"
                  className="flex items-center justify-end overflow-hidden whitespace-nowrap px-3 font-medium"
                >
                  {column.label}
                </th>
              ) : (
                <TableSortHeader
                  key={column.label}
                  column={column.key}
                  numeric={column.numeric}
                  sort={{ column: sort.key, direction: sort.direction === 'ascending' ? 'asc' : 'desc' }}
                  onSortChange={(key) => onSortChange(key as SortKey)}
                  className="flex items-center overflow-hidden whitespace-nowrap"
                >
                  {column.label}
                </TableSortHeader>
              ),
            )}
          </tr>
        </thead>
      </table>

      <div
        ref={viewport}
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        tabIndex={0}
        role="rowgroup"
        aria-label="Entries"
        className="min-h-0 flex-1 overflow-y-auto"
      >
        {count === 0 ? (
          <p className="px-3 py-6 text-sm text-dim">This folder is empty.</p>
        ) : (
          <div style={{ height: totalHeight }} className="relative">
            <div style={{ transform: `translateY(${offsetTop}px)` }}>
              {drawn.map((row, at) => {
                const index = start + at
                if (!row) {
                  // A row that has not arrived. Drawn as an empty line of the
                  // right height rather than skipped, so the list does not
                  // jump when it lands.
                  return <div key={`pending-${index}`} style={{ height: ROW_HEIGHT }} aria-hidden />
                }

                const size = sizeOf(row, basis)
                const chosen = row.id === selectedId
                // Why this row's two numbers differ, when they do. A second
                // name for shared bytes is the one worth marking in the list
                // itself rather than only in a tooltip: it is the row whose
                // zero would otherwise read as a scanner that gave up.
                const explanation = explainSize(row.traits, row.links)
                // Marked when this row's own size would otherwise be a mystery:
                // a second name, or a folder whose contents are named elsewhere
                // and counted there. Both draw a zero the reader would
                // otherwise read as a scanner that gave up.
                const shared = isSharedName(row.traits) || hasTrait(row.traits, TRAIT.holdsShared)
                return (
                  <button
                    key={row.id}
                    type="button"
                    role="row"
                    aria-selected={chosen}
                    /* One click highlights, and the picture beside this
                     * highlights with it; a second on a folder goes in. A
                     * single click that descended would make the two views
                     * impossible to point at the same thing, because pointing
                     * at it would have left it. */
                    onClick={() => onSelect(row)}
                    onDoubleClick={() => onDescend(row)}
                    title={explanation === null ? row.path : `${row.path}\n${explanation}`}
                    style={{ display: 'grid', gridTemplateColumns: GRID, height: ROW_HEIGHT }}
                    className={
                      chosen
                        ? 'w-full cursor-pointer items-center bg-accent-soft text-left'
                        : row.isDirectory
                          ? 'w-full cursor-pointer items-center text-left hover:bg-soft'
                          : 'w-full cursor-pointer items-center text-left hover:bg-soft'
                    }
                  >
                    <span role="gridcell" className="flex min-w-0 items-center gap-2 px-3">
                      {/* The bar is the row's share of the folder, so a glance
                          down the column is the shape of what is here. */}
                      <span className="h-1 w-10 shrink-0 overflow-hidden rounded-full bg-soft" aria-hidden>
                        <span className="block h-full bg-accent" style={{ width: total > 0 ? `${(size / total) * 100}%` : '0%' }} />
                      </span>
                      <span className={row.isDirectory ? 'truncate font-medium' : 'truncate text-dim'}>
                        {row.name}
                        {row.isDirectory && <span className="text-dim">/</span>}
                      </span>
                      {/* A word on the row rather than only in the tooltip: the
                          reader is scanning a column of sizes for something to
                          delete, and a zero with no reason beside it is the one
                          row they will act on wrongly. Only the second name is
                          marked here — the name holding the bytes is an
                          ordinary large row, and labelling it too would put a
                          badge on half the list. */}
                      {shared && (
                        <Badge variant="soft" className="shrink-0 px-1.5 py-0 text-[0.625rem] leading-4" title={explanation ?? undefined}>
                          linked
                        </Badge>
                      )}
                    </span>
                    <span role="gridcell" className="tabular truncate px-3 text-right text-xs text-dim">
                      {row.isDirectory ? formatCount(row.entries) : ''}
                    </span>
                    <span role="gridcell" className="tabular truncate px-3 text-right text-xs text-dim">
                      {formatShare(size, total)}
                    </span>
                    <span role="gridcell" className="tabular truncate px-3 text-right">
                      {formatBytes(row.allocated)}
                    </span>
                    <span role="gridcell" className="tabular truncate px-3 text-right text-dim">
                      {formatBytes(row.logical)}
                    </span>
                    <span role="gridcell" className="tabular truncate px-3 text-right text-xs text-dim">
                      {formatAge(row.subtreeModified)}
                    </span>
                  </button>
                )
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
