import { Segment, SegmentedControl } from '@/components/ui/segmented-control'
import { ComparisonReport } from '@/ComparisonReport'
import { EntryTable } from '@/EntryTable'
import { Treemap } from '@/Treemap'
import type { ChangesSummary, ComparisonSummary, Row, ScanResult, Size, SizeBasis, Sort, SortKey, View } from '@/core'
import { growthOf, pick, sizeOf } from '@/core'
import { describeChanges, describeComparison, describeOverhead, describeScan, explainSize, formatBytes, formatCount, formatSigned } from '@/format'

/** How a comparison is shown: as the folder's tree, or as one page. */
export type CompareMode = 'tree' | 'report'

interface RowsProps {
  /** Which tree is shown: the folder as it is, or what was written lately. */
  view: View
  /** Which reading of that tree the ids are from. */
  arena: number
  /** Moves whenever the tree changed. */
  version: number
  result: ScanResult
  /** What was written lately, when that is the view. */
  changes: ChangesSummary | null
  /** The span the view of what changed covers, in hours. */
  hours: number
  /** Two pictures compared, when that is the view. */
  comparison: ComparisonSummary | null
  compareMode: CompareMode
  onCompareMode: (mode: CompareMode) => void
  /** A place in the report was chosen: show it in the comparison's tree. */
  onOpenPlace: (id: number) => void
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
  view,
  arena,
  version,
  result,
  changes,
  hours,
  comparison,
  compareMode,
  onCompareMode,
  onOpenPlace,
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
  const compared = view === 'compare' && comparison !== null ? showing.compared : null
  // In a comparison the bars are shares of what moved in the folder, as the
  // tiles are; anywhere else, of what it holds.
  const total = compared === null ? sizeOf(showing, basis) : pick(compared.moved, basis)
  const overhead = compared === null ? describeOverhead(showing.logical, showing.allocated) : null
  // Why this entry in particular is unusual, when it is one — a folder carries
  // no traits, so in practice this speaks for a file the reader has descended
  // to rather than for a directory.
  const explanation = explainSize(showing.traits, showing.links)
  // `skipped` and the shared-bytes count are both for the whole scan, so they
  // are only true of the scan root. Repeating them inside every folder would
  // read as a claim about that folder, which nothing here knows.
  const atRoot = trail.length === 1
  const lately = view === 'changes' && changes !== null
  const elsewhere = lately || compared !== null
  const showsSkipped = !elsewhere && atRoot && result.skipped > 0
  const showsShared = !elsewhere && atRoot && result.sharedNames > 0
  // The scan-level sentence counts the names and the bytes; the entry-level one
  // only says that something inside is shared. Showing both at the root puts
  // the weaker claim beside the stronger one, saying the same thing twice.
  const showsExplanation = explanation !== null && !showsShared && compared === null

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* A comparison says what its numbers are first, and offers its other
          shape: the page that answers "how much came back, and from where". */}
      {compared !== null && comparison !== null && (
        <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-line px-4 py-2 text-xs text-dim">
          {compareMode === 'tree' ? (
            <>
              <span className="tabular text-sm">
                <span className={`font-medium ${(growthOf(showing, basis) ?? 0) > 0 ? 'text-bad' : (growthOf(showing, basis) ?? 0) < 0 ? 'text-good' : 'text-text'}`}>
                  {formatSigned(growthOf(showing, basis) ?? 0)}
                </span>{' '}
                · {formatBytes(pick(compared.before, basis))} → {formatBytes(sizeOf(showing, basis))}
              </span>
              {describeComparison(
                {
                  ...comparison,
                  // At the root, the comparison's own totals; below it, this
                  // folder's, which split into grown and freed exactly: what
                  // moved is the two added, the growth the two apart.
                  totals: atRoot
                    ? comparison.totals
                    : {
                        before: compared.before,
                        now: { logical: showing.logical, allocated: showing.allocated },
                        ...apart(compared.before, { logical: showing.logical, allocated: showing.allocated }, compared.moved),
                        new: 0,
                        gone: 0,
                        changed: 0,
                      },
                },
                basis,
              )
                .slice(0, atRoot ? undefined : 1)
                .map((part) => (
                  <span key={part}>{part}</span>
                ))}
            </>
          ) : (
            <span className="text-sm text-text">One page: what came back, and from where</span>
          )}
          <SegmentedControl
            aria-label="How to show the comparison"
            className="ml-auto"
            value={compareMode}
            onValueChange={(next) => {
              if (next === 'tree' || next === 'report') onCompareMode(next)
            }}
          >
            <Segment value="tree">Tree</Segment>
            <Segment value="report">Report</Segment>
          </SegmentedControl>
        </div>
      )}

      {compared !== null && comparison !== null && compareMode === 'report' ? (
        <ComparisonReport summary={comparison} basis={basis} root={trail[0]?.path ?? showing.path} onOpen={onOpenPlace} />
      ) : (
        <>
          {/* One strip: what this folder costs first, then how the numbers were
              read and anything about them that needs saying. It was two - a trail
              with the total, and the notes under it - until the trail moved up
              into the title bar, and a strip holding only a total is a strip of
              screen spent on one number. */}
          {compared === null && (
            <div className="flex shrink-0 flex-wrap items-baseline gap-x-4 gap-y-1 border-b border-line px-4 py-2 text-xs text-dim">
              <span className="tabular text-sm">
                <span className="font-medium text-text">{formatBytes(total)}</span> · {formatCount(showing.entries)} entries
              </span>
              {/* How the numbers were read, at the root: the same answer arrives in
                  minutes or in seconds, and a fallback from the fast way is said
                  rather than left to look like a slow disk. */}
              {/* In the view of what changed, the strip says what the numbers are:
                  what was written, what it holds now, and how far back the
                  journal reaches. Said at every depth - it is true of every
                  folder in this view, not only of its root. */}
              {lately && describeChanges(changes, hours).map((part) => <span key={part}>{part}</span>)}
              {!lately && atRoot && <span>{describeScan(result.scannedBy, result.elapsedMs)}</span>}
              {!lately && atRoot && result.fallback !== null && <span className="text-warn">{result.fallback}</span>}
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
                key={`${view}:${arena}:${showing.id}:${sort.key}:${sort.direction}`}
                view={view}
                arena={arena}
                version={version}
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
                key={`${view}:${arena}:${showing.id}:${basis}`}
                view={view}
                arena={arena}
                version={version}
                parentId={showing.id}
                parentName={showing.name}
                basis={basis}
                selected={selected}
                onDescend={onDescendById}
                onSelect={onSelectById}
              />
            </div>
          </div>
        </>
      )}
    </div>
  )
}

/** What a folder of a comparison held at two moments and moved, split into
 * the part that grew and the part that went: moved is the two added, the
 * growth the two apart. */
function apart(before: Size, now: Size, moved: Size): { grown: Size; freed: Size } {
  const half = (key: keyof Size) => {
    const growth = now[key] - before[key]
    return { grown: (moved[key] + growth) / 2, freed: (moved[key] - growth) / 2 }
  }
  const logical = half('logical')
  const allocated = half('allocated')
  return {
    grown: { logical: logical.grown, allocated: allocated.grown },
    freed: { logical: logical.freed, allocated: allocated.freed },
  }
}
