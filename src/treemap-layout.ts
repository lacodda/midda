/**
 * The arithmetic of drawing a treemap, apart from the canvas that draws it.
 *
 * The layout itself is the core's — squarified rectangles in fractions of the
 * box, with the cut-off and the gathered remainder already decided. What is
 * left here is everything between a fraction and a pixel: where a tile lands in
 * a box of a given size, which tile is under the pointer, what colour a
 * category is, and which tile the arrow keys move to.
 *
 * Kept out of the component for the reason `windowFor` is: a canvas cannot be
 * asked what it drew. Every pixel in it is the same to jsdom and to a
 * screenshot, so the only way these rules can be checked is as plain functions
 * over numbers — which is what they are.
 */

import type { Category, Rect } from '@/core'

/** A rectangle in pixels, as the canvas wants it. */
export interface Box {
  x: number
  y: number
  width: number
  height: number
}

/**
 * Where a tile lands in a box of this size.
 *
 * Rounded to whole pixels, and the far edge is rounded before the width is
 * taken from it rather than after: rounding a width independently of a position
 * leaves seams and overlaps between neighbours that shared an edge in fraction
 * space, and a treemap of a thousand tiles is a thousand chances to show one.
 */
export function boxOf(rect: Rect, width: number, height: number): Box {
  const left = Math.round(rect.x * width)
  const top = Math.round(rect.y * height)
  const right = Math.round((rect.x + rect.width) * width)
  const bottom = Math.round((rect.y + rect.height) * height)
  return { x: left, y: top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) }
}

/**
 * The tile at a point, or `null`.
 *
 * Searched from the end because that is the order they are drawn in: the last
 * tile painted is the one on top, and after rounding two neighbours can share a
 * boundary pixel. Without a rule the hover would flicker between them.
 */
export function tileAt(boxes: readonly Box[], x: number, y: number): number | null {
  for (let index = boxes.length - 1; index >= 0; index -= 1) {
    const box = boxes[index]
    if (!box) continue
    if (x >= box.x && x < box.x + box.width && y >= box.y && y < box.y + box.height) return index
  }
  return null
}

/** The colours a category is drawn in. */
export interface Palette {
  fill: string
  /** What a label on that fill reads as. */
  label: string
}

/**
 * The four fills, read out of the stylesheet rather than written here.
 *
 * A hex in this file would be a second place the theme lives, and it would be
 * wrong in the other theme: the window follows the system, and a violet chosen
 * against a near-black ground is invisible on a white one. The tokens are
 * dowel's, defined for both.
 *
 * Which tokens, and why these:
 *
 * - **build** takes the accent. It is what the product is pointing at — the
 *   gigabytes a tool will write again — and the eye should land on it before
 *   it has read a label.
 * - **source** takes `--info`, **media** takes `--good`. Not the `--series-*`
 *   ramp, which was the obvious reach and is the wrong shape for a canvas: a
 *   series is a fill with no matching text colour, and there is nothing to
 *   write a label in. Every one of these comes as a pair — `--info` with
 *   `--on-info` — worked out against both themes by the design system, which
 *   is exactly the problem a label on a coloured rectangle has.
 * - **other** is the neutral panel colour. It is the absence of a category
 *   rather than a fourth one, and a hue for it would claim to mean something.
 *
 * Not `--scale-*` either: that is a *sequential* ramp, one hue getting
 * lighter, and it says these four are steps of one thing.
 *
 * Every fill has to be opaque. A canvas fill is painted onto whatever is under
 * it, and dowel's surface tokens (`--soft`, `--line-2`) are white at a low
 * alpha — in the DOM they tint the panel behind them, on a canvas they leave
 * the tile see-through and the neighbour's label bleeds into it. `--raise` is
 * the opaque one.
 *
 * No fallback hexes. A colour written here is a second copy of the theme, and
 * it would be the *dark* theme's copy — wrong on a white ground, which is the
 * failure the tokens exist to prevent. When a token reads empty the stylesheet
 * has not arrived, and the answer to that is not to invent a violet.
 */
export function paletteOf(styles: CSSStyleDeclaration, category: Category): Palette {
  const read = (name: string) => styles.getPropertyValue(name).trim()
  switch (category) {
    case 'build':
      return { fill: read('--accent'), label: read('--on-accent') }
    case 'source':
      return { fill: read('--info'), label: read('--on-info') }
    case 'media':
      return { fill: read('--good'), label: read('--on-good') }
    default:
      return { fill: read('--raise'), label: read('--text') }
  }
}

/** What each category is called in the legend. */
export const CATEGORY_LABELS: Record<Category, string> = {
  build: 'rebuilt by a tool',
  source: 'source & docs',
  media: 'media & archives',
  other: 'other',
}

/**
 * The smallest fraction of a box worth drawing as its own tile.
 *
 * Under about 24×24 pixels a tile carries no label, is awkward to hit with a
 * pointer, and adds a border to a picture that is already mostly borders. The
 * core takes a fraction because it does not know the window's size; this is the
 * window doing that division.
 */
export function minAreaFor(width: number, height: number): number {
  const area = width * height
  if (!Number.isFinite(area) || area <= 0) return 0.001
  return Math.min(0.25, (24 * 24) / area)
}

/**
 * Which tile an arrow key moves to.
 *
 * Geometric rather than by index: the tiles are laid out as a picture, and a
 * reader pressing Right means the thing to the right of this one, not the next
 * one in the sorted list — those are the same only in the first row.
 *
 * Two conditions, and the second one is the one that matters. A candidate must
 * begin past the edge the reader is moving off — a tile whose *centre* is to
 * the right is not good enough, because a wide tile below overlaps this one and
 * still has its centre further right. And it must overlap this tile across the
 * axis of travel: without that, Left from the top-left tile walks diagonally
 * into the row below, which reads as the selection jumping at random. Among
 * what is left, the nearest wins, with the across-axis offset weighted so a
 * tile more nearly level is preferred over one merely closer.
 */
export function neighbourOf(boxes: readonly Box[], from: number, direction: 'up' | 'down' | 'left' | 'right'): number | null {
  const origin = boxes[from]
  if (!origin) return null

  const horizontal = direction === 'left' || direction === 'right'

  // The span of each tile along the axis of travel, and across it.
  const along = (box: Box) => (horizontal ? { from: box.x, to: box.x + box.width } : { from: box.y, to: box.y + box.height })
  const across = (box: Box) => (horizontal ? { from: box.y, to: box.y + box.height } : { from: box.x, to: box.x + box.width })

  const hereAlong = along(origin)
  const hereAcross = across(origin)
  const forward = direction === 'right' || direction === 'down'

  let best: number | null = null
  let bestScore = Infinity

  boxes.forEach((box, index) => {
    if (index === from || box.width <= 0 || box.height <= 0) return

    const thereAlong = along(box)
    const thereAcross = across(box)

    // Past the edge being left, not merely centred beyond it.
    const gap = forward ? thereAlong.from - hereAlong.to : hereAlong.from - thereAlong.to
    if (gap < 0) return

    // Overlapping across the axis: the tile has to be beside this one, not
    // diagonally away from it.
    const overlap = Math.min(hereAcross.to, thereAcross.to) - Math.max(hereAcross.from, thereAcross.from)
    if (overlap <= 0) return

    // The nearest, and among equals the one more nearly level. The offset is
    // between the two centres across the axis.
    const offset = Math.abs((thereAcross.from + thereAcross.to) / 2 - (hereAcross.from + hereAcross.to) / 2)
    const score = gap * 2 + offset
    if (score < bestScore) {
      bestScore = score
      best = index
    }
  })

  return best
}
