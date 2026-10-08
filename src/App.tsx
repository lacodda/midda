import { useCallback, useEffect, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'

import { Alert } from '@/components/ui/alert'
import { AppShell, Screen } from '@/components/ui/app-shell'
import { Breadcrumbs } from '@/components/ui/breadcrumbs'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/ui/empty-state'
import { ProductMark } from '@/components/ui/product-mark'
import { Progress } from '@/components/ui/progress'
import { Segment, SegmentedControl } from '@/components/ui/segmented-control'
import { StatusDot } from '@/components/ui/status-dot'
import { ResizeEdges, TitleBar } from '@/components/ui/window-frame'
import {
  accelerate,
  acceleration,
  cancelScan,
  changesSince,
  compareSnapshots,
  DEFAULT_SORT,
  GROWTH_SORT,
  isStale,
  launchRequest,
  listSnapshots,
  listVolumes,
  rescan,
  scanProgress,
  startScan,
  toggleSort,
  trailTo,
  trailToPath,
  type Acceleration,
  type ChangesSummary,
  type ComparisonSummary,
  type Row,
  type SessionState,
  type SizeBasis,
  type Sort,
  type SortKey,
  type View,
  type Volume,
} from '@/core'
import { CHANGE_SPANS, describeFreshness, formatBytes, formatCount } from '@/format'
import { Rows, type CompareMode } from '@/Rows'
import { SnapshotMenu } from '@/SnapshotMenu'
import { Splash } from '@/Splash'
import { Volumes } from '@/Volumes'

/**
 * How often the window asks the core how things stand: six times a second
 * while a read runs, so the counter looks live, and once a second otherwise,
 * which is as often as the keeper applies what changed.
 */
const POLL_READING_MS = 160
const POLL_IDLE_MS = 1000

/** What the window's own buttons are called, where a system frame would have
 * named them itself. */
const WINDOW_LABELS = { minimize: 'Minimize', maximize: 'Maximize', restore: 'Restore', close: 'Close' }

/**
 * The mark and the name, where a system title bar would print them.
 *
 * 18 px is the S band, so the mark is the filled tile: the outline and the
 * tonal steps of the larger levels do not survive at this size. ProductMark
 * picks the level from the size; the masters are the line's.
 */
function Brand() {
  return (
    <span className="flex items-center gap-2">
      <ProductMark product="midda" size={18} />
      <span className="font-semibold">midda</span>
      <span className="font-mono text-2xs text-faint">{__APP_VERSION__}</span>
    </span>
  )
}

/** Where the user is: choosing, opening a folder, or looking at one. */
type Stage = { at: 'choosing' } | { at: 'opening'; target: string } | { at: 'open'; target: string } | { at: 'failed'; target: string; message: string }

/** What the span control says: everything, what grew since an earlier
 * picture, or what was written lately. */
type Span = 'all' | 'grown' | 'day' | 'week'

/** The order each view opens in. A comparison's is its own question. */
const SORTS: Record<View, Sort> = { index: DEFAULT_SORT, changes: DEFAULT_SORT, compare: GROWTH_SORT }

/** One trail and the ids it is in, per view. */
interface Place {
  trail: Row[]
  arena: number
}

const NOWHERE: Place = { trail: [], arena: 0 }

export default function App() {
  const [volumes, setVolumes] = useState<Volume[] | null>(null)
  const [stage, setStage] = useState<Stage>({ at: 'choosing' })
  const [session, setSession] = useState<SessionState | null>(null)
  /** Where the folder being looked at stands with the fast scanner. */
  const [fast, setFast] = useState<Acceleration | null>(null)
  /** Why "accelerate" did not happen, when it did not. */
  const [notice, setNotice] = useState<string | null>(null)
  const [basis, setBasis] = useState<SizeBasis>('allocated')
  /** The order of each view, kept apart: a comparison sorted by growth and
   * the folder sorted by size are two questions, not one order. */
  const [sorts, setSorts] = useState<Record<View, Sort>>(SORTS)
  /** Which tree is on screen. */
  const [view, setView] = useState<View>('index')
  /** The span the view of what changed covers. */
  const [hours, setHours] = useState<number>(CHANGE_SPANS.day)
  const [changes, setChanges] = useState<ChangesSummary | null>(null)
  /** The journal is being read for the view of what changed. */
  const [gathering, setGathering] = useState(false)
  /** Two pictures of the folder compared, once asked. */
  const [comparison, setComparison] = useState<ComparisonSummary | null>(null)
  const [compareMode, setCompareMode] = useState<CompareMode>('tree')
  /** Where the reader is in each view. */
  const [places, setPlaces] = useState<Record<View, Place>>({ index: NOWHERE, changes: NOWHERE, compare: NOWHERE })
  /** The version of the folder's tree the screen was last drawn from. */
  const [version, setVersion] = useState(0)
  /** The one entry both the table and the picture are pointing at. */
  const [selected, setSelected] = useState<Row | null>(null)

  /* A list that could not be read still ends the splash: the window opens on
   * an empty list with the reason above it, and a folder can still be chosen.
   * Left unhandled, the rejection kept the splash up for good. */
  useEffect(() => {
    listVolumes()
      .then(setVolumes)
      .catch((cause: unknown) => {
        setVolumes([])
        setNotice(`The volumes could not be listed: ${String(cause)}`)
      })
  }, [])

  // The poll reads these without being rebuilt every time they change: it is
  // one loop for the life of an open folder, not one per render.
  const placesRef = useRef(places)
  const versionRef = useRef(version)
  const selectedRef = useRef(selected)
  const viewRef = useRef(view)
  useEffect(() => {
    placesRef.current = places
    versionRef.current = version
    selectedRef.current = selected
    viewRef.current = view
  }, [places, version, selected, view])

  const polling = useRef<number | null>(null)

  const stopPolling = useCallback(() => {
    if (polling.current !== null) {
      window.clearTimeout(polling.current)
      polling.current = null
    }
  }, [])

  // The poll outlives any one render, so it is cleared on unmount as well as on
  // closing — a window closed mid-read should not leave a timer asking a core
  // that is going away.
  useEffect(() => stopPolling, [stopPolling])

  /**
   * Finds the reader's place again after the tree changed under it: the
   * folder on screen by its path — or the deepest folder above it still
   * there — and the selection the same way.
   */
  const refind = useCallback(async (state: SessionState) => {
    const here = placesRef.current.index.trail.at(-1)?.path
    if (here === undefined) return
    try {
      const found = await trailToPath('index', here)
      setPlaces((places) => ({ ...places, index: { trail: found.rows, arena: found.arena } }))
      const chosen = selectedRef.current
      if (chosen !== null && viewRef.current === 'index') {
        const again = await trailToPath('index', chosen.path)
        const last = again.rows.at(-1)
        setSelected(last !== undefined && last.path === chosen.path ? last : null)
      }
      setVersion(state.version)
    } catch {
      // The next poll asks again.
    }
  }, [])

  const begin = useCallback(
    async (target: string) => {
      stopPolling()
      setStage({ at: 'opening', target })
      setSession(null)
      setNotice(null)
      setSorts(SORTS)
      setView('index')
      setChanges(null)
      setComparison(null)
      setPlaces({ index: NOWHERE, changes: NOWHERE, compare: NOWHERE })
      setSelected(null)

      try {
        await startScan(target)
      } catch (cause) {
        setStage({ at: 'failed', target, message: String(cause) })
        return
      }

      const poll = () => {
        void scanProgress()
          .then(async (state) => {
            setSession(state)
            if (state.error !== null && state.result === null) {
              setStage({ at: 'failed', target, message: state.error })
              return
            }
            if (state.result !== null) {
              const held = placesRef.current.index
              if (held.trail.length === 0) {
                // The first picture: the root, in the ids it came with.
                setPlaces((places) => ({ ...places, index: { trail: [state.result!.root], arena: state.arena } }))
                setVersion(state.version)
                setStage({ at: 'open', target })
              } else if (state.arena !== held.arena || state.version !== versionRef.current) {
                await refind(state)
              }
            }
            const busy = state.reading || state.freshness.catchingUp
            polling.current = window.setTimeout(poll, busy ? POLL_READING_MS : POLL_IDLE_MS)
          })
          .catch(() => {
            polling.current = window.setTimeout(poll, POLL_IDLE_MS)
          })
      }
      poll()
    },
    [stopPolling, refind],
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

  const place = places[view]
  // Whatever is at the end of the trail is what the table shows.
  const showing = place.trail.at(-1) ?? null

  const setTrail = useCallback(
    (change: (trail: Row[]) => Row[]) => {
      setPlaces((places) => ({ ...places, [view]: { ...places[view], trail: change(places[view].trail) } }))
    },
    [view],
  )

  const sort = sorts[view]

  const changeSort = useCallback(
    (key: SortKey) => {
      setSorts((sorts) => ({ ...sorts, [view]: toggleSort(sorts[view], key) }))
    },
    [view],
  )

  /* The switch moves the sort with it when the sort is on the other size.
   * Showing "on disk" while ordering by what files read as is two answers to
   * one question, and the reader cannot see which they are looking at. A sort
   * on name, date or item count is left alone: it is not about size at all. */
  const changeBasis = useCallback((next: SizeBasis) => {
    setBasis(next)
    const moved = (sort: Sort): Sort => (sort.key === 'allocated' || sort.key === 'logical' ? { key: next, direction: sort.direction } : sort)
    setSorts((sorts) => ({ index: moved(sorts.index), changes: moved(sorts.changes), compare: moved(sorts.compare) }))
  }, [])

  /** Puts a comparison on screen, at its root, as its tree. */
  const showComparison = useCallback((summary: ComparisonSummary) => {
    setComparison(summary)
    setPlaces((places) => ({ ...places, compare: { trail: [summary.root], arena: summary.arena } }))
    setCompareMode('tree')
    setSelected(null)
    setView('compare')
  }, [])

  /* "Grown": the picture of last time against now, in one press - or, when
   * there is none, the newest snapshot taken. Asked again on every press, so
   * the answer is as of the click. */
  const compareWithEarlier = useCallback(async () => {
    setSelected(null)
    setGathering(true)
    try {
      const listed = await listSnapshots()
      const usable = listed.snapshots.filter((snapshot) => snapshot.unusable === null)
      const earlier = usable.find((snapshot) => snapshot.kind === 'last') ?? usable[0]
      if (earlier === undefined) {
        setNotice(
          'There is no earlier picture of this folder to compare with yet. midda keeps one each time the folder is opened; Snapshots takes one now, to compare with later.',
        )
        return
      }
      showComparison(await compareSnapshots(earlier.name, null))
    } catch (cause) {
      setNotice(`The comparison could not be made: ${String(cause)}`)
    } finally {
      setGathering(false)
    }
  }, [showComparison])

  /* A place chosen in the report: its folder in the comparison's tree, with
   * a file selected rather than entered - a file has nothing inside. */
  const openPlace = useCallback(
    (id: number) => {
      void trailTo('compare', places.compare.arena, id)
        .then((steps) => {
          const last = steps.at(-1)
          const folder = last !== undefined && !last.isDirectory ? steps.slice(0, -1) : steps
          setPlaces((places) => ({ ...places, compare: { ...places.compare, trail: folder } }))
          setSelected(last !== undefined && !last.isDirectory ? last : null)
          setCompareMode('tree')
        })
        .catch((cause: unknown) => {
          if (!isStale(cause)) setNotice(String(cause))
        })
    },
    [places.compare.arena],
  )

  /* The span control. "All" is the folder as it is; the others ask the
   * journal what was written lately and show that as a tree of its own, with
   * the same table and picture. Asked again on every choice, so the answer
   * is as of the click. */
  const changeSpan = useCallback(async (next: Span) => {
    setSelected(null)
    if (next === 'all') {
      setView('index')
      return
    }
    if (next === 'grown') {
      await compareWithEarlier()
      return
    }
    const span = next === 'day' ? CHANGE_SPANS.day : CHANGE_SPANS.week
    setHours(span)
    setView('changes')
    setGathering(true)
    try {
      const summary = await changesSince(span)
      setChanges(summary)
      setPlaces((places) => ({ ...places, changes: { trail: [summary.root], arena: summary.arena } }))
    } catch (cause) {
      setNotice(`What changed could not be read: ${String(cause)}`)
      setView('index')
    } finally {
      setGathering(false)
    }
  }, [compareWithEarlier])

  const descend = useCallback(
    (row: Row) => {
      if (!row.isDirectory) return
      setTrail((trail) => [...trail, row])
      setSelected(null)
    },
    [setTrail],
  )

  /* A click in the picture arrives as an id, not a row: the treemap draws
   * rectangles the table may never have fetched. The trail is asked of the
   * core rather than appended to, because the window's own copy is only right
   * as long as the reader arrived by clicking down one level at a time. */
  const descendTo = useCallback(
    (id: number) => {
      void trailTo(view, place.arena, id)
        .then((steps) => {
          setTrail(() => steps)
          setSelected(null)
        })
        .catch((cause: unknown) => {
          if (!isStale(cause)) setNotice(String(cause))
        })
    },
    [view, place.arena, setTrail],
  )

  const selectById = useCallback(
    (id: number | null) => {
      if (id === null) {
        setSelected(null)
        return
      }
      // The same ask, for the same reason — and the last step of the trail is
      // the row itself, with both its sizes and its path.
      void trailTo(view, place.arena, id)
        .then((steps) => setSelected(steps.at(-1) ?? null))
        .catch(() => setSelected(null))
    },
    [view, place.arena],
  )

  const climbTo = useCallback(
    (depth: number) => {
      setTrail((trail) => trail.slice(0, depth + 1))
      setSelected(null)
    },
    [setTrail],
  )

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
  }, [setTrail])

  /* Opening: the volumes have not answered and nothing has been started. A
   * window started by "accelerate" skips it the moment its scan begins. */
  if (volumes === null && stage.at === 'choosing') return <Splash />

  const result = session?.result ?? null
  const reading = session?.reading ?? false
  const freshness =
    session === null || stage.at !== 'open'
      ? null
      : describeFreshness({ ...session.freshness, reading, readAt: result?.readAt ?? null })
  const journalled = session?.freshness.mode === 'journal'
  // The picture on screen is last time's, not yet brought up to date:
  // comparing it with last time would find nothing, and say so wrongly.
  const settling = session !== null && (session.freshness.savedAt !== null || session.freshness.catchingUp)
  const span: Span = view === 'index' ? 'all' : view === 'compare' ? 'grown' : hours === CHANGE_SPANS.day ? 'day' : 'week'
  const counted = { entries: session?.entries ?? 0, bytes: session?.bytes ?? 0, records: session?.records ?? 0 }

  return (
    <>
      {/* The frame is dowel's: one bar where the system's title bar and the
          application's toolbar used to be two, and the screen under it. The
          bar holds the mark, where you are, and the two or three things the
          window can do; everything in it that is not a control is a handle to
          drag the window by. */}
      <AppShell
        top={
          <TitleBar
            labels={WINDOW_LABELS}
            mark={<Brand />}
            actions={
              <div className="flex items-center gap-2 pr-1">
                {/* How current the picture is, and what keeps it so. The
                    whole story is the tooltip: the same picture can be a
                    second old or a day old and look identical. */}
                {freshness !== null && (
                  <span className="flex items-center gap-1.5 text-xs text-dim" title={freshness.detail}>
                    <StatusDot status={freshness.tone} label={freshness.label} />
                    <span className="tabular whitespace-nowrap">{freshness.label}</span>
                    {reading && result !== null && <span className="tabular whitespace-nowrap text-faint">· {formatCount(counted.entries)}</span>}
                  </span>
                )}

                {stage.at === 'open' && session?.freshness.mode === 'still' && !reading && (
                  <Button variant="ghost" size="sm" onClick={() => void rescan()} title="Read the folder again in full, keeping this picture until the new one is ready.">
                    Read again
                  </Button>
                )}

                {/* Offered only where it would help: NTFS under the folder,
                    and this window not already elevated. The prompt is the
                    reader's to ask for, never one sprung on them (ADR 0001). */}
                {fast === 'needs-elevation' && stage.at !== 'opening' && (
                  <Button
                    variant="soft"
                    size="sm"
                    onClick={speedUp}
                    title="Restart midda as an administrator to read the volume's Master File Table and follow its change journal: seconds instead of minutes, and fresh across restarts. Windows will ask first."
                  >
                    Accelerate
                  </Button>
                )}

                {/* What to show: everything; what grew since an earlier
                    picture, which needs no elevation; or what was written
                    lately, which is the journal's to say - without it those
                    two are there and say why they cannot be pressed rather
                    than hiding. */}
                {stage.at === 'open' && (
                  <SegmentedControl
                    aria-label="What to show"
                    value={span}
                    onValueChange={(next) => {
                      if (next === 'all' || next === 'grown' || next === 'day' || next === 'week') void changeSpan(next)
                    }}
                  >
                    <Segment value="all">All</Segment>
                    <span
                      className="inline-flex"
                      title={
                        settling
                          ? 'Once the picture is current: it is the one saved last time, still being brought up to date.'
                          : 'What grew and what was freed since the last time this folder was open, or since a snapshot.'
                      }
                    >
                      <Segment value="grown" disabled={settling}>
                        Grown
                      </Segment>
                    </span>
                    <span
                      className="inline-flex gap-0.5"
                      title={journalled ? 'What was written lately, from the NTFS change journal' : 'What was written lately comes from the NTFS change journal, which midda reads after Accelerate.'}
                    >
                      <Segment value="day" disabled={!journalled}>
                        Last day
                      </Segment>
                      <Segment value="week" disabled={!journalled}>
                        Last week
                      </Segment>
                    </span>
                  </SegmentedControl>
                )}

                {stage.at === 'open' && <SnapshotMenu onCompared={showComparison} />}

                {stage.at === 'open' && (
                  <SegmentedControl
                    aria-label="Size to show"
                    value={basis}
                    onValueChange={(next) => {
                      if (next === 'allocated' || next === 'logical') changeBasis(next)
                    }}
                  >
                    <Segment value="allocated">On disk</Segment>
                    <Segment value="logical">Size</Segment>
                  </SegmentedControl>
                )}

                {stage.at === 'opening' || reading ? (
                  <Button variant="ghost" size="sm" onClick={() => void cancelScan()}>
                    Stop
                  </Button>
                ) : (
                  <Button variant="ghost" size="sm" onClick={() => void chooseFolder()}>
                    Choose a folder…
                  </Button>
                )}
              </div>
            }
          >
            {/* The trail from the scan root to what is on screen. In the bar
                rather than over the table, because it is where the window is,
                and a title bar is where a window says that. The whole path is
                its tooltip: the trail folds its middle away when it is deep. */}
            {stage.at === 'open' && showing !== null && (
              <Breadcrumbs
                label="Where you are"
                className="pl-2"
                title={showing.path}
                max={5}
                items={place.trail.map((step) => ({ id: String(step.id), label: step.name }))}
                onSelect={(id) => climbTo(place.trail.findIndex((step) => String(step.id) === id))}
              />
            )}
          </TitleBar>
        }
      >
        {notice !== null && (
          <div className="shrink-0 border-b border-line px-4 py-2">
            <Alert tone="warn">{notice}</Alert>
          </div>
        )}

        <Screen scroll="held" pad="none">
          {stage.at === 'choosing' && volumes !== null && (
            <Volumes volumes={volumes} onPick={(path) => void begin(path)} onBrowse={() => void chooseFolder()} />
          )}

          {stage.at === 'opening' && (
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

          {stage.at === 'open' && view === 'changes' && gathering && (
            <div className="flex flex-1 flex-col items-center justify-center gap-4 p-8">
              <p className="text-sm text-dim">Reading the change journal</p>
              <Progress className="max-w-md" label="Reading the change journal" />
            </div>
          )}

          {stage.at === 'open' && view !== 'changes' && gathering && (
            <div className="flex flex-1 flex-col items-center justify-center gap-4 p-8">
              <p className="text-sm text-dim">Comparing with an earlier picture</p>
              <Progress className="max-w-md" label="Comparing with an earlier picture" />
            </div>
          )}

          {stage.at === 'open' && result !== null && showing !== null && !gathering && (
            <Rows
              view={view}
              arena={place.arena}
              version={view === 'index' ? version : 0}
              result={result}
              changes={changes}
              hours={hours}
              comparison={comparison}
              compareMode={compareMode}
              onCompareMode={setCompareMode}
              onOpenPlace={openPlace}
              trail={place.trail}
              showing={showing}
              sort={sort}
              basis={basis}
              selected={selected}
              onSortChange={changeSort}
              onDescend={descend}
              onDescendById={descendTo}
              onSelect={setSelected}
              onSelectById={selectById}
            />
          )}

          {stage.at === 'open' && showing === null && !gathering && <EmptyState title="Nothing to show" body="The scan came back empty." />}
        </Screen>
      </AppShell>

      {/* A frameless window has no border to grab; these put one back. */}
      <ResizeEdges />
    </>
  )
}
