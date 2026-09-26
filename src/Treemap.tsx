import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import { treemap, type Row, type SizeBasis, type Tile } from '@/core'
import { formatBytes, formatCount } from '@/format'
import { boxOf, CATEGORY_LABELS, minAreaFor, neighbourOf, paletteOf, tileAt, type Box } from '@/treemap-layout'

/** How much of the box a tile needs before a label fits on it. */
const LABEL_MIN_WIDTH = 56
const LABEL_MIN_HEIGHT = 24

interface TreemapProps {
  /** The folder being drawn. */
  parentId: number
  /** Its name, for the picture's own label. */
  parentName: string
  basis: SizeBasis
  /** The row the table has selected, so the picture can show the same one. */
  selected: Row | null
  /** A tile was clicked: descend into it if it is a folder. */
  onDescend: (id: number) => void
  /** A tile was highlighted — by hover, by arrow key, or by a click that did
   * not descend. The table follows. */
  onSelect: (id: number | null) => void
}

/**
 * The folder as a picture: one rectangle per child, area proportional to what
 * it occupies.
 *
 * Canvas rather than DOM, and it is the one place in this product where that is
 * true. A treemap is hundreds of filled rectangles that change together on
 * every hover, resize and descent — as elements that is hundreds of nodes the
 * browser lays out and repaints, and the picture is not a document: nothing in
 * it is selectable text, and nothing in it is a control on its own. What a
 * screen reader needs is served beside it, by the table, which holds the same
 * rows with the same order and the same names.
 *
 * The layout is not computed here. The core lays the rectangles out in
 * fractions and decides what is too small to draw; this multiplies by the
 * canvas size and paints. That split is why the arithmetic can be tested at all
 * — a canvas cannot be asked what it drew.
 */
export function Treemap({ parentId, parentName, basis, selected, onDescend, onSelect }: TreemapProps) {
  const canvas = useRef<HTMLCanvasElement>(null)
  const frame = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })
  const [tiles, setTiles] = useState<Tile[]>([])
  const [hovered, setHovered] = useState<number | null>(null)
  /** Where the keyboard is, which is not the same as what the pointer is over. */
  const [focused, setFocused] = useState<number | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  /** What the last copy said, shown briefly beside the button. */
  const [copied, setCopied] = useState<string | null>(null)

  // Which request is current: a reader who descends twice quickly has two
  // layouts in flight, and the slower one must not land on the newer picture.
  const generation = useRef(0)

  useEffect(() => {
    const element = frame.current
    if (!element) return

    const measure = () => setSize({ width: element.clientWidth, height: element.clientHeight })
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  // Ask the core for the layout. The window's own shape and its idea of the
  // smallest useful rectangle go with the request: the core works in fractions
  // and cannot know either.
  useEffect(() => {
    if (size.width <= 0 || size.height <= 0) return

    generation.current += 1
    const mine = generation.current

    void treemap(parentId, {
      basis,
      aspect: size.width / size.height,
      minArea: minAreaFor(size.width, size.height),
      maxTiles: 250,
    })
      .then((laid) => {
        if (mine !== generation.current) return
        setTiles(laid)
        setFailure(null)
        setHovered(null)
        setFocused(null)
      })
      .catch((cause: unknown) => {
        if (mine !== generation.current) return
        setFailure(String(cause))
      })
  }, [parentId, basis, size.width, size.height])

  const boxes = useMemo<Box[]>(() => tiles.map((tile) => boxOf(tile.rect, size.width, size.height)), [tiles, size])

  /** Which tile the table's selection stands for, if it is drawn at all. */
  const selectedIndex = useMemo(() => {
    if (selected === null) return null
    const at = tiles.findIndex((tile) => tile.id === selected.id)
    return at === -1 ? null : at
  }, [tiles, selected])

  // Paint. Everything that can change the picture is a dependency, and the
  // whole canvas is redrawn — a treemap has no partial update that is cheaper
  // than a few hundred filled rectangles.
  useEffect(() => {
    const element = canvas.current
    if (!element || size.width <= 0 || size.height <= 0) return

    // The backing store is in device pixels; the canvas is sized in CSS ones.
    // Without this the picture is soft on every display that is not 1×, which
    // on Windows is most of them.
    const ratio = window.devicePixelRatio || 1
    element.width = Math.round(size.width * ratio)
    element.height = Math.round(size.height * ratio)

    const context = element.getContext('2d')
    if (!context) return
    context.setTransform(ratio, 0, 0, ratio, 0, 0)

    const styles = getComputedStyle(element)
    const ground = styles.getPropertyValue('--bg').trim()
    const line = styles.getPropertyValue('--text').trim()

    // The stylesheet has not arrived. Painting with invented colours would put
    // the dark theme on a white window; an empty canvas for one frame is the
    // honest state, and the next paint has the tokens.
    if (!ground || !line) return

    context.clearRect(0, 0, size.width, size.height)

    tiles.forEach((tile, index) => {
      const box = boxes[index]
      if (!box || box.width <= 0 || box.height <= 0) return

      const palette = paletteOf(styles, tile.category)
      if (!palette.fill) return
      const lit = index === hovered || index === focused || index === selectedIndex

      context.fillStyle = palette.fill
      context.globalAlpha = lit ? 1 : 0.82
      context.fillRect(box.x, box.y, box.width, box.height)
      context.globalAlpha = 1

      // The gap between tiles is drawn as a stroke in the ground colour rather
      // than left as a margin: a margin would make the areas wrong, and the
      // area is the one thing the picture promises.
      context.strokeStyle = ground
      context.lineWidth = 1
      context.strokeRect(box.x + 0.5, box.y + 0.5, box.width - 1, box.height - 1)

      if (index === selectedIndex || index === focused) {
        context.strokeStyle = line
        context.lineWidth = 2
        context.strokeRect(box.x + 1, box.y + 1, box.width - 2, box.height - 2)
      }

      if (box.width < LABEL_MIN_WIDTH || box.height < LABEL_MIN_HEIGHT) return

      context.save()
      context.beginPath()
      context.rect(box.x, box.y, box.width, box.height)
      context.clip()
      context.fillStyle = palette.label
      context.font = `600 12px ${styles.getPropertyValue('--font-sans').trim()}`
      context.textBaseline = 'top'
      context.fillText(tile.name, box.x + 6, box.y + 5, box.width - 12)

      if (box.height >= LABEL_MIN_HEIGHT + 14) {
        context.globalAlpha = 0.75
        context.font = `400 11px ${styles.getPropertyValue('--font-mono').trim()}`
        context.fillText(formatBytes(tile.bytes), box.x + 6, box.y + 20, box.width - 12)
        context.globalAlpha = 1
      }
      context.restore()
    })
  }, [tiles, boxes, size, hovered, focused, selectedIndex])

  const pointAt = useCallback((event: React.MouseEvent<HTMLCanvasElement>) => {
    const element = event.currentTarget
    const bounds = element.getBoundingClientRect()
    return { x: event.clientX - bounds.left, y: event.clientY - bounds.top }
  }, [])

  const move = useCallback(
    (event: React.MouseEvent<HTMLCanvasElement>) => {
      const { x, y } = pointAt(event)
      setHovered(tileAt(boxes, x, y))
    },
    [boxes, pointAt],
  )

  /* One click points at a tile; two goes into it. The same gesture as the
   * table, and it has to be: with a single click that descended, a folder
   * could not be pointed at at all — pointing at it would have left it, and
   * the two views could never be showing the same thing. */
  const click = useCallback(
    (event: React.MouseEvent<HTMLCanvasElement>) => {
      const { x, y } = pointAt(event)
      const at = tileAt(boxes, x, y)
      if (at === null) return
      setFocused(at)

      const tile = tiles[at]
      // The remainder tile stands for no single node, so there is nothing for
      // the table to point at: it highlights here and says what it gathers,
      // which is all it can honestly do.
      onSelect(tile?.id ?? null)
    },
    [boxes, tiles, pointAt, onSelect],
  )

  const open = useCallback(
    (event: React.MouseEvent<HTMLCanvasElement>) => {
      const { x, y } = pointAt(event)
      const at = tileAt(boxes, x, y)
      if (at === null) return
      const tile = tiles[at]
      if (tile && tile.id !== null && tile.isDirectory) onDescend(tile.id)
    },
    [boxes, tiles, pointAt, onDescend],
  )

  const keys = useCallback(
    (event: React.KeyboardEvent<HTMLCanvasElement>) => {
      const directions: Record<string, 'up' | 'down' | 'left' | 'right'> = {
        ArrowUp: 'up',
        ArrowDown: 'down',
        ArrowLeft: 'left',
        ArrowRight: 'right',
      }

      const direction = directions[event.key]
      if (direction) {
        event.preventDefault()
        // Nothing focused yet: the first arrow lands on the biggest tile, which
        // is the first one — the layout comes back largest-first.
        const next = focused === null ? (tiles.length > 0 ? 0 : null) : neighbourOf(boxes, focused, direction)
        if (next === null) return
        setFocused(next)
        onSelect(tiles[next]?.id ?? null)
        return
      }

      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault()
        if (focused === null) return
        const tile = tiles[focused]
        if (tile && tile.id !== null && tile.isDirectory) onDescend(tile.id)
        return
      }

      if (event.key === 'Home') {
        event.preventDefault()
        if (tiles.length === 0) return
        setFocused(0)
        onSelect(tiles[0]?.id ?? null)
      }
    },
    [boxes, tiles, focused, onDescend, onSelect],
  )

  const shown = hovered ?? focused
  const tip = shown === null ? null : (tiles[shown] ?? null)
  const tipBox = shown === null ? null : (boxes[shown] ?? null)

  const total = useMemo(() => tiles.reduce((sum, tile) => sum + tile.bytes, 0), [tiles])

  /**
   * The picture, as an image on the clipboard.
   *
   * Straight off the canvas rather than through the core: the bytes are already
   * rendered here, and sending a layout back across the bridge for the core to
   * rasterize would mean a second renderer to keep in step with this one — two
   * pictures of one folder that could disagree.
   *
   * `navigator.clipboard.write` needs a user gesture, which a button is, and
   * the WebView2 it runs in supports `image/png`. What it does not support is
   * being called from a promise chain that awaits first, so the `ClipboardItem`
   * is handed the *promise* of a blob rather than an awaited one.
   */
  const copy = useCallback(() => {
    const element = canvas.current
    if (!element) return
    setCopied(null)

    const blob = new Promise<Blob>((resolve, reject) => {
      element.toBlob((made) => (made ? resolve(made) : reject(new Error('the picture could not be encoded'))), 'image/png')
    })

    void navigator.clipboard
      .write([new ClipboardItem({ 'image/png': blob })])
      .then(() => setCopied('Copied'))
      // The browser's own message names a DOM method and an exception class.
      // What a reader can act on is that it did not happen, so that is what
      // they are told.
      .catch(() => setCopied('Could not copy'))
  }, [])

  // The confirmation clears itself: a label that stayed would read as a state
  // the picture is in rather than something that just happened.
  useEffect(() => {
    if (copied === null) return
    const timer = window.setTimeout(() => setCopied(null), 2_000)
    return () => window.clearTimeout(timer)
  }, [copied])

  if (failure !== null) {
    return <p className="p-4 text-sm text-bad">{failure}</p>
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* The legend and the one action, on a bar of their own. Floated over the
          picture they would cover a tile — and the tile they cover is the
          largest one, because that is where the corner is. */}
      <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-line px-4 py-1.5 text-xs text-dim">
        {(['build', 'source', 'media', 'other'] as const).map((category) => (
          <span key={category} className="flex items-center gap-1.5">
            <span aria-hidden className={`size-2.5 rounded-xs ${LEGEND_SWATCH[category]}`} />
            {CATEGORY_LABELS[category]}
          </span>
        ))}

        <span className="ml-auto flex items-center gap-2">
          {copied !== null && <span aria-live="polite">{copied}</span>}
          <Button variant="ghost" size="sm" onClick={copy} title="Copy the picture as a PNG image">
            Copy picture
          </Button>
        </span>
      </div>

      <div ref={frame} className="relative min-h-0 flex-1 p-2">
        <canvas
          ref={canvas}
          style={{ width: size.width, height: size.height }}
          className="absolute inset-2 rounded-md focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
          tabIndex={0}
          // The picture is one control, not a hundred: the rows it draws are in
          // the table beside it, where a screen reader can read them as rows.
          role="img"
          aria-label={`Treemap of ${parentName}: ${formatCount(tiles.length)} areas, ${formatBytes(total)}`}
          onMouseMove={move}
          onMouseLeave={() => setHovered(null)}
          onClick={click}
          onDoubleClick={open}
          onKeyDown={keys}
        />

        {tip !== null && tipBox !== null && (
          <div
            // Above the tile unless the tile is near the top, in which case
            // below it: a tooltip clipped by the panel says nothing.
            style={{
              left: Math.min(Math.max(tipBox.x, 8), Math.max(8, size.width - 260)),
              top: tipBox.y > 80 ? tipBox.y - 8 : tipBox.y + tipBox.height + 8,
              transform: tipBox.y > 80 ? 'translateY(-100%)' : undefined,
            }}
            className="pointer-events-none absolute z-10 max-w-65 rounded-md border border-line bg-raise px-3 py-2 text-xs shadow-lift"
            role="status"
          >
            {/* The whole path, not the name: a rectangle called `src` is one
                of a dozen on a developer's disk, and which one it is is the
                question the picture raised. `break-all` because a path has no
                spaces to wrap at. */}
            <p className="mb-1 break-all font-mono text-2xs text-accent">{tip.path || tip.name}</p>
            <p className="tabular">
              <span className="font-medium">{formatBytes(tip.bytes)}</span>
              <span className="text-dim">
                {' · '}
                {formatCount(tip.entries)} {tip.entries === 1 ? 'entry' : 'entries'}
              </span>
            </p>
            {tip.id !== null && tip.isDirectory && <p className="mt-1 text-2xs text-dim">Double-click to open</p>}
          </div>
        )}
      </div>
    </div>
  )
}

/**
 * The legend swatches, in the same tokens the canvas paints with.
 *
 * Classes rather than a second lookup of the same variables: a class is how the
 * theme reaches an element, and a swatch built from `paletteOf` would be a
 * second code path to keep in step for no gain.
 */
const LEGEND_SWATCH = {
  build: 'bg-accent',
  source: 'bg-info',
  media: 'bg-good',
  other: 'bg-raise border border-line',
} as const
