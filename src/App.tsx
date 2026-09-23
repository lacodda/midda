import { useCallback, useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'

import { Alert } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/ui/empty-state'
import { Progress } from '@/components/ui/progress'
import {
  accelerate,
  acceleration,
  cancelScan,
  DEFAULT_SORT,
  launchRequest,
  listVolumes,
  scanProgress,
  startScan,
  toggleSort,
  trailTo,
  type Acceleration,
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
  const [counted, setCounted] = useState({ entries: 0, bytes: 0, records: 0 })
  /** Where the folder being looked at stands with the fast scanner. */
  const [fast, setFast] = useState<Acceleration | null>(null)
  /** Why "accelerate" did not happen, when it did not. */
  const [notice, setNotice] = useState<string | null>(null)
  const [basis, setBasis] = useState<SizeBasis>('allocated')
  const [sort, setSort] = useState<Sort>(DEFAULT_SORT)
  /** The path from the scan root down to what is on screen. */
  const [trail, setTrail] = useState<Row[]>([])
  /** The one entry both the table and the picture are pointing at. */
  const [selected, setSelected] = useState<Row | null>(null)

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
      setCounted({ entries: 0, bytes: 0, records: 0 })
      setNotice(null)
      setSort(DEFAULT_SORT)
      setTrail([])
      setSelected(null)

      try {
        await startScan(target)
      } catch (cause) {
        setStage({ at: 'failed', target, message: String(cause) })
        return
      }

      stopPolling()
      polling.current = window.setInterval(() => {
        void scanProgress().then((state) => {
          setCounted({ entries: state.entries, bytes: state.bytes, records: state.records })
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

  /* A window started by "accelerate" was given the folder to scan on its
   * command line: it goes straight to it rather than asking a second time. */
  useEffect(() => {
    void launchRequest().then((path) => {
      if (path !== null) void begin(path)
    })
  }, [begin])

  const target = stage.at === 'choosing' ? null : stage.target

  /* Asked per folder, not once: the fast scanner needs NTFS under the path as
   * well as elevation, and a USB stick on exFAT is not made faster by a
   * prompt. */
  useEffect(() => {
    void acceleration(target).then(setFast)
  }, [target])

  const speedUp = useCallback(() => {
    setNotice(null)
    // On success this window closes and an elevated one takes over; the
    // promise only settles here when that did not happen.
    accelerate(target).catch((cause: unknown) => setNotice(String(cause)))
  }, [target])

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
    if (!row.isDirectory) return
    setTrail((trail) => [...trail, row])
    setSelected(null)
  }, [])

  /* A click in the picture arrives as an id, not a row: the treemap draws
   * rectangles the table may never have fetched. The trail is asked of the
   * core rather than appended to, because the window's own copy is only right
   * as long as the reader arrived by clicking down one level at a time. */
  const descendTo = useCallback((id: number) => {
    void trailTo(id).then((steps) => {
      setTrail(steps)
      setSelected(null)
    })
  }, [])

  const selectById = useCallback((id: number | null) => {
    if (id === null) {
      setSelected(null)
      return
    }
    // The same ask, for the same reason — and the last step of the trail is
    // the row itself, with both its sizes and its path.
    void trailTo(id).then((steps) => setSelected(steps.at(-1) ?? null))
  }, [])

  const climbTo = useCallback((depth: number) => {
    setTrail((trail) => trail.slice(0, depth + 1))
    setSelected(null)
  }, [])

  /* Backspace goes up a level, from anywhere that is not a text field. It is
   * what Explorer does, and the mockup says so on the breadcrumb. */
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Backspace') return
      const target = event.target as HTMLElement | null
      if (target?.tagName === 'INPUT' || target?.tagName === 'TEXTAREA' || target?.isContentEditable) return
      event.preventDefault()
      setTrail((trail) => (trail.length > 1 ? trail.slice(0, -1) : trail))
      setSelected(null)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
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
          {/* Offered only where it would help: NTFS under the folder, and this
              window not already elevated. The prompt is the reader's to ask
              for, never one sprung on them (ADR 0001). */}
          {fast === 'needs-elevation' && stage.at !== 'scanning' && (
            <Button
              variant="soft"
              size="sm"
              onClick={speedUp}
              title="Restart midda as an administrator to read the volume's Master File Table: seconds instead of minutes. Windows will ask first."
            >
              Accelerate
            </Button>
          )}

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

      {notice !== null && (
        <div className="shrink-0 border-b border-line px-4 py-2">
          <Alert tone="warn">{notice}</Alert>
        </div>
      )}

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
            {/* The MFT reader reads the whole table before it places a single
                entry, so for those seconds the moving number is the records
                read — shown as that, not passed off as entries. */}
            {counted.records > 0 && counted.entries === 0 ? (
              <p className="tabular text-sm">
                <span className="text-dim">Reading the MFT ·</span> <span className="font-medium">{formatCount(counted.records)}</span>{' '}
                <span className="text-dim">records</span>
              </p>
            ) : (
              <p className="tabular text-sm">
                <span className="font-medium">{formatCount(counted.entries)}</span> <span className="text-dim">entries ·</span>{' '}
                <span className="font-medium">{formatBytes(counted.bytes)}</span> <span className="text-dim">on disk</span>
              </p>
            )}
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
            selected={selected}
            onSortChange={changeSort}
            onDescend={descend}
            onDescendById={descendTo}
            onClimb={climbTo}
            onSelect={setSelected}
            onSelectById={selectById}
          />
        )}

        {stage.at === 'done' && showing === null && <EmptyState title="Nothing to show" body="The scan came back empty." />}
      </main>
    </div>
  )
}
