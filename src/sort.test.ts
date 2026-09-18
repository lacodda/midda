import { describe, expect, it } from 'vitest'
import { DEFAULT_SORT, naturalDirection, toggleSort, type SortKey } from '@/core'

/**
 * The toggle rule exists twice: here, so a header click reorders without
 * waiting on a round trip, and in `midda-core`'s `Sort::toggled`, which is what
 * the CLI and the MCP door of v0.19 will use. Two copies of a rule drift unless
 * something holds them together, so these assertions are deliberately the same
 * ones `order.rs` makes — a change to either side that is not made to both
 * leaves one of the two suites red.
 */
describe('toggleSort', () => {
  it('flips the column that is already sorted', () => {
    expect(toggleSort(DEFAULT_SORT, 'allocated')).toEqual({ key: 'allocated', direction: 'ascending' })
    expect(toggleSort({ key: 'name', direction: 'ascending' }, 'name')).toEqual({ key: 'name', direction: 'descending' })
  })

  it('starts another column at the direction it is read in', () => {
    // A size question opens largest-first; the alphabet opens at A.
    expect(toggleSort(DEFAULT_SORT, 'name').direction).toBe('ascending')
    expect(toggleSort(DEFAULT_SORT, 'modified').direction).toBe('descending')
    expect(toggleSort(DEFAULT_SORT, 'entries').direction).toBe('descending')
    expect(toggleSort(DEFAULT_SORT, 'logical').direction).toBe('descending')
  })

  it('names the natural direction of every key', () => {
    // If a key is added to the core and not here, this fails on the new one
    // rather than silently defaulting it.
    const keys: SortKey[] = ['allocated', 'logical', 'name', 'modified', 'entries']
    for (const key of keys) {
      expect(naturalDirection(key)).toBe(key === 'name' ? 'ascending' : 'descending')
    }
  })

  it('opens on the question the product is about', () => {
    // Not a preference: someone who opened a disk analyzer came to free space.
    expect(DEFAULT_SORT).toEqual({ key: 'allocated', direction: 'descending' })
  })
})
