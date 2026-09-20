/**
 * Turning the core's numbers into what a person reads.
 *
 * Kept apart from the components so it can be tested without a DOM: these are
 * the numbers the whole product is an argument about, and a size rendered wrong
 * is worse than one not rendered at all.
 */

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
 * A count with thousands separators, in the reader's own locale.
 */
export function formatCount(count: number): string {
  return count.toLocaleString()
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
