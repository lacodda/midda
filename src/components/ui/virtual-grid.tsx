import {
  useCallback,
  useEffect,
  useEffectEvent,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
} from 'react'
import { cn } from 'dowel-ui'
import { scrollToShow, windowFor } from './virtual-list'

/*
 * VirtualGrid.
 *
 * Rows in columns, a hundred thousand of them, with one chosen: a commit
 * graph, the entries of a disk, a log. Only the rows in view are drawn, as in
 * VirtualList; what this adds is everything a list a person works through
 * needs and VirtualList, a list to read, does not have.
 *
 * - **A grid, with a cursor.** One tab stop; the arrows, PageUp, PageDown,
 *   Home and End move the chosen row, and Enter opens it. The grid names the
 *   chosen row with `aria-activedescendant`, and every move scrolls that row
 *   into view first - the least distance if it is near, a third of the way
 *   down if it is far - because the row a reader is told about has to exist.
 * - **Rows by index, not an array.** `count` says how many there are and
 *   `renderRow(index)` draws one, or answers `null` for a row not read yet,
 *   which is drawn empty at its height until it arrives. `onRangeChange` says
 *   which rows are in view, so a product reading a page at a time knows which
 *   pages to read.
 * - **A heading that stands.** `header` is a row of `VirtualGridColumn`s,
 *   sticky at the top, on the same grid template as every row (`columns`), so
 *   the two cannot drift apart.
 * - **A layer over the window.** `layer` is drawn over the rows in view, in a
 *   box exactly their size, in the same render as the rows - a canvas column
 *   redrawn with the scroll stays on the rows it belongs to.
 *
 * furca drew its commit graph this way and midda its entries, each from
 * VirtualList's arithmetic alone, before the set had this.
 */

export interface VirtualGridView {
  /** The first row drawn. */
  start: number
  /** One past the last row drawn. */
  end: number
  /** How tall a row is, so a layer can place what it draws. */
  rowHeight: number
}

export interface VirtualGridProps extends Omit<HTMLAttributes<HTMLDivElement>, 'children' | 'onKeyDown'> {
  /** What a screen reader calls the grid: "Commits", "Entries". */
  label: string
  /** How many rows there are, read or not. */
  count: number
  /** How tall one row is, in pixels. Fixed, and rows are held to it. */
  rowHeight: number
  /** The grid template the heading and every row share - a class such as
   * `grid-cols-[1fr_8rem_6rem]`. */
  columns: string
  /** The heading: `VirtualGridColumn`s, one per column. */
  header?: ReactNode
  /** The cells of a row - `VirtualGridCell`s - or `null` for a row not read
   * yet, which is drawn empty until it is. */
  renderRow: (index: number) => ReactNode
  /** A stable key per row. Without it rows are keyed by index, which is right
   * only when a row's place never changes. */
  rowKey?: (index: number) => string | number
  /** The chosen row, or `null` for none. Left out, the grid keeps its own. */
  selected?: number | null
  /** Told the row a click or the keyboard chose. */
  onSelectedChange?: (index: number) => void
  /** Enter on the chosen row, or a double click on a row: open what it
   * stands for. */
  onActivate?: (index: number) => void
  /** Told which rows are drawn whenever that changes: `start` to `end`, the
   * end not included. */
  onRangeChange?: (start: number, end: number) => void
  /** Drawn over the rows in view, in a box exactly their size and hidden from
   * readers: a canvas the product redraws for the window it is given. */
  layer?: (view: VirtualGridView) => ReactNode
  /** How many rows to draw beyond each edge of the view. */
  overscan?: number
}

export function VirtualGrid({
  label,
  count,
  rowHeight,
  columns,
  header,
  renderRow,
  rowKey,
  selected: selectedProp,
  onSelectedChange,
  onActivate,
  onRangeChange,
  layer,
  overscan = 6,
  className,
  ...props
}: VirtualGridProps) {
  const base = useId()
  const box = useRef<HTMLDivElement>(null)
  const head = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(0)
  const [headerHeight, setHeaderHeight] = useState(0)

  const [own, setOwn] = useState<number | null>(null)
  const chosen = selectedProp === undefined ? own : selectedProp
  // A count that shrank under the chosen row leaves nothing chosen.
  const current = chosen !== null && chosen >= 0 && chosen < count ? chosen : null

  const measure = useCallback(() => {
    const element = box.current
    if (element === null) return
    setViewportHeight(element.clientHeight)
    setScrollTop(element.scrollTop)
    setHeaderHeight(head.current?.offsetHeight ?? 0)
  }, [])
  useEffect(() => {
    const element = box.current
    if (element === null) return
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [measure])

  // The heading stands over the top of the box, so the rows have the rest.
  const room = Math.max(0, viewportHeight - headerHeight)
  const { start, end, totalHeight, offsetTop } = windowFor({ count, rowHeight, viewportHeight: room, scrollTop, overscan })

  // Told after the rows are drawn, so a product's read lands on rows that
  // are on screen. In a layout effect, before paint, so a page read from a
  // cache fills the rows in the same frame. Only when the window moves: a
  // callback written inline is new on every render, and calling it each
  // time would loop a product that sets state from it.
  const tell = useEffectEvent((from: number, to: number) => onRangeChange?.(from, to))
  useLayoutEffect(() => {
    tell(start, end)
  }, [start, end])

  /* Bring a row into view: the row `aria-activedescendant` names has to be
   * in the document. */
  const reveal = (index: number) => {
    const element = box.current
    if (element === null) return
    const next = scrollToShow({ index, rowHeight, scrollTop: element.scrollTop, viewportHeight: Math.max(0, element.clientHeight - headerHeight) })
    if (next === element.scrollTop) return
    element.scrollTop = next
    setScrollTop(next)
  }

  // A row chosen from outside - the commit a branch points at, the entry a
  // search found - is brought into view as well, once each time it changes.
  const revealChosen = useEffectEvent(() => {
    if (current !== null) reveal(current)
  })
  useEffect(() => {
    revealChosen()
  }, [selectedProp])

  const choose = (index: number) => {
    const at = Math.min(Math.max(0, index), count - 1)
    if (at < 0) return
    if (selectedProp === undefined) setOwn(at)
    onSelectedChange?.(at)
    reveal(at)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // A page is a screenful less one row, so the row at the edge stays in
    // sight as the reference the reader paged from.
    const page = Math.max(1, Math.floor(room / rowHeight) - 1)
    const from = current ?? -1
    const steps: Record<string, number> = {
      ArrowDown: from + 1,
      ArrowUp: current === null ? 0 : from - 1,
      PageDown: from + page,
      PageUp: current === null ? 0 : from - page,
      Home: 0,
      End: count - 1,
    }
    if (event.key in steps) {
      event.preventDefault()
      choose(steps[event.key]!)
    } else if (event.key === 'Enter' && current !== null && onActivate) {
      event.preventDefault()
      onActivate(current)
    }
  }

  const rowId = (index: number) => `${base}-${index}`
  const headed = header !== undefined
  const rows = Array.from({ length: end - start }, (_, offset) => {
    const index = start + offset
    const cells = renderRow(index)
    const isChosen = index === current
    return (
      <div
        key={rowKey?.(index) ?? index}
        id={rowId(index)}
        role="row"
        // One-based, after the heading: what a reader is told is the row's
        // place in the whole grid, not among the few in the document.
        aria-rowindex={index + 1 + (headed ? 1 : 0)}
        aria-selected={isChosen}
        // A row not read yet says so, and is drawn empty at its height.
        aria-busy={cells === null || undefined}
        // The click focuses the grid on its way, which is the tab stop.
        onClick={() => choose(index)}
        onDoubleClick={onActivate && (() => onActivate(index))}
        className={cn(
          'grid cursor-pointer items-center text-sm transition-colors',
          columns,
          isChosen ? 'bg-accent-soft text-text' : 'hover:bg-soft',
        )}
        style={{ height: rowHeight }}
      >
        {cells ?? <div role="gridcell" />}
      </div>
    )
  })

  return (
    <div
      ref={box}
      role="grid"
      aria-label={label}
      aria-rowcount={count + (headed ? 1 : 0)}
      aria-activedescendant={current !== null && current >= start && current < end ? rowId(current) : undefined}
      // One stop for the whole grid: the arrows move inside it.
      tabIndex={0}
      onKeyDown={onKeyDown}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      className={cn(
        'relative overflow-auto',
        // Inside the box: it scrolls, and a ring outside would be cut by
        // whatever holds it.
        'focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent',
        className,
      )}
      {...props}
    >
      {headed && (
        <div
          ref={head}
          role="row"
          aria-rowindex={1}
          className={cn('sticky top-0 grid items-center border-b border-line bg-raise [z-index:var(--z-sticky)]', columns)}
        >
          {header}
        </div>
      )}
      {/* The spacer is what the scrollbar measures; the rows in view ride on
        * it, and the layer rides over exactly them. */}
      <div style={{ height: totalHeight }} className="relative">
        <div style={{ transform: `translateY(${offsetTop}px)` }} className="absolute inset-x-0 top-0">
          {rows}
        </div>
        {layer && (
          <div
            aria-hidden
            className="pointer-events-none absolute inset-x-0"
            style={{ top: offsetTop, height: (end - start) * rowHeight }}
          >
            {layer({ start, end, rowHeight })}
          </div>
        )}
      </div>
    </div>
  )
}

/** A cell of a row. Its words truncate, so a row keeps its height. */
export function VirtualGridCell({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div role="gridcell" className={cn('min-w-0 truncate px-2', className)} {...props} />
}

/** A heading of a column, in the grid's standing row. */
export function VirtualGridColumn({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div role="columnheader" className={cn('caption min-w-0 truncate px-2 py-1.5', className)} {...props} />
}
