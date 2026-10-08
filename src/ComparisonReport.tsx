import { useCallback, useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { RowButton } from '@/components/ui/list-row'
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import { comparisonReport, isStale, pick, type ComparisonSummary, type PlaceRow, type Report, type Rest, type SizeBasis } from '@/core'
import { describePlace, describeSpan, formatBytes, formatCount, formatSigned, relativeTo, reportMarkdown } from '@/format'

/** How many places each list names before adding up the rest. */
const PLACES = 30

interface ComparisonReportProps {
  summary: ComparisonSummary
  basis: SizeBasis
  /** The folder compared, as the title names it. */
  root: string
  /** A place was chosen: the window shows it in the comparison's tree. */
  onOpen: (id: number) => void
}

/**
 * The report of a comparison: one page, the answer to "how much came back,
 * and from where".
 *
 * Three figures — what came back, what was written meanwhile, the two
 * together — then the places each came from, the most first. A place is
 * where bytes went or came once: a folder that went whole is one line, not
 * its two hundred thousand files, and the lines add up to the figures above
 * them (ADR 0010). Each line opens that place in the comparison's tree.
 *
 * The page copies as Markdown, because a report is what one keeps: in a
 * note, in an issue, beside the next one.
 */
export function ComparisonReport({ summary, basis, root, onOpen }: ComparisonReportProps) {
  const [report, setReport] = useState<Report | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [copied, setCopied] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    comparisonReport(summary.arena, basis, PLACES)
      .then((next) => {
        if (current) setReport(next)
      })
      .catch((cause: unknown) => {
        if (current && !isStale(cause)) setFailure(String(cause))
      })
    return () => {
      current = false
    }
  }, [summary.arena, basis])

  const { totals } = summary
  const freed = pick(totals.freed, basis)
  const grown = pick(totals.grown, basis)
  const net = pick(totals.now, basis) - pick(totals.before, basis)

  const copy = useCallback(() => {
    if (report === null) return
    const text = reportMarkdown({ ...summary, root, ...report }, basis)
    navigator.clipboard
      .writeText(text)
      .then(() => setCopied('Copied'))
      .catch(() => setCopied('Could not copy'))
  }, [report, summary, root, basis])

  // The confirmation clears itself, as the picture's does.
  useEffect(() => {
    if (copied === null) return
    const timer = window.setTimeout(() => setCopied(null), 2_000)
    return () => window.clearTimeout(timer)
  }, [copied])

  if (failure !== null) return <p className="p-4 text-sm text-bad">{failure}</p>

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-6 py-5">
        <header className="flex flex-wrap items-start gap-x-4 gap-y-2">
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-lg font-semibold" title={root}>
              What changed in {root}
            </h2>
            <p className="text-sm text-dim">
              {capitalised(describeSpan(summary))} · {basis === 'allocated' ? 'on disk' : 'by size'}
            </p>
          </div>
          <span className="flex items-center gap-2">
            {copied !== null && (
              <span aria-live="polite" className="text-xs text-dim">
                {copied}
              </span>
            )}
            <Button variant="ghost" size="sm" disabled={report === null} onClick={copy} title="Copy the report as Markdown, for a note or an issue">
              Copy as Markdown
            </Button>
          </span>
        </header>

        <StatRow>
          {/* The counts say only what they know: how many entries went or
              came. Files that changed size are counted once, in the strip,
              without a side - a file can be either. */}
          <StatTile label="Came back" value={formatBytes(freed)} tone="accent" delta={`${entries(totals.gone)} gone`} />
          <StatTile label="Written meanwhile" value={formatBytes(grown)} delta={`${entries(totals.new)} new`} />
          <StatTile
            label="Net"
            value={formatSigned(net)}
            delta={`${formatBytes(pick(totals.before, basis))} → ${formatBytes(pick(totals.now, basis))}`}
            deltaTone={net < 0 ? 'good' : net > 0 ? 'bad' : 'default'}
          />
        </StatRow>

        <Places title="Where it came back from" side="freed" root={root} basis={basis} places={report?.freed ?? null} rest={report?.freedRest ?? null} onOpen={onOpen} />
        <Places title="What grew meanwhile" side="grown" root={root} basis={basis} places={report?.grown ?? null} rest={report?.grownRest ?? null} onOpen={onOpen} />
      </div>
    </div>
  )
}

interface PlacesProps {
  title: string
  side: 'freed' | 'grown'
  /** The folder compared: each place is named from inside it. */
  root: string
  basis: SizeBasis
  places: PlaceRow[] | null
  rest: Rest | null
  onOpen: (id: number) => void
}

/** One side of the report: the places bytes went from, or came to. */
function Places({ title, side, root, basis, places, rest, onOpen }: PlacesProps) {
  if (places !== null && places.length === 0) {
    return (
      <section>
        <h3 className="mb-2 text-sm font-semibold">{title}</h3>
        <p className="text-sm text-dim">{side === 'freed' ? 'Nothing went.' : 'Nothing grew.'}</p>
      </section>
    )
  }
  return (
    <section>
      <h3 className="mb-2 text-sm font-semibold">{title}</h3>
      <ul className="divide-y divide-line border-y border-line">
        {(places ?? []).map((place) => (
          <li key={`${place.kind}:${place.id}`}>
            <RowButton
              onClick={() => onOpen(place.id)}
              title={`${place.path}\nOpen it in the comparison`}
              start={<span className={`tabular w-20 text-right font-medium ${side === 'freed' ? 'text-good' : 'text-bad'}`}>{formatBytes(pick(place[side], basis))}</span>}
              description={describePlace(place)}
            >
              <span className="font-mono text-xs">{relativeTo(root, place.path)}</span>
            </RowButton>
          </li>
        ))}
        {rest !== null && rest.places > 0 && (
          <li className="flex items-center gap-3 px-3 py-2 text-sm text-dim">
            <span className="tabular w-20 text-right">{formatBytes(rest.bytes)}</span>
            <span>
              {formatCount(rest.places)} more {rest.places === 1 ? 'place' : 'places'}
            </span>
          </li>
        )}
      </ul>
    </section>
  )
}

/** A count of entries, in the singular when it is one. */
function entries(count: number): string {
  return `${formatCount(count)} ${count === 1 ? 'entry' : 'entries'}`
}

function capitalised(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}
