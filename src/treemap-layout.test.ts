import { describe, expect, it } from 'vitest'

import { boxOf, minAreaFor, neighbourOf, tileAt, type Box } from '@/treemap-layout'

describe('boxOf', () => {
  it('turns a fraction of the box into whole pixels', () => {
    const box = boxOf({ x: 0, y: 0, width: 0.5, height: 1 }, 800, 600)
    expect(box).toEqual({ x: 0, y: 0, width: 400, height: 600 })
  })

  it('leaves no seam between neighbours that shared an edge', () => {
    // The rule this function exists for. Rounding a width on its own gives two
    // tiles that either overlap by a pixel or leave a hairline of background
    // between them, and a picture of a thousand tiles shows it a thousand
    // times. Rounding the far edge and subtracting makes the seam impossible.
    const left = boxOf({ x: 0, y: 0, width: 1 / 3, height: 1 }, 1000, 400)
    const right = boxOf({ x: 1 / 3, y: 0, width: 2 / 3, height: 1 }, 1000, 400)

    expect(left.x + left.width).toBe(right.x)
    expect(right.x + right.width).toBe(1000)
  })

  it('leaves no seam where rounding pulls the two edges apart', () => {
    // A split at 0.3 of 205 pixels: the left tile ends at 61.5 and the right
    // starts there. Rounding each width on its own gives 62 and 144, which is
    // 206 — a pixel of overlap. Rounding the far edge gives 62 and 143.
    const left = boxOf({ x: 0, y: 0, width: 0.3, height: 1 }, 205, 100)
    const right = boxOf({ x: 0.3, y: 0, width: 0.7, height: 1 }, 205, 100)

    expect(left.x + left.width).toBe(right.x)
    expect(left.width + right.width).toBe(205)
  })

  it('covers the box exactly across a whole row of tiles', () => {
    // The property, rather than one lucky split: whatever the fractions, the
    // widths add up to the box and every tile starts where the last one ended.
    const fractions = [0.17, 0.23, 0.09, 0.31, 0.2]
    let at = 0
    let edge = 0
    for (const fraction of fractions) {
      const box = boxOf({ x: at, y: 0, width: fraction, height: 1 }, 733, 100)
      expect(box.x).toBe(edge)
      edge = box.x + box.width
      at += fraction
    }
    expect(edge).toBe(733)
  })

  it('gives a tile of no area no pixels rather than a negative width', () => {
    const box = boxOf({ x: 0.5, y: 0.5, width: 0, height: 0 }, 400, 400)
    expect(box.width).toBe(0)
    expect(box.height).toBe(0)
  })
})

describe('tileAt', () => {
  const boxes: Box[] = [
    { x: 0, y: 0, width: 100, height: 100 },
    { x: 100, y: 0, width: 50, height: 100 },
  ]

  it('finds the tile under a point', () => {
    expect(tileAt(boxes, 50, 50)).toBe(0)
    expect(tileAt(boxes, 120, 20)).toBe(1)
  })

  it('gives the boundary pixel to the tile drawn later', () => {
    // Two tiles can share a pixel column after rounding. Without a rule the
    // hover flickers between them as the pointer sits still.
    expect(tileAt(boxes, 100, 50)).toBe(1)
  })

  it('answers nothing outside every tile', () => {
    expect(tileAt(boxes, 400, 400)).toBeNull()
    expect(tileAt([], 0, 0)).toBeNull()
  })
})

describe('minAreaFor', () => {
  it('is the share a 24-pixel square takes of this box', () => {
    // 900×600 is 540,000 px²; a 24×24 tile is 576 of them.
    expect(minAreaFor(900, 600)).toBeCloseTo(576 / 540_000, 10)
  })

  it('never asks for more than a quarter of the box', () => {
    // A box smaller than four 24-pixel squares would otherwise ask the core to
    // draw nothing but the remainder.
    expect(minAreaFor(30, 30)).toBeLessThanOrEqual(0.25)
  })

  it('survives a box that has not been measured yet', () => {
    // The first frame, before the ResizeObserver has run.
    expect(minAreaFor(0, 0)).toBeGreaterThan(0)
    expect(Number.isFinite(minAreaFor(0, 0))).toBe(true)
  })
})

describe('neighbourOf', () => {
  /*
   * A two-row layout, the shape squarified output actually takes:
   *
   *   +--------+-----+
   *   |   0    |  1  |
   *   +--------+-----+
   *   |  2  |  3  |4 |
   *   +-----+-----+--+
   */
  const boxes: Box[] = [
    { x: 0, y: 0, width: 120, height: 60 },
    { x: 120, y: 0, width: 80, height: 60 },
    { x: 0, y: 60, width: 70, height: 40 },
    { x: 70, y: 60, width: 70, height: 40 },
    { x: 140, y: 60, width: 60, height: 40 },
  ]

  it('moves to the tile in the direction pressed', () => {
    expect(neighbourOf(boxes, 0, 'right')).toBe(1)
    expect(neighbourOf(boxes, 1, 'left')).toBe(0)
    expect(neighbourOf(boxes, 0, 'down')).toBe(2)
    expect(neighbourOf(boxes, 2, 'up')).toBe(0)
  })

  it('stays in its row rather than cutting a corner', () => {
    // From tile 2, Right means 3 — not 4, which is further, and not 1, which is
    // up and to the right. A move that jumps rows makes the picture
    // unnavigable: the reader loses where they were.
    expect(neighbourOf(boxes, 2, 'right')).toBe(3)
    expect(neighbourOf(boxes, 3, 'right')).toBe(4)
  })

  it('never walks backwards', () => {
    // Pressing Right at the right edge does nothing, rather than wrapping to
    // the left: a wrap reads as the selection jumping at random.
    expect(neighbourOf(boxes, 4, 'right')).toBeNull()
    expect(neighbourOf(boxes, 0, 'left')).toBeNull()
    expect(neighbourOf(boxes, 0, 'up')).toBeNull()
  })

  it('ignores a tile of no area', () => {
    // An empty rectangle is not somewhere the selection can go: the reader
    // would see nothing highlighted and think the key did nothing.
    const withEmpty: Box[] = [
      { x: 0, y: 0, width: 100, height: 100 },
      { x: 100, y: 0, width: 0, height: 0 },
      { x: 120, y: 0, width: 80, height: 100 },
    ]
    expect(neighbourOf(withEmpty, 0, 'right')).toBe(2)
  })

  it('will not move diagonally into another row', () => {
    // The condition the row test cannot see. Here nothing is level with tile 0
    // to its left — but tile 1 sits below and starts further left, so a rule
    // that only asks "is it past my edge" hands it over, and Left from the top
    // of the picture drops a row.
    //
    // Found by mutation: removing the overlap check left every other test
    // green.
    const stacked: Box[] = [
      { x: 100, y: 0, width: 100, height: 50 },
      { x: 0, y: 50, width: 60, height: 50 },
    ]
    expect(neighbourOf(stacked, 0, 'left')).toBeNull()
    expect(neighbourOf(stacked, 1, 'right')).toBeNull()

    // And once they do overlap, the same move works.
    const beside: Box[] = [
      { x: 100, y: 0, width: 100, height: 50 },
      { x: 0, y: 20, width: 60, height: 50 },
    ]
    expect(neighbourOf(beside, 0, 'left')).toBe(1)
  })

  it('answers nothing when there is nowhere to go', () => {
    expect(neighbourOf([], 0, 'right')).toBeNull()
    expect(neighbourOf(boxes, 99, 'right')).toBeNull()
  })
})
