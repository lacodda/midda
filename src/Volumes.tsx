import { Button } from '@/components/ui/button'
import { RowButton } from '@/components/ui/list-row'
import { Spinner } from '@/components/ui/spinner'
import type { Volume } from '@/core'
import { formatBytes, formatShare } from '@/format'

interface VolumesProps {
  /** `null` while the list is still being read. */
  volumes: Volume[] | null
  onPick: (path: string) => void
  onBrowse: () => void
}

/** What is used on a volume, when it could be read. */
function usedOf(volume: Volume): { used: number; share: number } | null {
  if (volume.total === null || volume.free === null || volume.total <= 0) return null
  const used = volume.total - volume.free
  return { used, share: used / volume.total }
}

/**
 * The first screen: where is the space, across every volume at once.
 *
 * The question "which drive should I look at" is nearly always answered by "the
 * one that is nearly full", and a file picker cannot say which that is. So the
 * drives are shown with how full each already is, sorted by what is left rather
 * than by letter — the one about to run out belongs at the top, whatever it is
 * called.
 *
 * This is not the scan. Nothing here has been read: these are the numbers the
 * volume itself reports, which arrive instantly and are the reason the reader
 * can choose before waiting for anything.
 */
export function Volumes({ volumes, onPick, onBrowse }: VolumesProps) {
  if (volumes === null) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Spinner label="Reading the volumes" />
      </div>
    )
  }

  // Fullest first. A volume that would not report its space has no claim on the
  // top of the list, so it sorts last — the same rule the table uses for a row
  // with no date: unknown is not the same as zero.
  const ordered = [...volumes].sort((a, b) => {
    const [left, right] = [usedOf(a), usedOf(b)]
    if (left === null && right === null) return a.path.localeCompare(b.path)
    if (left === null) return 1
    if (right === null) return -1
    return right.share - left.share
  })

  const known = ordered.map(usedOf).filter((space): space is { used: number; share: number } => space !== null)
  const totalUsed = known.reduce((sum, space) => sum + space.used, 0)
  const totalCapacity = ordered.reduce((sum, volume) => sum + (volume.total ?? 0), 0)

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-1 flex-col justify-center gap-6 p-8">
      <div>
        <h1 className="text-lg font-semibold">Where is the space?</h1>
        <p className="mt-1 text-sm text-dim">
          Every volume on this machine, fullest first. Sizes are what the volume gives up, not what a file reads as. Without
          administrator rights the scan walks the filesystem, which is slower and works everywhere.
        </p>
      </div>

      <ul className="flex flex-col gap-2">
        {ordered.map((volume) => {
          const space = usedOf(volume)

          return (
            <li key={volume.path}>
              {/* A choice in a list, so dowel's RowButton, in the card the
                  first screen has always drawn: the border and the room are
                  what make five drives read as five things to pick from. */}
              <RowButton
                onClick={() => onPick(volume.path)}
                className="gap-4 rounded-lg border border-line px-4 py-3 hover:border-line-2"
                description={
                  space !== null && volume.total !== null && volume.free !== null ? (
                    <span className="tabular">
                      {formatBytes(space.used)} used of {formatBytes(volume.total)} · {formatBytes(volume.free)} free
                    </span>
                  ) : (
                    // A drive that is not ready — an empty card reader, a
                    // disconnected share — is offered without numbers rather
                    // than hidden: it is still somewhere a scan could point.
                    'not ready'
                  )
                }
                end={
                  space !== null && (
                    <>
                      <span className="tabular w-12 text-right">{formatShare(space.used, volume.total ?? 0)}</span>
                      <span className="h-1.5 w-32 overflow-hidden rounded-full bg-soft" aria-hidden>
                        <span
                          className={
                            space.share > 0.9 ? 'block h-full bg-bad' : space.share > 0.75 ? 'block h-full bg-warn' : 'block h-full bg-accent'
                          }
                          style={{ width: `${Math.round(space.share * 100)}%` }}
                        />
                      </span>
                    </>
                  )
                }
              >
                {volume.label}
              </RowButton>
            </li>
          )
        })}
      </ul>

      <div className="flex items-baseline justify-between gap-4">
        <Button variant="ghost" size="sm" onClick={onBrowse}>
          Or choose a folder…
        </Button>
        {known.length > 1 && totalCapacity > 0 && (
          <span className="tabular text-xs text-dim">
            {formatBytes(totalUsed)} used across {known.length} volumes
          </span>
        )}
      </div>
    </div>
  )
}
