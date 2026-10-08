import { useCallback, useState } from 'react'

import { Button } from '@/components/ui/button'
import {
  ConfirmDialog,
  ConfirmDialogActions,
  ConfirmDialogClose,
  ConfirmDialogDescription,
  ConfirmDialogHeader,
  ConfirmDialogPopup,
  ConfirmDialogTitle,
} from '@/components/ui/confirm-dialog'
import { Popover, PopoverDescription, PopoverPopup, PopoverTitle, PopoverTrigger } from '@/components/ui/popover'
import { Select, SelectItem, SelectPopup, SelectTrigger, SelectValue } from '@/components/ui/select'
import {
  compareSnapshots,
  deleteSnapshot,
  listSnapshots,
  takeSnapshot,
  type ComparisonSummary,
  type SnapshotRow,
  type Snapshots,
} from '@/core'
import { describeSnapshot, formatBytes, formatCount } from '@/format'

/** What "to" is when it is the folder as it is, rather than a snapshot. */
const NOW = 'now'

interface SnapshotMenuProps {
  /** A comparison was made: the window shows it. */
  onCompared: (summary: ComparisonSummary) => void
}

/**
 * The pictures of the open folder: compare two, take one, delete one.
 *
 * A popover rather than a screen: it is beside the control that shows a
 * comparison, and everything in it is a step towards one. The common case
 * needs none of it - "Grown" compares with last time in one press - and this
 * is for the rest: a snapshot taken before a cleanup, compared with after.
 *
 * The list is asked for every time the popover opens rather than kept: a
 * snapshot is a file in the user's data folder, and the window is not the
 * only thing that can delete one.
 */
export function SnapshotMenu({ onCompared }: SnapshotMenuProps) {
  const [open, setOpen] = useState(false)
  const [list, setList] = useState<Snapshots | null>(null)
  const [from, setFrom] = useState<string | null>(null)
  const [to, setTo] = useState<string>(NOW)
  /** What is running: a press of a button the reader should not press twice. */
  const [busy, setBusy] = useState<'taking' | 'comparing' | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  /** The snapshot the reader asked to delete, waiting on their answer. */
  const [doomed, setDoomed] = useState<SnapshotRow | null>(null)

  const reload = useCallback(async (prefer?: string) => {
    try {
      const listed = await listSnapshots()
      setList(listed)
      const usable = listed.snapshots.filter((snapshot) => snapshot.unusable === null)
      // The one just taken, or the one already chosen if it is still there,
      // or last time, or the newest.
      setFrom((held) => {
        const keep = prefer ?? held
        if (keep !== null && usable.some((snapshot) => snapshot.name === keep)) return keep
        return (usable.find((snapshot) => snapshot.kind === 'last') ?? usable[0])?.name ?? null
      })
      setTo((held) => (held === NOW || usable.some((snapshot) => snapshot.name === held) ? held : NOW))
    } catch (cause) {
      setFailure(String(cause))
    }
  }, [])

  const toggle = useCallback(
    (next: boolean) => {
      setOpen(next)
      if (next) {
        setFailure(null)
        void reload()
      }
    },
    [reload],
  )

  const take = useCallback(async () => {
    setBusy('taking')
    setFailure(null)
    try {
      const taken = await takeSnapshot()
      await reload(taken.name)
    } catch (cause) {
      setFailure(String(cause))
    } finally {
      setBusy(null)
    }
  }, [reload])

  const compare = useCallback(async () => {
    if (from === null) return
    setBusy('comparing')
    setFailure(null)
    try {
      const summary = await compareSnapshots(from, to === NOW ? null : to)
      setOpen(false)
      onCompared(summary)
    } catch (cause) {
      setFailure(String(cause))
    } finally {
      setBusy(null)
    }
  }, [from, to, onCompared])

  const remove = useCallback(async () => {
    if (doomed === null) return
    const name = doomed.name
    setDoomed(null)
    try {
      await deleteSnapshot(name)
    } catch (cause) {
      setFailure(String(cause))
    }
    await reload()
  }, [doomed, reload])

  const snapshots = list?.snapshots ?? []
  const usable = snapshots.filter((snapshot) => snapshot.unusable === null)
  const labels: Record<string, string> = { [NOW]: 'Now' }
  for (const snapshot of usable) labels[snapshot.name] = describeSnapshot(snapshot)
  const notNow = list?.notNow ?? null
  // Comparing with now needs the picture on screen to be current; two
  // snapshots do not.
  const blocked = from === null || from === to || (to === NOW && notNow !== null)

  return (
    <>
      <Popover open={open} onOpenChange={toggle}>
        <PopoverTrigger
          render={
            <Button variant="ghost" size="sm" title="Take a snapshot of this folder, or compare two pictures of it">
              Snapshots
            </Button>
          }
        />
        <PopoverPopup size="lg" align="end">
          <PopoverTitle>Snapshots of this folder</PopoverTitle>
          <PopoverDescription>
            Compare two pictures of it: what grew, and what a cleanup gave back. midda keeps the one from the last time this folder was open; take one before a
            cleanup to compare with after.
          </PopoverDescription>

          {/* The comparison first: it is what the popover is for. */}
          <div className="mt-4 flex items-center gap-2">
            <Select value={from} onValueChange={(value) => setFrom(value as string)} items={labels}>
              <SelectTrigger size="sm" aria-label="Compare from" className="min-w-0 flex-1">
                <SelectValue placeholder="Nothing to compare yet" />
              </SelectTrigger>
              <SelectPopup>
                {usable.map((snapshot) => (
                  <SelectItem key={snapshot.name} value={snapshot.name}>
                    {labels[snapshot.name]}
                  </SelectItem>
                ))}
              </SelectPopup>
            </Select>
            <span className="text-xs text-dim" aria-hidden>
              →
            </span>
            <Select value={to} onValueChange={(value) => setTo(value as string)} items={labels}>
              <SelectTrigger size="sm" aria-label="Compare to" className="min-w-0 flex-1">
                <SelectValue />
              </SelectTrigger>
              <SelectPopup>
                <SelectItem value={NOW}>Now</SelectItem>
                {usable.map((snapshot) => (
                  <SelectItem key={snapshot.name} value={snapshot.name}>
                    {labels[snapshot.name]}
                  </SelectItem>
                ))}
              </SelectPopup>
            </Select>
            <Button variant="primary" size="sm" disabled={blocked || busy !== null} onClick={() => void compare()}>
              {busy === 'comparing' ? 'Comparing…' : 'Compare'}
            </Button>
          </div>

          {/* Why a button above or below is not there to press, said rather
              than left for the reader to guess. */}
          {notNow !== null && <p className="mt-2 text-xs text-warn">Not with now yet: {notNow}.</p>}
          {failure !== null && <p className="mt-2 text-xs text-bad">{failure}</p>}

          <ul className="mt-4 divide-y divide-line border-y border-line" aria-label="Snapshots">
            {snapshots.length === 0 && list !== null && (
              <li className="py-2 text-xs text-dim">
                None yet. The picture of last time appears the next time this folder is opened; one taken now appears at once.
              </li>
            )}
            {snapshots.map((snapshot) => (
              <li key={snapshot.name} className="flex items-center gap-3 py-1.5 text-xs">
                <span className="min-w-0 flex-1">
                  <span className={snapshot.unusable === null ? 'block truncate' : 'block truncate text-dim'}>{describeSnapshot(snapshot)}</span>
                  {snapshot.unusable !== null && <span className="block truncate text-2xs text-warn">{snapshot.unusable}</span>}
                </span>
                <span className="tabular shrink-0 text-dim">{formatBytes(snapshot.bytes)}</span>
                <Button variant="danger" size="xs" onClick={() => setDoomed(snapshot)} aria-label={`Delete the snapshot: ${describeSnapshot(snapshot)}`}>
                  Delete
                </Button>
              </li>
            ))}
          </ul>
          {list !== null && list.lastNote !== null && <p className="mt-2 text-xs text-warn">{list.lastNote}</p>}

          <div className="mt-3 flex items-center gap-3">
            {/* What they cost, because the product is a disk analyzer and
                these are files on the disk. */}
            <span className="tabular flex-1 text-xs text-dim">
              {list === null ? '' : `${formatCount(snapshots.length)} ${snapshots.length === 1 ? 'snapshot' : 'snapshots'} · ${formatBytes(list.bytes)} on this disk`}
            </span>
            <Button
              variant="soft"
              size="sm"
              disabled={notNow !== null || busy !== null}
              title={notNow ?? 'Keep a picture of the folder as it is now, to compare with later'}
              onClick={() => void take()}
            >
              {busy === 'taking' ? 'Taking…' : 'Take a snapshot'}
            </Button>
          </div>
        </PopoverPopup>
      </Popover>

      {/* Deleting a snapshot is the one thing here that cannot be undone: the
          moment it pictured cannot be taken again. */}
      <ConfirmDialog open={doomed !== null} onOpenChange={(next) => !next && setDoomed(null)}>
        <ConfirmDialogPopup>
          <ConfirmDialogHeader>
            <ConfirmDialogTitle>Delete this snapshot?</ConfirmDialogTitle>
            <ConfirmDialogDescription>
              {doomed === null ? '' : `${describeSnapshot(doomed)} · ${formatBytes(doomed.bytes)}. `}
              The folder cannot be pictured as it was at that moment again.
            </ConfirmDialogDescription>
          </ConfirmDialogHeader>
          <ConfirmDialogActions>
            <Button variant="ghost" render={<ConfirmDialogClose />}>
              Keep it
            </Button>
            <Button variant="danger" onClick={() => void remove()}>
              Delete
            </Button>
          </ConfirmDialogActions>
        </ConfirmDialogPopup>
      </ConfirmDialog>
    </>
  )
}
