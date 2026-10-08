/**
 * Turning the core's numbers into what a person reads.
 *
 * Kept apart from the components so it can be tested without a DOM: these are
 * the numbers the whole product is an argument about, and a size rendered wrong
 * is worse than one not rendered at all.
 */

import { documentLocale } from 'dowel-ui'

/**
 * Bytes as a person reads them: `1.2 GB`.
 *
 * Binary steps with decimal names, which is what every disk tool on Windows
 * does and what Explorer's own properties dialog shows. Being consistent with
 * the machine the user is looking at beats being right about SI, because the
 * number they will compare this against is Explorer's.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—'
  if (bytes === 0) return '0 B'

  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB']
  let step = Math.min(Math.floor(Math.log2(bytes) / 10), units.length - 1)
  let value = bytes / 1024 ** step

  // One decimal from a kilobyte up, none for bare bytes: "1.5 KB" is useful,
  // "1536.0 B" is noise. Three significant figures below ten so a 1.2 GB folder
  // does not read as 1 GB beside a 1.9 GB one.
  let digits = step === 0 ? 0 : value < 10 ? 1 : 0

  // Rounding can push a value up into the next unit, and the step was chosen
  // before the rounding: 1023.6 GB renders as "1024 GB", which is a unit nobody
  // writes. Carry it. Seen on the first screen built — a 1.8 TB drive reported
  // "1024 GB used".
  if (Number(value.toFixed(digits)) >= 1024 && step < units.length - 1) {
    step += 1
    value /= 1024
    digits = value < 10 ? 1 : 0
  }

  return `${value.toFixed(digits)} ${units[step]}`
}

/**
 * A count with thousands separators, in the language the interface speaks.
 *
 * Not the machine's: a bare `toLocaleString()` asks the browser, and an
 * English window on a Russian machine then wrote `1 234 567 entries` beside
 * `1.2 GB` - two conventions on one line. `documentLocale()` reads
 * `<html lang>`, which is where the page says what it is written in.
 */
export function formatCount(count: number): string {
  return count.toLocaleString(documentLocale())
}

/**
 * How long ago, in words: `today`, `3 days`, `2 years`.
 *
 * Coarse on purpose. The question this answers is "is this layer live or
 * fossilized", and an exact date invites reading precision into a number that
 * is the maximum mtime of a whole subtree.
 */
export function formatAge(millis: number | null, now: number = Date.now()): string {
  if (millis === null || !Number.isFinite(millis)) return '—'

  const days = Math.floor((now - millis) / 86_400_000)
  if (days < 0) return 'ahead'
  if (days === 0) return 'today'
  if (days === 1) return 'yesterday'
  if (days < 30) return `${days} days`

  // Years are counted before months are ruled out. Handing off at "twelve
  // 30-day months" and picking up at "one 365-day year" leaves the five days
  // between them belonging to neither: 360 days rendered as "0 years", which is
  // a row claiming to be from the future. Found on screen, on a real list.
  const years = Math.floor(days / 365)
  if (years >= 1) return years === 1 ? '1 year' : `${years} years`

  const months = Math.max(Math.floor(days / 30), 1)
  return months === 1 ? '1 month' : `${months} months`
}

/**
 * What share of a total something takes, as a percentage string.
 *
 * A total of zero gives `0%` rather than `NaN%`: an empty scan is a real state
 * and the first screen should not show arithmetic.
 */
export function formatShare(part: number, total: number): string {
  if (total <= 0) return '0%'
  const share = (part / total) * 100
  // Below a tenth of a percent, say so rather than rounding to zero — a row
  // reading "0.0%" beside a real number looks like a bug.
  if (share > 0 && share < 0.1) return '<0.1%'
  return `${share.toFixed(share < 10 ? 1 : 0)}%`
}

/**
 * The gap between what a folder reads as and what it occupies, when it is worth
 * mentioning.
 *
 * Returns `null` when the two are close enough that saying anything would be
 * noise. This is the product's own argument made visible: a folder of a hundred
 * thousand tiny files costs far more than it reads, and a sparse virtual disk
 * costs far less.
 *
 * An entry that occupies *nothing* and reads as something is the extreme of the
 * second case, not an absence of one: a cloud placeholder, or a second name for
 * bytes counted elsewhere. It used to return `null` here, which left the two
 * numbers a reader could see with no account of themselves at all.
 */
export function describeOverhead(logical: number, allocated: number): string | null {
  if (logical === 0) return null
  if (allocated === 0) return `${formatBytes(logical)} that occupies nothing on this disk`

  const ratio = allocated / logical
  if (ratio >= 1.1) return `${formatBytes(allocated - logical)} more on disk than it reads`
  if (ratio <= 0.9) return `${formatBytes(logical - allocated)} less on disk than it reads`
  return null
}

/**
 * Why this entry's two sizes are what they are, in words, or `null` when there
 * is nothing unusual to report.
 *
 * Ordered by how surprising each one is rather than by bit. A reader looking at
 * a row that reads 5 GB and occupies nothing wants the sentence that explains
 * *that*, and a file can carry several of these at once — a cloud placeholder
 * is sparse as well, and reporting "sparse" to someone whose file is in the
 * cloud is true and useless.
 */
export function explainSize(traits: number, links: number | null): string | null {
  const has = (bit: number) => (traits & bit) === bit

  // The bit values are the core's; see `TRAIT` in core.ts. Written out here
  // rather than imported to keep this module free of the bridge — it is the
  // one place tested without a DOM or a Tauri runtime.
  const PLACEHOLDER = 1
  const COMPRESSED = 2
  const SPARSE = 4
  const LINKED = 8
  const COUNTED_HERE = 16
  const HOLDS_SHARED = 32

  if (has(LINKED) && !has(COUNTED_HERE)) {
    return 'another name for bytes counted elsewhere on this disk — deleting this frees nothing'
  }
  if (has(PLACEHOLDER)) {
    return 'stored in the cloud: it reads as its full size and occupies almost nothing here'
  }
  if (has(LINKED) && has(COUNTED_HERE)) {
    const names = links !== null && links > 1 ? `${formatCount(links)} names` : 'more than one name'
    return `the same bytes under ${names} on this disk, counted here`
  }
  if (has(COMPRESSED)) return 'compressed by NTFS: it occupies less than it reads'
  if (has(SPARSE)) return 'sparse: parts of it read as zeroes and occupy nothing'
  // Last, because it is the weakest claim: it says something is in here, not
  // that this entry is anything. It is also the one that keeps a folder reading
  // 0 B from being a mystery.
  if (has(HOLDS_SHARED)) return 'what is inside is named elsewhere too, and counted there'
  return null
}

/**
 * A duration as a person reads it: `0.8 s`, `12 s`, `2 min 5 s`.
 *
 * Tenths only under ten seconds. The number exists to compare the two
 * scanners, and the difference between them is seconds against minutes — a
 * tenth matters on the fast side and is noise on the slow one.
 */
export function formatDuration(millis: number): string {
  if (!Number.isFinite(millis) || millis < 0) return '—'
  // "0.0 s" reads as "did not happen"; a scan that fast still happened.
  if (millis < 100) return 'under 0.1 s'
  const seconds = millis / 1000
  if (seconds < 10) return `${seconds.toFixed(1)} s`
  const whole = Math.round(seconds)
  if (whole < 60) return `${whole} s`
  const minutes = Math.floor(whole / 60)
  const rest = whole % 60
  return rest === 0 ? `${minutes} min` : `${minutes} min ${rest} s`
}

/**
 * How a finished scan was read, in one line: `read from the MFT in 2.1 s`.
 *
 * Said on every result because the two scanners give the same numbers at very
 * different speeds, and a reader deciding whether "accelerate" is worth an
 * administrator's prompt needs to see what the slow way cost them.
 */
export function describeScan(scannedBy: string, elapsedMs: number): string {
  const took = elapsedMs > 0 ? ` in ${formatDuration(elapsedMs)}` : ''
  return scannedBy === 'mft' ? `read from the MFT${took}` : `walked folder by folder${took}`
}

/**
 * How long ago a moment was, in the words a status line uses.
 *
 * Coarse on purpose: "saved 2 h ago" is what decides whether the picture is
 * worth trusting, and the minute it was saved is in the tooltip.
 */
export function formatAgo(millis: number, now: number = Date.now()): string {
  const minutes = Math.floor((now - millis) / 60_000)
  if (minutes < 1) return 'just now'
  if (minutes < 60) return `${minutes} min ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} h ago`
  const days = Math.floor(hours / 24)
  return days === 1 ? 'yesterday' : `${days} days ago`
}

/**
 * A moment as a status line names it: the time alone today, the date and the
 * time before that.
 */
export function formatMoment(millis: number, now: number = Date.now()): string {
  const moment = new Date(millis)
  const time = moment.toLocaleTimeString(documentLocale(), { hour: '2-digit', minute: '2-digit' })
  if (moment.toDateString() === new Date(now).toDateString()) return time
  return `${moment.toLocaleDateString(documentLocale(), { day: 'numeric', month: 'short' })} ${time}`
}

/** The tone of a freshness line: a picture kept current, one being brought
 * up to date, or one nothing is keeping. */
export type FreshnessTone = 'good' | 'info' | 'neutral'

/** What the title bar says about how current the picture is. */
export interface FreshnessLine {
  tone: FreshnessTone
  label: string
  /** The whole story, for the tooltip. */
  detail: string
}

/** The parts of the session the line is written from. Mirrors `Freshness`
 * and the two facts beside it, without importing the bridge. */
export interface FreshnessFacts {
  mode: 'none' | 'journal' | 'watching' | 'still'
  savedAt: number | null
  catchingUp: boolean
  note: string | null
  /** A full read is running under the picture on screen. */
  reading: boolean
  /** When the folder was last read in full. */
  readAt: number | null
}

/**
 * The freshness line: how current the picture is, said honestly.
 *
 * Every state names its source, because the same picture can be seconds old
 * or a day old and look identical. A saved index on screen says how old it
 * is; a picture nothing is keeping says when it was read; a watched or
 * journalled one says what keeps it, and the tooltip says how far that
 * reaches - the watcher ends with the window, the journal does not.
 */
export function describeFreshness(facts: FreshnessFacts, now: number = Date.now()): FreshnessLine | null {
  const why = facts.note !== null && facts.note !== '' ? ` ${capitalise(facts.note)}.` : ''
  if (facts.catchingUp) {
    return {
      tone: 'info',
      label: 'catching up · USN',
      detail: 'Applying what changed while midda was closed, from the NTFS change journal.',
    }
  }
  if (facts.reading && facts.savedAt !== null) {
    return {
      tone: 'info',
      label: `saved ${formatAgo(facts.savedAt, now)} · reading again`,
      detail: `The index saved ${formatMoment(facts.savedAt, now)}, shown while the folder is read again.${why}`,
    }
  }
  if (facts.reading) {
    return { tone: 'info', label: 'reading again', detail: `The folder is being read again; the picture stays until the new one is ready.${why}` }
  }
  switch (facts.mode) {
    case 'journal':
      return {
        tone: 'good',
        label: 'fresh · USN',
        detail:
          'Every change on this volume is applied as it happens, from the NTFS change journal. The next start picks up where this one left off.',
      }
    case 'watching':
      return {
        tone: 'good',
        label: 'watching',
        detail: `Changes under this folder are applied while midda is open. The next start reads the folder again; Accelerate keeps the index fresh across restarts.${why}`,
      }
    case 'still': {
      const read = facts.readAt === null ? 'read once' : `scanned at ${formatMoment(facts.readAt, now)}`
      return { tone: 'neutral', label: read, detail: `Nothing is keeping this picture current.${why}` }
    }
    case 'none':
      return null
  }
}

function capitalise(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/** The spans the view of what changed offers, in hours. */
export const CHANGE_SPANS = { day: 24, week: 24 * 7 } as const

/**
 * What the view of recent changes says above its table: what was written,
 * what it holds now, and what the numbers cannot say.
 */
export function describeChanges(
  summary: { created: number; written: number; deleted: number; reachesBackTo: number | null; covers: boolean },
  hours: number,
  now: number = Date.now(),
): string[] {
  const span = hours === CHANGE_SPANS.day ? 'the last day' : hours === CHANGE_SPANS.week ? 'the last week' : `the last ${hours} h`
  const files = (count: number, what: string) => `${formatCount(count)} ${count === 1 ? 'file' : 'files'} ${what}`
  const parts = [`Written in ${span}: ${files(summary.created, 'new')}, ${files(summary.written, 'written to')}`]
  if (summary.deleted > 0) parts.push(`${formatCount(summary.deleted)} deleted`)
  // The one thing a reader would assume and must not: that a file written
  // to grew by all of what it holds.
  parts.push('sizes are what the files hold now, not what they grew by')
  if (!summary.covers && summary.reachesBackTo !== null) {
    parts.push(`the change journal reaches back only to ${formatMoment(summary.reachesBackTo, now)}`)
  }
  return parts
}

/**
 * A change in bytes, with its sign: `+1.2 GB`, `−300 MB`.
 *
 * The minus is the minus sign rather than a hyphen: it lines up with the plus
 * in a column of figures, and a hyphen before a number reads as a dash.
 */
export function formatSigned(bytes: number): string {
  if (!Number.isFinite(bytes)) return '—'
  if (bytes === 0) return '0 B'
  return `${bytes > 0 ? '+' : '−'}${formatBytes(Math.abs(bytes))}`
}

/** Which way an entry went between two moments, on balance. */
export type Way = 'grew' | 'freed' | 'even'

/** Which way something holding `before` then and `now` now went. */
export function wayOf(before: number, now: number): Way {
  return now > before ? 'grew' : now < before ? 'freed' : 'even'
}

/** The marks a row of a comparison carries, as the words on it. `changed`
 * has none: the change column says it, and a badge on half the rows would
 * say nothing. */
export const MARK_WORDS = { new: 'new', gone: 'gone', replaced: 'replaced', changed: null } as const

/** The totals of a comparison, as `describeComparison` reads them. Mirrors
 * `Totals` without importing the bridge. */
export interface ComparisonFacts {
  then: number | null
  now: number | null
  toNow: boolean
  totals: {
    before: { logical: number; allocated: number }
    now: { logical: number; allocated: number }
    grown: { logical: number; allocated: number }
    freed: { logical: number; allocated: number }
    new: number
    gone: number
    changed: number
  }
}

/** The span of a comparison in words: `since Oct 5 09:12`, or both ends
 * when the later one is a snapshot too. */
export function describeSpan(facts: Pick<ComparisonFacts, 'then' | 'now' | 'toNow'>, now: number = Date.now()): string {
  const then = facts.then === null ? 'an earlier picture' : formatMoment(facts.then, now)
  if (facts.toNow || facts.now === null) return `since ${then}`
  return `${then} → ${formatMoment(facts.now, now)}`
}

/**
 * What the strip above a comparison says: over what span, what grew, what
 * went, and the two together.
 *
 * Grown and freed are said apart before the net is: a folder that netted
 * zero may be where everything happened, and a net alone hides it.
 */
export function describeComparison(facts: ComparisonFacts, basis: 'allocated' | 'logical', now: number = Date.now()): string[] {
  const of = (size: { logical: number; allocated: number }) => (basis === 'allocated' ? size.allocated : size.logical)
  const { totals } = facts
  const span = describeSpan(facts, now)
  // Below the root a folder's own counts are not known, only its bytes: what
  // decides "nothing" is both, so a folder that lost seventy megabytes is
  // never told it did not change.
  if (of(totals.grown) === 0 && of(totals.freed) === 0 && totals.new + totals.gone + totals.changed === 0) return [`Nothing changed size ${span}`]
  const parts = [
    `${capitalise(span)}: ${formatBytes(of(totals.grown))} grew, ${formatBytes(of(totals.freed))} freed`,
    `net ${formatSigned(of(totals.now) - of(totals.before))}`,
  ]
  const counted = [
    totals.new > 0 ? `${formatCount(totals.new)} new` : null,
    totals.gone > 0 ? `${formatCount(totals.gone)} gone` : null,
    totals.changed > 0 ? `${formatCount(totals.changed)} ${totals.changed === 1 ? 'file' : 'files'} resized` : null,
  ].filter((part) => part !== null)
  if (counted.length > 0) parts.push(counted.join(' · '))
  return parts
}

/** How a snapshot is named in a list: what kind of picture, of when. */
export function describeSnapshot(snapshot: { kind: 'last' | 'taken'; at: number | null }, now: number = Date.now()): string {
  const when = snapshot.at === null ? 'an unknown moment' : formatMoment(snapshot.at, now)
  return snapshot.kind === 'last' ? `Last time · ${when}` : `Taken · ${when}`
}

/** What a place in a report is, in words. */
export function describePlace(place: { kind: 'gone' | 'new' | 'replaced' | 'files'; isDirectory: boolean; entries: number }): string {
  const entries = `${formatCount(place.entries)} ${place.entries === 1 ? 'entry' : 'entries'}`
  switch (place.kind) {
    case 'gone':
      return place.isDirectory ? `folder gone · ${entries}` : 'gone'
    case 'new':
      return place.isDirectory ? `new folder · ${entries}` : 'new'
    case 'replaced':
      return place.isDirectory ? 'a folder where a file was' : 'a file where a folder was'
    case 'files':
      return `files here · ${formatCount(place.entries)} changed`
  }
}

/** What the size a report is in is called. */
function basisWords(basis: 'allocated' | 'logical'): string {
  return basis === 'allocated' ? 'on disk' : 'by size'
}

/** A report's inputs, without importing the bridge. */
export interface ReportFacts extends ComparisonFacts {
  root: string
  freed: ReportPlace[]
  freedRest: { places: number; bytes: number }
  grown: ReportPlace[]
  grownRest: { places: number; bytes: number }
}

export interface ReportPlace {
  path: string
  isDirectory: boolean
  kind: 'gone' | 'new' | 'replaced' | 'files'
  freed: { logical: number; allocated: number }
  grown: { logical: number; allocated: number }
  entries: number
}

/**
 * The report of a comparison as Markdown, for a note or an issue: what came
 * back, what was written meanwhile, and the places each came from.
 *
 * Paths go in code spans, so a folder called `*` or `_build_` stays a name;
 * a pipe would end a table cell, and is escaped.
 */
export function reportMarkdown(facts: ReportFacts, basis: 'allocated' | 'logical', now: number = Date.now()): string {
  const of = (size: { logical: number; allocated: number }) => (basis === 'allocated' ? size.allocated : size.logical)
  const { totals } = facts
  const cell = (text: string) => text.replaceAll('|', String.raw`\|`)
  const code = (path: string) => (path.includes('`') ? `\`\` ${cell(path)} \`\`` : `\`${cell(path)}\``)
  const lines = [
    `# What changed in ${facts.root}`,
    '',
    `${capitalise(describeSpan(facts, now))} · ${basisWords(basis)}`,
    '',
    '| | |',
    '| --- | ---: |',
    `| Came back | ${formatBytes(of(totals.freed))} |`,
    `| Written meanwhile | ${formatBytes(of(totals.grown))} |`,
    `| Net | ${formatSigned(of(totals.now) - of(totals.before))} (${formatBytes(of(totals.before))} → ${formatBytes(of(totals.now))}) |`,
  ]
  const section = (title: string, places: ReportPlace[], rest: { places: number; bytes: number }, side: 'freed' | 'grown') => {
    if (places.length === 0) return
    lines.push('', `## ${title}`, '', `| ${side === 'freed' ? 'Freed' : 'Grew'} | Where | What |`, '| ---: | --- | --- |')
    for (const place of places) {
      lines.push(`| ${formatBytes(of(place[side]))} | ${code(relativeTo(facts.root, place.path))} | ${describePlace(place)} |`)
    }
    if (rest.places > 0) {
      lines.push(`| ${formatBytes(rest.bytes)} | ${formatCount(rest.places)} more ${rest.places === 1 ? 'place' : 'places'} | |`)
    }
  }
  section('Where it came back from', facts.freed, facts.freedRest, 'freed')
  section('What grew meanwhile', facts.grown, facts.grownRest, 'grown')
  return `${lines.join('\n')}\n`
}

/**
 * `path` as it reads inside `root`: `app\target` for `C:\work\app\target`
 * inside `C:\work`. The root itself reads as its own name, and a path
 * outside it as itself.
 *
 * A report is about one folder, named in its title; repeating that prefix on
 * every line pushed what each line is about past the edge of the page, where
 * it was cut off - the part a reader needed.
 */
export function relativeTo(root: string, path: string): string {
  const trimmed = root.replace(/[\\/]+$/, '')
  if (path === root || path === trimmed) return trimmed.split(/[\\/]/).at(-1) || trimmed
  if (path.startsWith(trimmed) && /^[\\/]/.test(path.slice(trimmed.length))) return path.slice(trimmed.length + 1)
  return path
}
