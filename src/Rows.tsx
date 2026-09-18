import type { Row, ScanResult, SizeBasis } from '@/core'
import { sizeOf } from '@/core'
import { describeOverhead, formatAge, formatBytes, formatCount, formatShare } from '@/format'

interface RowsProps {
  result: ScanResult
  /** The scan root down to what is on screen. */
  trail: Row[]
  showing: Row
  rows: Row[]
  basis: SizeBasis
  onDescend: (row: Row) => void
  onClimb: (depth: number) => void
}

/**
 * What the scan found, one level at a time.
 *
 * A plain list rather than a virtualized table: this version shows one level's
 * children, which is hundreds of rows at worst. The virtualized table over the
 * whole tree, with sorting and columns, is v0.2.0 — building it here would mean
 * building it before there is a screen to judge it on.
 */
export function Rows({ result, trail, showing, rows, basis, onDescend, onClimb }: RowsProps) {
  const total = sizeOf(showing, basis)
  const overhead = describeOverhead(showing.logical, showing.allocated)
  // `skipped` counts the whole scan, so it is only true of the scan root.
  const showsSkipped = trail.length === 1 && result.skipped > 0

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line px-4 py-2.5 text-sm">
        <nav className="flex min-w-0 items-center gap-1" aria-label="Where you are">
          {trail.map((step, depth) => (
            <span key={step.id} className="flex min-w-0 items-center gap-1">
              {depth > 0 && <span className="text-dim">/</span>}
              {depth === trail.length - 1 ? (
                <span className="truncate font-medium">{step.name}</span>
              ) : (
                <button type="button" onClick={() => onClimb(depth)} className="cursor-pointer truncate text-dim hover:text-text">
                  {step.name}
                </button>
              )}
            </span>
          ))}
        </nav>

        <span className="tabular ml-auto shrink-0 text-dim">
          <span className="font-medium text-text">{formatBytes(total)}</span> · {formatCount(showing.entries)} entries
        </span>
      </div>

      {(overhead !== null || showsSkipped) && (
        <div className="flex shrink-0 flex-wrap gap-x-4 border-b border-line bg-soft/40 px-4 py-1.5 text-xs text-dim">
          {overhead !== null && <span>{overhead}</span>}
          {result.clusterBytes !== null && overhead !== null && <span>cluster {formatBytes(result.clusterBytes)}</span>}
          {/* A scan of a system volume always refuses something. Saying so is
              the difference between a total that is short and a total that is
              quietly wrong.
              Only at the scan root: the count is for the whole scan, and
              repeating it inside every folder would read as a claim about that
              folder — which nothing here knows. */}
          {showsSkipped && <span>{formatCount(result.skipped)} entries could not be read in this scan</span>}
        </div>
      )}

      <ul className="min-h-0 flex-1 overflow-y-auto">
        {rows.length === 0 && <li className="px-4 py-6 text-sm text-dim">This folder is empty.</li>}

        {rows.map((row) => {
          const size = sizeOf(row, basis)
          const gap = describeOverhead(row.logical, row.allocated)

          return (
            <li key={row.id}>
              <button
                type="button"
                onClick={() => onDescend(row)}
                disabled={!row.isDirectory}
                className={
                  row.isDirectory
                    ? 'flex w-full cursor-pointer items-center gap-3 px-4 py-1.5 text-left text-sm hover:bg-soft'
                    : 'flex w-full items-center gap-3 px-4 py-1.5 text-left text-sm'
                }
                title={gap === null ? row.path : `${row.path}\n${gap}`}
              >
                {/* The bar is the row's share of what is on screen, so a glance
                    down the list is the shape of this folder. */}
                <span className="h-1 w-16 shrink-0 overflow-hidden rounded-full bg-soft" aria-hidden>
                  <span className="block h-full bg-accent" style={{ width: total > 0 ? `${(size / total) * 100}%` : '0%' }} />
                </span>

                <span className={row.isDirectory ? 'min-w-0 flex-1 truncate font-medium' : 'min-w-0 flex-1 truncate text-dim'}>
                  {row.name}
                  {row.isDirectory && <span className="text-dim">/</span>}
                </span>

                <span className="tabular w-16 shrink-0 text-right text-xs text-dim">{formatShare(size, total)}</span>
                <span className="tabular w-24 shrink-0 text-right">{formatBytes(size)}</span>
                <span className="tabular w-20 shrink-0 text-right text-xs text-dim">{formatAge(row.subtreeModified)}</span>
              </button>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
