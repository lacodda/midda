import { EntryTable } from '@/EntryTable'
import { Treemap } from '@/Treemap'
import type { Row, ScanResult, SizeBasis, Sort, SortKey } from '@/core'
import { sizeOf } from '@/core'
import { describeOverhead, describeScan, explainSize, formatBytes, formatCount } from '@/format'

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
  onSelect: (row: Row | null) => void
  onSelectById: (id: number | null) => void
}

/**
 * What the scan found: what it costs, how it was read, and the two views of
 * what is inside. Where you are is the title bar's to say.
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
  onSelect,
  onSelectById,
}: RowsProps) {
  const total = sizeOf(showing, basis)
  const overhead = describeOverhead(showing.logical, showing.allocated)
  // Why this entry in particular is unusual, when it is one — a folder carries
  // no traits, so in practice this speaks for a file the reader has descended
  // to rather than for a directory.
  const explanation = explainSize(showing.traits, showing.links)
  // `skipped` and the shared-bytes count are both for the whole scan, so they
  // are only true of the scan root. Repeating them inside every folder would
  // read as a claim about that folder, which nothing here knows.
  const atRoot = trail.length === 1
  const showsSkipped = atRoot && result.skipped > 0
  const showsShared = atRoot && result.sharedNames > 0
  // The scan-level sentence counts the names and the bytes; the entry-level one
  // only says that something inside is shared. Showing both at the root puts
  // the weaker claim beside the stronger one, saying the same thing twice.
  const showsExplanation = explanation !== null && !showsShared

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* One strip: what this folder costs first, then how the numbers were
          read and anything about them that needs saying. It was two - a trail
          with the total, and the notes under it - until the trail moved up
          into the title bar, and a strip holding only a total is a strip of
          screen spent on one number. */}
      <div className="flex shrink-0 flex-wrap items-baseline gap-x-4 gap-y-1 border-b border-line px-4 py-2 text-xs text-dim">
        <span className="tabular text-sm">
          <span className="font-medium text-text">{formatBytes(total)}</span> · {formatCount(showing.entries)} entries
        </span>
        {/* How the numbers were read, at the root: the same answer arrives in
            minutes or in seconds, and a fallback from the fast way is said
            rather than left to look like a slow disk. */}
        {atRoot && <span>{describeScan(result.scannedBy, result.elapsedMs)}</span>}
        {atRoot && result.fallback !== null && <span className="text-warn">{result.fallback}</span>}
        {overhead !== null && <span>{overhead}</span>}
        {showsExplanation && <span>{explanation}</span>}
        {result.clusterBytes !== null && overhead !== null && <span>cluster {formatBytes(result.clusterBytes)}</span>}
        {/* The difference between midda's total and a naive one, stated rather
            than silently applied. A volume holding a package manager's store
            can double under a scanner that counts every name of a hard-linked
            file, and a reader comparing two tools should be able to see which
            of them is explaining itself. */}
        {showsShared && (
          <span>
            {formatCount(result.sharedNames)} {result.sharedNames === 1 ? 'entry is another name' : 'entries are other names'} for bytes counted
            elsewhere — {formatBytes(result.sharedReclaimed)} not counted twice
          </span>
        )}
        {/* A scan of a system volume always refuses something. Saying so is
            the difference between a total that is short and a total that is
            quietly wrong. Only at the scan root: the count is for the whole
            scan, and repeating it inside every folder would read as a claim
            about that folder — which nothing here knows. */}
        {showsSkipped && <span>{formatCount(result.skipped)} entries could not be read in this scan</span>}
      </div>

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
