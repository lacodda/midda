import { afterEach, describe, expect, it } from 'vitest'
import {
  CHANGE_SPANS,
  describeChanges,
  describeComparison,
  describePlace,
  describeSnapshot,
  formatSigned,
  relativeTo,
  reportMarkdown,
  wayOf,
  type ComparisonFacts,
  type ReportFacts,
  describeFreshness,
  describeOverhead,
  describeScan,
  explainSize,
  formatAge,
  formatAgo,
  formatBytes,
  formatCount,
  formatDuration,
  formatShare,
} from '@/format'

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

  it('accounts for an entry that reads as something and occupies nothing', () => {
    // A cloud placeholder, or a second name for shared bytes. This used to fall
    // through to null, which left the two numbers on screen with no account of
    // themselves — the most surprising row in the table was the silent one.
    expect(describeOverhead(5_898_024, 0)).toMatch(/occupies nothing/)
  })
})

describe('explainSize', () => {
  // The bit values mirror `TRAIT` in core.ts, which mirrors `Traits` in the
  // core. Written out rather than imported so this file stays free of the
  // bridge — and so a change to the numbering shows up here as a failure.
  const PLACEHOLDER = 1
  const COMPRESSED = 2
  const SPARSE = 4
  const LINKED = 8
  const COUNTED_HERE = 16
  const HOLDS_SHARED = 32

  it('says nothing about an ordinary file', () => {
    expect(explainSize(0, 1)).toBeNull()
  })

  it('tells the second name that deleting it frees nothing', () => {
    const explanation = explainSize(LINKED, 2)
    expect(explanation).toMatch(/frees nothing/)
  })

  it('does not tell that to the name holding the bytes', () => {
    // The polarity that matters. Both rows carry LINKED; only one of them shows
    // a zero, and putting the warning on the other would send a reader to
    // delete the name that is actually costing them the space.
    const owner = explainSize(LINKED | COUNTED_HERE, 2)
    expect(owner).not.toMatch(/frees nothing/)
    expect(owner).toMatch(/counted here/)
  })

  it('counts the names when it knows how many', () => {
    expect(explainSize(LINKED | COUNTED_HERE, 3)).toMatch(/3 names/)
    expect(explainSize(LINKED | COUNTED_HERE, null)).toMatch(/more than one name/)
  })

  it('explains a cloud placeholder', () => {
    expect(explainSize(PLACEHOLDER, 1)).toMatch(/cloud/)
  })

  it('explains compression and sparseness', () => {
    expect(explainSize(COMPRESSED, 1)).toMatch(/compressed/i)
    expect(explainSize(SPARSE, 1)).toMatch(/sparse/i)
  })

  it('lets a folder that reads as empty account for itself', () => {
    // A folder holding nothing but second names shows 0 B. On screen, with no
    // sentence beside it, that reads as a scanner that gave up on a folder the
    // reader can see is full.
    expect(explainSize(HOLDS_SHARED, null)).toMatch(/named elsewhere/)
  })

  it('prefers what this entry is over what is inside it', () => {
    // A second name that also holds second names beneath it: the stronger claim
    // is about the entry itself.
    expect(explainSize(LINKED | HOLDS_SHARED, 2)).toMatch(/frees nothing/)
  })

  it('leads with the most surprising fact when a file carries several', () => {
    // A cloud placeholder is sparse as well — measured on real OneDrive files,
    // 2026-09-20 — and telling someone their file is sparse when it is in the
    // cloud is true and useless.
    expect(explainSize(PLACEHOLDER | SPARSE, 1)).toMatch(/cloud/)
    // And a second name outranks everything: it is the one that changes what
    // deleting the row would do.
    expect(explainSize(LINKED | PLACEHOLDER | SPARSE, 2)).toMatch(/frees nothing/)
  })
})

describe('formatCount', () => {
  afterEach(() => {
    document.documentElement.lang = ''
  })

  it('separates thousands the way the interface language does', () => {
    document.documentElement.lang = 'en'
    expect(formatCount(1_234_567)).toBe('1,234,567')
  })

  it('follows the page, not the machine', () => {
    // German is on no machine this runs on by default, so a count that comes
    // out with dots came from `<html lang>` and nowhere else.
    document.documentElement.lang = 'de'
    expect(formatCount(1_234_567)).toBe('1.234.567')
  })
})

describe('formatDuration', () => {
  it('keeps tenths only while they matter', () => {
    expect(formatDuration(16)).toBe('under 0.1 s')
    expect(formatDuration(840)).toBe('0.8 s')
    expect(formatDuration(9_949)).toBe('9.9 s')
    expect(formatDuration(12_400)).toBe('12 s')
  })

  it('switches to minutes past one', () => {
    expect(formatDuration(59_600)).toBe('1 min')
    expect(formatDuration(125_000)).toBe('2 min 5 s')
  })

  it('refuses a number that is not a duration', () => {
    expect(formatDuration(-1)).toBe('—')
    expect(formatDuration(Number.NaN)).toBe('—')
  })
})

describe('describeScan', () => {
  it('names the scanner and what it cost', () => {
    expect(describeScan('mft', 2_100)).toBe('read from the MFT in 2.1 s')
    expect(describeScan('walk', 72_000)).toBe('walked folder by folder in 1 min 12 s')
  })

  it('leaves the time out when it was not measured', () => {
    expect(describeScan('walk', 0)).toBe('walked folder by folder')
  })
})

describe('formatAgo', () => {
  const now = Date.UTC(2026, 9, 4, 12, 0, 0)

  it('names the age a status line needs, coarsely', () => {
    expect(formatAgo(now - 20_000, now)).toBe('just now')
    expect(formatAgo(now - 5 * 60_000, now)).toBe('5 min ago')
    expect(formatAgo(now - 3 * 3_600_000, now)).toBe('3 h ago')
    expect(formatAgo(now - 30 * 3_600_000, now)).toBe('yesterday')
    expect(formatAgo(now - 5 * 86_400_000, now)).toBe('5 days ago')
  })
})

describe('describeFreshness', () => {
  const now = Date.UTC(2026, 9, 4, 12, 0, 0)
  const facts = { mode: 'watching' as const, savedAt: null, catchingUp: false, note: null, reading: false, readAt: now - 60_000 }

  it('names the source that keeps the picture current, and how far it reaches', () => {
    expect(describeFreshness({ ...facts, mode: 'journal' }, now)).toMatchObject({ tone: 'good', label: 'fresh · USN' })
    const watching = describeFreshness(facts, now)
    expect(watching).toMatchObject({ tone: 'good', label: 'watching' })
    expect(watching?.detail).toContain('next start reads the folder again')
  })

  it('says how old a saved picture is while it is read again', () => {
    const line = describeFreshness({ ...facts, reading: true, savedAt: now - 2 * 3_600_000 }, now)
    expect(line).toMatchObject({ tone: 'info', label: 'saved 2 h ago · reading again' })
  })

  it('says when a picture nothing keeps was read, and why nothing keeps it', () => {
    const line = describeFreshness({ ...facts, mode: 'still', note: 'the watcher stopped: the folder was deleted' }, now)
    expect(line?.tone).toBe('neutral')
    expect(line?.label).toMatch(/^scanned at /)
    expect(line?.detail).toContain('The watcher stopped: the folder was deleted.')
  })

  it('puts catching up before everything else', () => {
    expect(describeFreshness({ ...facts, mode: 'journal', catchingUp: true, savedAt: now }, now)?.label).toBe('catching up · USN')
  })

  it('says nothing when nothing is open', () => {
    expect(describeFreshness({ ...facts, mode: 'none' }, now)).toBeNull()
  })
})

describe('describeChanges', () => {
  const now = Date.UTC(2026, 9, 4, 12, 0, 0)

  it('says what was written and what the sizes mean', () => {
    const parts = describeChanges({ created: 1204, written: 1, deleted: 0, reachesBackTo: null, covers: true }, CHANGE_SPANS.day, now)
    expect(parts[0]).toBe(`Written in the last day: ${formatCount(1204)} files new, 1 file written to`)
    expect(parts).toContain('sizes are what the files hold now, not what they grew by')
    expect(parts.some((part) => part.includes('deleted'))).toBe(false)
  })

  it('says when the journal does not reach back as far as asked', () => {
    const parts = describeChanges({ created: 1, written: 0, deleted: 3, reachesBackTo: now - 3_600_000, covers: false }, CHANGE_SPANS.week, now)
    expect(parts[0]).toContain('the last week')
    expect(parts).toContain('3 deleted')
    expect(parts.at(-1)).toMatch(/^the change journal reaches back only to /)
  })
})

describe('formatSigned', () => {
  it('signs growth and shrinkage with a plus and a true minus', () => {
    expect(formatSigned(1.2 * 1024 ** 3)).toBe('+1.2 GB')
    expect(formatSigned(-300 * 1024 ** 2)).toBe('−300 MB')
    expect(formatSigned(0)).toBe('0 B')
    expect(formatSigned(Number.NaN)).toBe('—')
  })
})

describe('wayOf', () => {
  it('reads the balance, not the movement', () => {
    expect(wayOf(10, 20)).toBe('grew')
    expect(wayOf(20, 10)).toBe('freed')
    // A folder where as much went as came is even on balance, however much
    // happened in it: its tile is large and neutral, not red or green.
    expect(wayOf(500, 500)).toBe('even')
  })
})

const GB = 1024 ** 3
const size = (gigabytes: number) => ({ logical: gigabytes * GB, allocated: gigabytes * GB })

const comparison: ComparisonFacts = {
  then: Date.UTC(2026, 9, 5, 9, 12),
  now: Date.UTC(2026, 9, 8, 14, 30),
  toNow: true,
  totals: { before: size(147), now: size(106), grown: size(1.2), freed: size(42.2), new: 3, gone: 31_376, changed: 1 },
}

describe('describeComparison', () => {
  const now = Date.UTC(2026, 9, 8, 15, 0)

  it('says what grew and what was freed before it says the net', () => {
    const parts = describeComparison(comparison, 'allocated', now)
    expect(parts[0]).toMatch(/^Since .+: 1\.2 GB grew, 42 GB freed$/)
    expect(parts[1]).toBe('net −41 GB')
    expect(parts[2]).toBe(`3 new · ${formatCount(31_376)} gone · 1 file resized`)
  })

  it('names both ends when the later picture is a snapshot too', () => {
    const parts = describeComparison({ ...comparison, toNow: false }, 'allocated', now)
    expect(parts[0]).toContain('→')
  })

  it('says plainly when nothing changed size', () => {
    const still = { ...comparison, totals: { ...comparison.totals, grown: size(0), freed: size(0), new: 0, gone: 0, changed: 0 } }
    expect(describeComparison(still, 'allocated', now)).toEqual([expect.stringMatching(/^Nothing changed size since /)])
  })

  it('does not tell a folder below the root, whose counts it does not know, that nothing changed', () => {
    // Found on screen: inside a build folder that went whole, the strip said
    // "Nothing changed size" above seventy megabytes gone.
    const inside = { ...comparison, totals: { ...comparison.totals, grown: size(0), freed: size(0.07), new: 0, gone: 0, changed: 0 } }
    expect(describeComparison(inside, 'allocated', now)[0]).toMatch(/^Since .+: 0 B grew, 72 MB freed$/)
  })
})

describe('describeSnapshot', () => {
  it('names the kind of picture and its moment', () => {
    const now = Date.UTC(2026, 9, 8, 15, 0)
    expect(describeSnapshot({ kind: 'last', at: now - 3_600_000 }, now)).toMatch(/^Last time · /)
    expect(describeSnapshot({ kind: 'taken', at: null }, now)).toBe('Taken · an unknown moment')
  })
})

describe('describePlace', () => {
  it('says whether a place went whole or held loose files', () => {
    expect(describePlace({ kind: 'gone', isDirectory: true, entries: 31_376 })).toBe(`folder gone · ${formatCount(31_376)} entries`)
    expect(describePlace({ kind: 'new', isDirectory: true, entries: 1 })).toBe('new folder · 1 entry')
    expect(describePlace({ kind: 'files', isDirectory: true, entries: 12 })).toBe('files here · 12 changed')
    expect(describePlace({ kind: 'replaced', isDirectory: false, entries: 1 })).toBe('a file where a folder was')
  })
})

describe('reportMarkdown', () => {
  const report: ReportFacts = {
    ...comparison,
    root: 'D:/work',
    freed: [
      { path: 'D:/work/app/node_modules', isDirectory: true, kind: 'gone', freed: size(30), grown: size(0), entries: 31_000 },
      { path: 'D:/work/odd|name', isDirectory: true, kind: 'files', freed: size(2), grown: size(0.5), entries: 4 },
    ],
    freedRest: { places: 12, bytes: 10.2 * GB },
    grown: [{ path: 'D:/work/downloads', isDirectory: true, kind: 'new', freed: size(0), grown: size(0.7), entries: 2 }],
    grownRest: { places: 0, bytes: 0 },
  }
  const now = Date.UTC(2026, 9, 8, 15, 0)

  it('heads the page with what came back, what was written and the net', () => {
    const text = reportMarkdown(report, 'allocated', now)
    expect(text.split('\n')[0]).toBe('# What changed in D:/work')
    expect(text).toContain('| Came back | 42 GB |')
    expect(text).toContain('| Written meanwhile | 1.2 GB |')
    expect(text).toContain('| Net | −41 GB (147 GB → 106 GB) |')
  })

  it('lists each place once, with the rest added up', () => {
    const text = reportMarkdown(report, 'allocated', now)
    expect(text).toContain(`| 30 GB | ${'`'}app/node_modules${'`'} | folder gone · ${formatCount(31_000)} entries |`)
    expect(text).toContain('| 10 GB | 12 more places | |')
    expect(text).toContain('## What grew meanwhile')
  })

  it('keeps a pipe in a path from ending the cell', () => {
    const text = reportMarkdown(report, 'allocated', now)
    expect(text).toContain(String.raw`${'`'}odd\|name${'`'}`)
    expect(text).not.toContain('odd|name')
  })
})

describe('relativeTo', () => {
  it('reads a path inside the folder from the folder', () => {
    expect(relativeTo('C:\\work', 'C:\\work\\app\\target')).toBe('app\\target')
    expect(relativeTo('C:\\work\\', 'C:\\work\\app')).toBe('app')
    expect(relativeTo('/home/me', '/home/me/a/b')).toBe('a/b')
  })

  it('names the folder itself by its name, and leaves a path outside alone', () => {
    expect(relativeTo('C:\\work', 'C:\\work')).toBe('work')
    expect(relativeTo('C:\\', 'C:\\')).toBe('C:')
    expect(relativeTo('C:\\work', 'C:\\workshop\\a')).toBe('C:\\workshop\\a')
  })
})
