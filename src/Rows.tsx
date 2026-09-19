import { EntryTable } from '@/EntryTable'
import { Treemap } from '@/Treemap'
import type { Row, ScanResult, SizeBasis, Sort, SortKey } from '@/core'
import { sizeOf } from '@/core'
import { describeOverhead, formatBytes, formatCount } from '@/format'

interface RowsProps {
  result: ScanResult
  /** The scan root down to what is on screen. */
  trail: Row[]
  showing: Row
  sort: Sort
  basis: SizeBasis
  /** The entry both views have highlighted, if any. */
  selected: Row | null
  onSortChange: (key: SortKey) => void
  onDescend: (row: Row) => void
  onDescendById: (id: number) => void
  onClimb: (depth: number) => void
  onSelect: (row: Row | null) => void
  onSelectById: (id: number | null) => void
}

/**
 * What the scan found: where you are, what it costs, and the two views of what
 * is inside.
 *
 * Both at once, deliberately. The table answers "which of these is biggest" one
 * row at a time and can be read; the picture answers "what does this folder
 * look like" before a word has been read. Neither is a mode — a product that
 * made them tabs would make the reader choose between the question and the
 * answer.
 */
export function Rows({
  result,
  trail,
  showing,
  sort,
  basis,
  selected,
  onSortChange,
  onDescend,
  onDescendById,
  onClimb,
  onSelect,
  onSelectById,
}: RowsProps) {
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

      {/* Side by side above 60rem, stacked below it. The split is the shape of
          the product: the list and the picture are two readings of one folder,
          and a window narrow enough that they would crowd each other gets them
          one after the other rather than one instead of the other. */}
      <div className="grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)_minmax(0,1fr)] lg:grid-cols-[minmax(0,53fr)_minmax(0,47fr)] lg:grid-rows-1">
        <div className="flex min-h-0 min-w-0 flex-col border-b border-line lg:border-b-0 lg:border-r">
          {/* Keyed by the folder and the order: a change to either makes every
              row held meaningless, and a fresh component says that better than
              clearing five pieces of state by hand. */}
          <EntryTable
            key={`${showing.id}:${sort.key}:${sort.direction}`}
            parentId={showing.id}
            total={total}
            sort={sort}
            basis={basis}
            selectedId={selected?.id ?? null}
            onSortChange={onSortChange}
            onDescend={onDescend}
            onSelect={onSelect}
          />
        </div>

        <div className="flex min-h-0 min-w-0 flex-col">
          <Treemap
            // Same reasoning as the table: a new folder or a new basis makes
            // every rectangle held meaningless.
            key={`${showing.id}:${basis}`}
            parentId={showing.id}
            parentName={showing.name}
            basis={basis}
            selected={selected}
            onDescend={onDescendById}
            onSelect={onSelectById}
          />
        </div>
      </div>
    </div>
  )
}
