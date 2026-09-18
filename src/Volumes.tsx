import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import type { Volume } from '@/core'
import { formatBytes } from '@/format'

interface VolumesProps {
  /** `null` while the list is still being read. */
  volumes: Volume[] | null
  onPick: (path: string) => void
  onBrowse: () => void
}

/**
 * The first screen: where is the space, and which of it should be read.
 *
 * A drive is shown with how full it already is, because the question "which
 * drive" is usually answered by "the one that is nearly full" — and making the
 * user open a picker to find that out would be asking them for the answer.
 */
export function Volumes({ volumes, onPick, onBrowse }: VolumesProps) {
  if (volumes === null) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Spinner />
      </div>
    )
  }

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-1 flex-col justify-center gap-6 p-8">
      <div>
        <h1 className="text-lg font-semibold">What should midda read?</h1>
        <p className="mt-1 text-sm text-dim">
          Sizes are what the volume gives up, not what a file reads as. Without administrator rights the scan walks the
          filesystem, which is slower and works everywhere.
        </p>
      </div>

      <ul className="flex flex-col gap-2">
        {volumes.map((volume) => {
          const used = volume.total !== null && volume.free !== null ? volume.total - volume.free : null
          const share = used !== null && volume.total ? used / volume.total : null

          return (
            <li key={volume.path}>
              <button
                type="button"
                onClick={() => onPick(volume.path)}
                className="flex w-full cursor-pointer items-center gap-4 rounded-lg border border-line px-4 py-3 text-left transition-colors hover:border-line-2 hover:bg-soft"
              >
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-medium">{volume.label}</span>
                  {volume.total !== null && used !== null ? (
                    <span className="tabular mt-0.5 block text-xs text-dim">
                      {formatBytes(used)} used of {formatBytes(volume.total)}
                    </span>
                  ) : (
                    // A drive that is not ready — an empty card reader, a
                    // disconnected share — is offered without numbers rather
                    // than hidden: it is still somewhere a scan could point.
                    <span className="mt-0.5 block text-xs text-dim">not ready</span>
                  )}
                </span>

                {share !== null && (
                  <span className="h-1.5 w-32 shrink-0 overflow-hidden rounded-full bg-soft" aria-hidden>
                    <span
                      className={share > 0.9 ? 'block h-full bg-bad' : share > 0.75 ? 'block h-full bg-warn' : 'block h-full bg-accent'}
                      style={{ width: `${Math.round(share * 100)}%` }}
                    />
                  </span>
                )}
              </button>
            </li>
          )
        })}
      </ul>

      <div>
        <Button variant="ghost" size="sm" onClick={onBrowse}>
          Or choose a folder…
        </Button>
      </div>
    </div>
  )
}
