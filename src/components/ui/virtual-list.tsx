import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { cn } from 'dowel-ui'

/*
 * A list of a hundred thousand rows that puts a screenful in the DOM.
 *
 * The browser is fine with long lists until it is not: a hundred thousand
 * `<div>`s is a layout the machine recomputes on every change, and the page
 * stops responding while it does. What is drawn instead is the window the
 * reader can actually see, held in place by a tall spacer, so the scrollbar
 * still says how much there is.
 *
 * No virtualisation library. `react-window`, `react-virtuoso` and
 * `@tanstack/react-virtual` all solve the general problem - variable heights
 * measured at runtime, horizontal windows, grids - and the general problem is
 * not the one a design system has: every long list in a product is a column of
 * rows of one height. That case is the arithmetic below, and it is small
 * enough to read.
 *
 * The arithmetic is exported separately, and that is the part worth stating.
 * A window computed inside an effect can only be tested by a browser, and
 * jsdom gives every element a height of zero - so a test would assert against
 * a list that believes it is invisible, and pass whatever the code did. As
 * plain functions the rules are checkable: given a scroll offset and a height,
 * these rows are drawn.
 *
 * Variable row heights are deliberately not supported. Measuring them means
 * rendering to find out, which means a second pass, a cache, and a scrollbar
 * that changes length as the reader travels. When a product needs that, it
 * needs a library rather than a bigger version of this.
 */

export interface Window {
  /** Index of the first row to render. */
  start: number
  /** Index one past the last row to render. */
  end: number
  /** The height of the whole list, drawn or not - what makes the scrollbar
   * honest. */
  totalHeight: number
  /** How far down the first drawn row sits. Applied as a translation rather
   * than as a top margin, so the rows are not re-laid-out on every scroll. */
  offsetTop: number
}

export interface WindowInput {
  /** How many rows there are in total. */
  count: number
  /** How tall one row is, in pixels. One height for every row. */
  rowHeight: number
  /** How tall the scrolling box is. */
  viewportHeight: number
  /** How far the box is scrolled. */
  scrollTop: number
  /** How many extra rows to draw beyond each edge.
   *
   * Not decoration: without it a fast scroll paints blank where rows have not
   * been drawn yet, because the scroll event arrives after the pixels. Three
   * rows is enough to cover a frame at ordinary speeds and cheap enough not to
   * matter. */
  overscan?: number
}

/** Which rows to draw. Pure arithmetic, so it can be checked without a browser.
 *
 * Every edge case here is one that shows as a blank list rather than as an
 * error: a viewport of zero (the box has not been measured yet), a scroll
 * position past the end (the list shrank under the reader), a negative one
 * (elastic scrolling on a trackpad). */
export function windowFor({
  count,
  rowHeight,
  viewportHeight,
  scrollTop,
  overscan = 3,
}: WindowInput): Window {
  const rows = Math.max(0, Math.floor(count))
  const height = Math.max(1, rowHeight)
  const totalHeight = rows * height

  if (rows === 0) return { start: 0, end: 0, totalHeight: 0, offsetTop: 0 }

  // Clamped rather than trusted. A negative scrollTop is what a trackpad hands
  // over at the top of an elastic scroll, and `Math.floor` of it walks the
  // start index backwards into rows that do not exist.
  const top = Math.min(Math.max(0, scrollTop), Math.max(0, totalHeight - 1))

  const first = Math.floor(top / height)
  /* One more row than fits, so the row cut by the bottom edge is drawn rather
   * than appearing as the reader scrolls into it.
   *
   * The `+ 1` is also what keeps the first frame from being empty: before the
   * effect measures the box, `viewportHeight` is 0 and this is 1. A
   * `Math.max(1, ...)` around it looks like the guard for that and is dead
   * code - the expression cannot reach zero. Mutation found it: removing the
   * floor changed nothing at all. */
  const visible = Math.ceil(Math.max(0, viewportHeight) / height) + 1

  const start = Math.max(0, first - overscan)
  const end = Math.min(rows, first + visible + overscan)

  return { start, end, totalHeight, offsetTop: start * height }
}

export interface VirtualListProps<Row> {
  rows: readonly Row[]
  /** How tall one row is. Fixed, and the component enforces it - a row that
   * grows past this height overlaps its neighbour, which is why the value is
   * required rather than measured. */
  rowHeight: number
  children: (row: Row, index: number) => ReactNode
  /** A stable identity per row. Without it React keys by index, and a list
   * that is sorted or filtered reuses the wrong DOM node - the row moves but
   * its input keeps the text that was typed into a different row. */
  rowKey: (row: Row, index: number) => string | number
  overscan?: number
  /** What a screen reader calls the list. Required: a scrolling box of rows
   * with no name is announced as "list" and nothing else, and the word is the
   * product's to give. */
  label: string
  className?: string
}

export function VirtualList<Row>({
  rows,
  rowHeight,
  children,
  rowKey,
  overscan = 3,
  label,
  className,
}: VirtualListProps<Row>) {
  const viewport = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(0)

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

    /* The height is watched, not read once. A list inside a panel that opens,
     * a window the reader resizes, a font that loads late - all change the
     * viewport after mount, and a list that measured once draws the wrong
     * number of rows until something scrolls. */
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [measure])

  const { start, end, totalHeight, offsetTop } = windowFor({
    count: rows.length,
    rowHeight,
    viewportHeight,
    scrollTop,
    overscan,
  })

  return (
    <div
      ref={viewport}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      // `tabIndex` because a box that scrolls has to be reachable: a keyboard
      // user with no focusable row inside cannot otherwise page through it.
      tabIndex={0}
      role="list"
      aria-label={label}
      className={cn(
        'relative overflow-y-auto',
        'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
        className,
      )}
    >
      {/* The spacer is what the scrollbar measures. Without it the box is only
        * as tall as the rows drawn, and the reader can never scroll past the
        * first screenful. */}
      <div style={{ height: totalHeight }} className="relative">
        <div
          // Translated rather than positioned with `top`: a transform is
          // composited, while changing `top` re-lays-out the subtree on every
          // scroll event.
          style={{ transform: `translateY(${offsetTop}px)` }}
          className="absolute inset-x-0 top-0"
        >
          {rows.slice(start, end).map((row, offset) => {
            const index = start + offset
            return (
              <div
                key={rowKey(row, index)}
                role="listitem"
                /* The pair that makes a window honest to a screen reader.
                 *
                 * Without them the reader counts the rows it can see and says
                 * "3 of 20" in a list of a hundred thousand - the rows in the
                 * DOM are a screenful, and that is exactly what a virtual list
                 * hides. `aria-setsize` is the real total, `aria-posinset` the
                 * real position.
                 *
                 * Deliberately not `aria-rowcount`/`aria-rowindex`, which were
                 * the obvious names and are wrong: those belong to `grid`,
                 * `table` and `treegrid`. On a `list` they are unsupported -
                 * axe rejects them, and a reader would simply ignore them,
                 * which is the worse half: the markup would look like it said
                 * something. */
                aria-setsize={rows.length}
                aria-posinset={index + 1}
                style={{ height: rowHeight }}
              >
                {children(row, index)}
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
