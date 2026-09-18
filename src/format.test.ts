import { describe, expect, it } from 'vitest'
import { describeOverhead, formatAge, formatBytes, formatCount, formatShare } from '@/format'

describe('formatBytes', () => {
  it('names the binary steps the way Explorer does', () => {
    // The number the user will compare this against is the one Windows shows
    // them, so the steps are binary and the names are decimal.
    expect(formatBytes(1024)).toBe('1.0 KB')
    expect(formatBytes(1024 * 1024)).toBe('1.0 MB')
    expect(formatBytes(1024 ** 3)).toBe('1.0 GB')
    expect(formatBytes(1024 ** 4)).toBe('1.0 TB')
  })

  it('keeps a decimal where it separates two folders and drops it where it does not', () => {
    // 1.2 GB beside 1.9 GB has to read as different sizes.
    expect(formatBytes(1.2 * 1024 ** 3)).toBe('1.2 GB')
    expect(formatBytes(1.9 * 1024 ** 3)).toBe('1.9 GB')
    // Past ten the decimal stops carrying information.
    expect(formatBytes(42.7 * 1024 ** 3)).toBe('43 GB')
  })

  it('shows bare bytes as whole numbers', () => {
    expect(formatBytes(1)).toBe('1 B')
    expect(formatBytes(999)).toBe('999 B')
    expect(formatBytes(0)).toBe('0 B')
  })

  it('refuses to render a number that is not a size', () => {
    expect(formatBytes(-1)).toBe('—')
    expect(formatBytes(Number.NaN)).toBe('—')
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe('—')
  })

  it('carries into the next unit when rounding fills the current one', () => {
    // The step is chosen before the rounding, so a value just under the next
    // unit rounds up to 1024 of the one below — a unit nobody writes. Found on
    // the first screen built: a 1.8 TB drive reported "1024 GB used".
    expect(formatBytes(1024 ** 4 - 1)).toBe('1.0 TB')
    expect(formatBytes(1023.6 * 1024 ** 3)).toBe('1.0 TB')
    expect(formatBytes(1024 ** 2 - 1)).toBe('1.0 MB')
    // And the value just below the carry is left alone.
    expect(formatBytes(1023 * 1024 ** 3)).toBe('1023 GB')
  })

  it('does not run off the end of its units', () => {
    // A petabyte-scale number is not a real volume, but a corrupt one can
    // arrive, and an undefined unit renders as "undefined".
    expect(formatBytes(1024 ** 7)).toMatch(/PB$/)
  })
})

describe('formatAge', () => {
  const now = Date.UTC(2026, 8, 18)
  const daysAgo = (days: number) => now - days * 86_400_000

  it('answers the question it is asked: live or fossilized', () => {
    expect(formatAge(daysAgo(0), now)).toBe('today')
    expect(formatAge(daysAgo(1), now)).toBe('yesterday')
    expect(formatAge(daysAgo(5), now)).toBe('5 days')
    expect(formatAge(daysAgo(45), now)).toBe('1 month')
    expect(formatAge(daysAgo(400), now)).toBe('1 year')
    expect(formatAge(daysAgo(1200), now)).toBe('3 years')
  })

  it('leaves no gap between the last month and the first year', () => {
    // Months step by 30 days and years by 365, so the five days between the
    // twelfth month and the first year belonged to neither branch: 360 days
    // rendered as "0 years". Found on screen, on a real list of files.
    for (let days = 28; days <= 800; days += 1) {
      const age = formatAge(daysAgo(days), now)
      expect(age, `${days} days ago`).not.toMatch(/^0 /)
    }
    // 360 and 364 days are twelve 30-day months and not yet a 365-day year;
    // they now say so instead of falling through to "0 years".
    expect(formatAge(daysAgo(360), now)).toBe('12 months')
    expect(formatAge(daysAgo(364), now)).toBe('12 months')
    expect(formatAge(daysAgo(365), now)).toBe('1 year')
  })

  it('says nothing when the filesystem said nothing', () => {
    // An invented date would poison every "what is old" question the later
    // versions are built on.
    expect(formatAge(null, now)).toBe('—')
  })

  it('does not pretend a future timestamp is an age', () => {
    // A file dated tomorrow is a clock that was wrong, not a negative age.
    expect(formatAge(now + 86_400_000, now)).toBe('ahead')
  })
})

describe('formatShare', () => {
  it('gives a percentage of a total', () => {
    expect(formatShare(50, 200)).toBe('25%')
    expect(formatShare(1, 200)).toBe('0.5%')
  })

  it('does not show arithmetic on an empty scan', () => {
    expect(formatShare(0, 0)).toBe('0%')
  })

  it('says a row is small rather than rounding it away', () => {
    // "0.0%" beside a real number looks like a bug.
    expect(formatShare(1, 100_000)).toBe('<0.1%')
  })
})

describe('describeOverhead', () => {
  it('names the cost of a folder full of tiny files', () => {
    // The product's own argument: a hundred thousand small files cost far more
    // than they read.
    expect(describeOverhead(30, 8192)).toMatch(/more on disk/)
  })

  it('names the saving of a sparse or compressed file', () => {
    expect(describeOverhead(360_000, 45_056)).toMatch(/less on disk/)
  })

  it('stays quiet when the two numbers agree', () => {
    // Saying "0 B more on disk" on every ordinary row would be noise.
    expect(describeOverhead(4096, 4096)).toBeNull()
    expect(describeOverhead(1000, 1024)).toBeNull()
  })

  it('stays quiet rather than dividing by zero', () => {
    expect(describeOverhead(0, 0)).toBeNull()
    expect(describeOverhead(0, 4096)).toBeNull()
  })
})

describe('formatCount', () => {
  it('separates thousands', () => {
    expect(formatCount(1_234_567)).toMatch(/1\D?234\D?567/)
  })
})
