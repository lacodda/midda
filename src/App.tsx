import { useCallback, useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'

import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/ui/empty-state'
import { Progress } from '@/components/ui/progress'
import {
  cancelScan,
  DEFAULT_SORT,
  listVolumes,
  scanProgress,
  startScan,
  toggleSort,
  type Row,
  type ScanResult,
  type SizeBasis,
  type Sort,
  type SortKey,
  type Volume,
} from '@/core'
import { formatBytes, formatCount } from '@/format'
import { Rows } from '@/Rows'
import { Volumes } from '@/Volumes'

/**
 * How often the window asks the core how far the scan has got.
 *
 * Six times a second: fast enough that the counter looks live, slow enough that
 * a scan reading a million directories is not also answering a million
 * questions about itself.
 */
const POLL_MS = 160

/** Where the user is: choosing, scanning, or reading a result. */
type Stage =
  | { at: 'choosing' }
  | { at: 'scanning'; target: string }
  | { at: 'done'; target: string; result: ScanResult }
  | { at: 'failed'; target: string; message: string }

export default function App() {
  const [volumes, setVolumes] = useState<Volume[] | null>(null)
  const [stage, setStage] = useState<Stage>({ at: 'choosing' })
  const [counted, setCounted] = useState({ entries: 0, bytes: 0 })
  const [basis, setBasis] = useState<SizeBasis>('allocated')
  const [sort, setSort] = useState<Sort>(DEFAULT_SORT)
  /** The path from the scan root down to what is on screen. */
  const [trail, setTrail] = useState<Row[]>([])

  useEffect(() => {
    void listVolumes().then(setVolumes)
  }, [])

  const polling = useRef<number | null>(null)

  const stopPolling = useCallback(() => {
    if (polling.current !== null) {
      window.clearInterval(polling.current)
      polling.current = null
    }
  }, [])

  // The poll outlives any one render, so it is cleared on unmount as well as on
  // finishing — a window closed mid-scan should not leave a timer asking a core
  // that is going away.
  useEffect(() => stopPolling, [stopPolling])

  const begin = useCallback(
    async (target: string) => {
      setStage({ at: 'scanning', target })
      setCounted({ entries: 0, bytes: 0 })
      setSort(DEFAULT_SORT)
      setTrail([])

      try {
        await startScan(target)
      } catch (cause) {
        setStage({ at: 'failed', target, message: String(cause) })
        return
      }

      stopPolling()
      polling.current = window.setInterval(() => {
        void scanProgress().then((state) => {
          setCounted({ entries: state.entries, bytes: state.bytes })
          if (state.running) return

          stopPolling()
          if (state.error !== null) {
            setStage({ at: 'failed', target, message: state.error })
          } else if (state.result !== null) {
            setStage({ at: 'done', target, result: state.result })
            setTrail([state.result.root])
          }
        })
      }, POLL_MS)
    },
    [stopPolling],
  )

  const chooseFolder = useCallback(async () => {
    const picked = await open({ directory: true })
    if (typeof picked === 'string') void begin(picked)
  }, [begin])

  // Whatever is at the end of the trail is what the table shows.
  const showing = trail.at(-1) ?? null

  const changeSort = useCallback((key: SortKey) => {
    setSort((sort) => toggleSort(sort, key))
  }, [])

  /* The switch moves the sort with it when the sort is on the other size.
   * Showing "on disk" while ordering by what files read as is two answers to
   * one question, and the reader cannot see which they are looking at. A sort
   * on name, date or item count is left alone: it is not about size at all. */
  const changeBasis = useCallback((next: SizeBasis) => {
    setBasis(next)
    setSort((sort) => (sort.key === 'allocated' || sort.key === 'logical' ? { key: next, direction: sort.direction } : sort))
  }, [])

  const descend = useCallback((row: Row) => {
    if (row.isDirectory) setTrail((trail) => [...trail, row])
  }, [])

  const climbTo = useCallback((depth: number) => {
    setTrail((trail) => trail.slice(0, depth + 1))
  }, [])

  return (
    <div className="flex h-full flex-col bg-bg text-text">
      <header className="flex shrink-0 items-center justify-between gap-4 border-b border-line px-4 py-2.5">
        <div className="flex items-center gap-2">
          {/* 20 px is the S band, so the mark is the filled tile: the outline
              and the three tonal steps do not survive at this size. The rule is
              held by src-tauri/tests/brand_assets.rs. */}
          <img src="/mark.svg" alt="" width={20} height={20} />
          <span className="font-semibold">midda</span>
          <span className="text-xs text-dim">{__APP_VERSION__}</span>
        </div>

        <div className="flex items-center gap-2">
          {stage.at === 'done' && (
            <div className="mr-1 flex items-center gap-1 rounded-md border border-line p-0.5 text-xs">
              {(
                [
                  ['allocated', 'On disk'],
                  ['logical', 'Size'],
                ] as const
              ).map(([value, label]) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => changeBasis(value)}
                  aria-pressed={basis === value}
                  className={
                    basis === value
                      ? 'cursor-pointer rounded bg-accent-soft px-2 py-1 font-medium text-accent'
                      : 'cursor-pointer rounded px-2 py-1 text-dim hover:text-text'
                  }
                >
                  {label}
                </button>
              ))}
            </div>
          )}

          {stage.at === 'scanning' ? (
            <Button variant="ghost" size="sm" onClick={() => void cancelScan()}>
              Stop
            </Button>
          ) : (
            <Button variant="ghost" size="sm" onClick={() => void chooseFolder()}>
              Choose a folder…
            </Button>
          )}
        </div>
      </header>

      <main className="flex min-h-0 flex-1 flex-col">
        {stage.at === 'choosing' && <Volumes volumes={volumes} onPick={(path) => void begin(path)} onBrowse={() => void chooseFolder()} />}

        {stage.at === 'scanning' && (
          <div className="flex flex-1 flex-col items-center justify-center gap-4 p-8">
            <p className="max-w-xl truncate text-sm text-dim" title={stage.target}>
              Reading {stage.target}
            </p>
            {/* Indeterminate on purpose: nothing knows how many entries a
                volume holds until the walk has read them all, and a bar that
                creeps to 90% and waits is a lie the reader learns to
                distrust. */}
            <Progress className="max-w-md" label="Reading the disk" />
            <p className="tabular text-sm">
              <span className="font-medium">{formatCount(counted.entries)}</span> <span className="text-dim">entries ·</span>{' '}
              <span className="font-medium">{formatBytes(counted.bytes)}</span> <span className="text-dim">on disk</span>
            </p>
          </div>
        )}

        {stage.at === 'failed' && (
          <div className="flex flex-1 items-center justify-center p-8">
            <Alert tone="bad" title="The scan did not finish">
              {stage.message}
            </Alert>
          </div>
        )}

        {stage.at === 'done' && showing !== null && (
          <Rows
            result={stage.result}
            trail={trail}
            showing={showing}
            sort={sort}
            basis={basis}
            onSortChange={changeSort}
            onDescend={descend}
            onClimb={climbTo}
          />
        )}

        {stage.at === 'done' && showing === null && <EmptyState title="Nothing to show" body="The scan came back empty." />}
      </main>
    </div>
  )
}
