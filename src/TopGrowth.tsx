import { useEffect, useState } from 'react'

import { RowButton } from '@/components/ui/list-row'
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import { isStale, pick, topGrowth, type ComparisonSummary, type GrowerRow, type GrowerSide, type SizeBasis, type Top, type View } from '@/core'
import { CHANGE_SPANS, describeGrower, describeSpan, formatBytes, formatCount, formatGap, formatPerDay, formatRunway, formatSigned, relativeTo } from '@/format'

/** How many lines each side names before adding up the rest. The rule
 * names about fifty at most; twenty is what a reader reads. */
const LINES = 20

/** A day, in milliseconds: what a pace is per. */
const DAY_MS = 86_400_000

interface TopGrowthProps {
  /** A comparison, or what was written lately. */
  view: Extract<View, 'compare' | 'changes'>
  arena: number
  basis: SizeBasis
  /** The folder the view is of, as the title names it. */
  root: string
  /** The comparison, when that is the view. */
  comparison: ComparisonSummary | null
  /** The span of what was written, in hours. */
  hours: number
  /** A line was chosen: the window shows it in the view's tree. */
  onOpen: (id: number) => void
}

/**
 * Who is eating the disk: the few folders that account for the growth, the
 * most first, each counted once - and, in a comparison, the pace and how long
 * the free space lasts at it.
 *
 * A folder is named as deep as its growth is one sizeable thing, and only for
 * what it grew besides the folders named under it; what grew in pieces too
 * small to name is one line at the end. The lines and that line add up to the
 * whole (ADR 0011). Each line opens its folder in the tree.
 */
export function TopGrowth({ view, arena, basis, root, comparison, hours, onOpen }: TopGrowthProps) {
  const [top, setTop] = useState<Top | null>(null)
  const [failure, setFailure] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    topGrowth(view, arena, basis, LINES)
      .then((next) => {
        if (current) setTop(next)
      })
      .catch((cause: unknown) => {
        if (current && !isStale(cause)) setFailure(String(cause))
      })
    return () => {
      current = false
    }
  }, [view, arena, basis])

  if (failure !== null) return <p className="p-4 text-sm text-bad">{failure}</p>

  const written = view === 'changes'
  // A pace per line is the same division as the whole's: the line's own
  // share over the span. Only where the whole has one.
  const perDay = (bytes: number) => (top?.pace != null && top.spanMs !== null && top.spanMs > 0 ? formatPerDay((bytes * DAY_MS) / top.spanMs) : null)
  const span =
    comparison !== null
      ? describeSpan(comparison)
      : hours === CHANGE_SPANS.day
        ? 'in the last day'
        : hours === CHANGE_SPANS.week
          ? 'in the last week'
          : `in the last ${hours} h`

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-6 py-5">
        <header className="min-w-0">
          <h2 className="truncate text-lg font-semibold" title={root}>
            {written ? `Where the most was written in ${root}` : `What is eating ${root}`}
          </h2>
          <p className="text-sm text-dim">
            {capitalised(span)} · {basis === 'allocated' ? 'on disk' : 'by size'}
          </p>
        </header>

        {/* What was written says its counts, and that its sizes are what
            files hold now, in the strip above: not said twice. */}
        {comparison !== null && <ComparisonFigures comparison={comparison} top={top} basis={basis} root={root} />}

        <Lines
          title={written ? 'Where it was written' : 'What grew'}
          side={written ? 'written' : 'grew'}
          root={root}
          lines={top?.grew ?? null}
          perDay={perDay}
          onOpen={onOpen}
        />
        {top?.freed != null && <Lines title="What was freed" side="freed" root={root} lines={top.freed} perDay={() => null} onOpen={onOpen} />}
      </div>
    </div>
  )
}

interface ComparisonFiguresProps {
  comparison: ComparisonSummary
  top: Top | null
  basis: SizeBasis
  root: string
}

/** The three figures over a comparison: how much, how fast, how long. */
function ComparisonFigures({ comparison, top, basis, root }: ComparisonFiguresProps) {
  const { totals } = comparison
  const net = pick(totals.now, basis) - pick(totals.before, basis)
  const pace = top?.pace ?? null
  const spanMs = top?.spanMs ?? null
  // The whole volume, when the folder is the volume: then its pace is the
  // volume's. Any other folder is one of the things filling it.
  const volume = /^[A-Za-z]:\\?$/.test(root)

  return (
    <StatRow>
      <StatTile
        label="Net growth"
        value={formatSigned(net)}
        // The two halves of the net, not a direction: one colour for both
        // would say the freed half went the same way as the grown one.
        delta={`${formatBytes(pick(totals.grown, basis))} grew · ${formatBytes(pick(totals.freed, basis))} freed`}
      />
      <StatTile
        label="Pace"
        value={pace === null ? '—' : formatPerDay(pace.perDay)}
        delta={
          spanMs === null
            ? undefined
            : pace === null
              ? `a pace needs pictures a day apart; these are ${formatGap(spanMs)} apart`
              : `over ${formatGap(spanMs)}`
        }
      />
      {/* Only against now: free space is a fact about the present, and two
          old pictures' pace set against it would be a forecast from then. */}
      {pace !== null && comparison.toNow && top?.free != null && (
        <StatTile
          label="Free space lasts"
          value={pace.fullInDays === null ? 'not filling' : formatRunway(pace.fullInDays)}
          // Under a month is news to act on; said in the line under the
          // figure, which is where a tile says which way things are going.
          deltaTone={pace.fullInDays !== null && pace.fullInDays < 30 ? 'bad' : 'default'}
          delta={
            pace.fullInDays === null
              ? `${formatBytes(top.free)} free · it shrank on disk, on balance`
              : volume
                ? `${formatBytes(top.free)} free, at this pace`
                : `${formatBytes(top.free)} free on the volume, at this folder's pace alone`
          }
        />
      )}
    </StatRow>
  )
}

interface LinesProps {
  title: string
  side: 'grew' | 'freed' | 'written'
  root: string
  lines: GrowerSide | null
  /** The line's pace, when the whole has one. */
  perDay: (bytes: number) => string | null
  onOpen: (id: number) => void
}

/** One side: the lines, the most first, and what they leave unnamed. */
function Lines({ title, side, root, lines, perDay, onOpen }: LinesProps) {
  if (lines !== null && lines.growers.length === 0) {
    return (
      <section>
        <h3 className="mb-2 text-sm font-semibold">{title}</h3>
        <p className="text-sm text-dim">{side === 'freed' ? 'Nothing was freed.' : side === 'written' ? 'Nothing was written.' : 'Nothing grew.'}</p>
      </section>
    )
  }
  const largest = lines?.growers[0]?.own ?? 0
  const amount = (bytes: number) => (side === 'grew' ? formatSigned(bytes) : formatBytes(bytes))
  const tone = side === 'freed' ? 'text-good' : side === 'grew' ? 'text-bad' : 'text-text'
  const fill = side === 'freed' ? 'bg-good' : side === 'grew' ? 'bg-bad' : 'bg-accent'
  return (
    <section>
      <h3 className="mb-2 text-sm font-semibold">{title}</h3>
      <ul className="divide-y divide-line border-y border-line">
        {(lines?.growers ?? []).map((line: GrowerRow) => {
          const pace = perDay(line.own)
          return (
            <li key={`${line.loose ? 'files' : 'folder'}:${line.id}`}>
              <RowButton
                onClick={() => onOpen(line.id)}
                title={`${line.path}\nOpen it in the tree`}
                start={<span className={`tabular w-24 text-right font-medium ${tone}`}>{amount(line.own)}</span>}
                description={pace === null ? describeGrower(line, side) : `${describeGrower(line, side)} · ${pace}`}
                end={
                  // The line's share of the largest, so a glance down the
                  // column is the shape of the answer.
                  <span className="h-1 w-16 overflow-hidden rounded-full bg-soft" aria-hidden>
                    <span className={`block h-full ${fill}`} style={{ width: largest > 0 ? `${(line.own / largest) * 100}%` : '0%' }} />
                  </span>
                }
              >
                <span className="font-mono text-xs">{line.loose ? `${relativeTo(root, line.path)} (its own files)` : relativeTo(root, line.path)}</span>
              </RowButton>
            </li>
          )
        })}
        {lines !== null && lines.more.places > 0 && (
          <li className="flex items-center gap-3 px-3 py-2 text-sm text-dim">
            <span className="tabular w-24 text-right">{amount(lines.more.bytes)}</span>
            <span>
              {formatCount(lines.more.places)} more {lines.more.places === 1 ? 'folder' : 'folders'}
            </span>
          </li>
        )}
        {/* What no line names, said rather than dropped: without it the
            lines would read as all of it. In a comparison it is on balance -
            smaller growth less what was freed - which is what makes the lines
            and it add up to the net; the freed side would only say the same
            sum again. What was written only adds, so there it is pieces. */}
        {lines !== null && lines.rest !== 0 && side !== 'freed' && (
          <li className="flex items-center gap-3 px-3 py-2 text-sm text-dim">
            <span className="tabular w-24 text-right">{side === 'written' ? formatBytes(lines.rest) : formatSigned(lines.rest)}</span>
            <span>
              {side === 'written'
                ? `elsewhere, in pieces under ${formatBytes(lines.threshold)} each`
                : `everything else, on balance: smaller growth, less what was freed`}
            </span>
          </li>
        )}
      </ul>
    </section>
  )
}

function capitalised(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}
