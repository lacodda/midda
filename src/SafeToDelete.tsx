import { useCallback, useEffect, useMemo, useState } from 'react'

import { ActionBar } from '@/components/ui/action-bar'
import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  ConfirmDialog,
  ConfirmDialogActions,
  ConfirmDialogBody,
  ConfirmDialogClose,
  ConfirmDialogDescription,
  ConfirmDialogHeader,
  ConfirmDialogPopup,
  ConfirmDialogTitle,
} from '@/components/ui/confirm-dialog'
import { Copyable } from '@/components/ui/copyable'
import { Progress } from '@/components/ui/progress'
import { Segment, SegmentedControl } from '@/components/ui/segmented-control'
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import {
  cancelDelete,
  cleanupReach,
  deleteFindings,
  deletionLog,
  findings,
  isStale,
  keepEntry,
  pick,
  runCleanup,
  type DeletingRow,
  type DeletionLog,
  type FindingRow,
  type FindingsSummary,
  type Group,
  type GroupRow,
  type Reach,
  type SizeBasis,
  type Way,
} from '@/core'
import {
  GROUP_WORDS,
  describeCleaned,
  describeDeleted,
  describeKept,
  describeLastWritten,
  formatAgo,
  formatBytes,
  formatCount,
  relativeTo,
} from '@/format'
import { OwnRules } from '@/OwnRules'
import { VirtualDisks } from '@/VirtualDisks'

/** How many findings each group lists before adding up the rest. A build
 * folder per project is a few dozen on a working disk; bytecode folders can
 * be hundreds, and the rest of them is one line until asked for. */
const LINES = 25

/** How many deletions the log lists. */
const LOGGED = 20

interface SafeToDeleteProps {
  arena: number
  /** Moves whenever the folder's tree changed: the findings are asked again. */
  version: number
  basis: SizeBasis
  /** The folder opened, as the title names it. */
  root: string
  /** A deletion running now, as the session says - followed in the dialog. */
  deleting: DeletingRow | null
  /** A finding was chosen: the window shows its folder in the tree. */
  onOpen: (id: number) => void
}

/** A cleanup the reader asked for: its finding, and what its tool said. */
interface Asked {
  finding: FindingRow
  /** `null` while the tool is being asked where it cleans. */
  reach: Reach | null
  running: boolean
}

/** How a group's picks go: each its own way - for good when it comes back
 * as it was, into the Recycle Bin when it may not - or all one way. */
type GroupWay = 'each' | Way

/** The way a finding picked goes, by its group's switch. What cannot go to
 * the bin goes for good, whatever the switch says: the confirmation says so,
 * and asks again for a careful one. */
function wayOf(finding: FindingRow, mode: GroupWay): Way {
  if (mode === 'each') return finding.way
  if (mode === 'recycle-bin') return finding.binRefused === null ? 'recycle-bin' : 'for-good'
  return 'for-good'
}

/** A finding picked, with how it goes. */
interface Picked {
  finding: FindingRow
  group: Group
  way: Way
}

/**
 * What can go: what the rules find, grouped, the most freed first, each with
 * how it comes back, what is lost and how careful to be - a developer's
 * dependencies, builds and caches, and what Windows and the reader's
 * downloads leave behind (ADR 0016).
 *
 * Every figure is what deleting would give back, not what the folder holds:
 * a `node_modules` linked from a package store holds a gigabyte and frees a
 * few megabytes, and its line says both. What a cache's own tool can clean,
 * it cleans: midda asks the tool where its cache is, runs its command only
 * when that is the folder found, and reads the folder again to say how much
 * came back (ADR 0012).
 *
 * What the reader picks, midda deletes (ADR 0018): what comes back as it was
 * for good - in the Recycle Bin it would only move, and free nothing until
 * the bin is emptied - and what may not into the bin. A group's switch sends
 * all of it one way; the confirmation lists both ways by name, and deleting a
 * careful one for good is asked for a second time. Below, the log of what
 * was deleted, each with how it comes back. Below that, the virtual disks:
 * what is inside each, and what compacting it gives back (ADR 0015).
 */
export function SafeToDelete({ arena, version, basis, root, deleting, onOpen }: SafeToDeleteProps) {
  const [summary, setSummary] = useState<FindingsSummary | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  /** The group listed whole, when the reader asked for one. */
  const [whole, setWhole] = useState<Group | null>(null)
  const [asked, setAsked] = useState<Asked | null>(null)
  /** What the last cleanup or deletion did, said at the top until the next. */
  const [outcome, setOutcome] = useState<{ tone: 'good' | 'warn'; text: string } | null>(null)
  /** The findings picked, by id - in the ids of `arena`. */
  const [picked, setPicked] = useState<{ arena: number; ids: ReadonlySet<number> }>({ arena, ids: new Set() })
  const [modes, setModes] = useState<Partial<Record<Group, GroupWay>>>({})
  /** Whether the confirmation is open, and how far it has gone. */
  const [confirming, setConfirming] = useState<'review' | 'again' | 'running' | null>(null)
  const [log, setLog] = useState<DeletionLog | null>(null)

  useEffect(() => {
    let current = true
    findings(arena, basis, LINES, whole)
      .then((next) => {
        if (current) {
          setSummary(next)
          setFailure(null)
        }
      })
      .catch((cause: unknown) => {
        if (current && !isStale(cause)) setFailure(String(cause))
      })
    return () => {
      current = false
    }
  }, [arena, version, basis, whole])

  useEffect(() => {
    let current = true
    deletionLog(LOGGED)
      .then((next) => current && setLog(next))
      .catch(() => current && setLog(null))
    return () => {
      current = false
    }
  }, [version])

  /** Every finding listed, by id: what a pick is read against. */
  const listed = useMemo(() => {
    const by = new Map<number, { finding: FindingRow; group: Group }>()
    if (summary?.arena !== arena) return by
    for (const group of summary.groups) for (const finding of group.rows) by.set(finding.id, { finding, group: group.group })
    return by
  }, [summary, arena])

  /** The picks that are findings now: one that went, or whose tree was read
   * anew, is not picked any more. */
  const picks: Picked[] = useMemo(() => {
    if (picked.arena !== arena) return []
    const out: Picked[] = []
    for (const id of picked.ids) {
      const held = listed.get(id)
      if (held !== undefined) out.push({ ...held, way: wayOf(held.finding, modes[held.group] ?? 'each') })
    }
    return out
  }, [picked, listed, arena, modes])

  const toggle = useCallback(
    (ids: number[], on: boolean) =>
      setPicked((held) => {
        const next = new Set(held.arena === arena ? held.ids : [])
        for (const id of ids) {
          if (on) next.add(id)
          else next.delete(id)
        }
        return { arena, ids: next }
      }),
    [arena],
  )

  const ask = useCallback(
    (finding: FindingRow) => {
      setAsked({ finding, reach: null, running: false })
      cleanupReach(arena, finding.id)
        .then((reach) => setAsked((held) => (held?.finding.id === finding.id ? { ...held, reach } : held)))
        .catch((cause: unknown) =>
          setAsked((held) =>
            held?.finding.id === finding.id
              ? { ...held, reach: { line: finding.cleanup?.line ?? '', cleans: null, refused: isStale(cause) ? 'the folder changed; open this page again' : String(cause) } }
              : held,
          ),
        )
    },
    [arena],
  )

  const run = useCallback(() => {
    if (asked === null) return
    const { finding } = asked
    setAsked({ ...asked, running: true })
    runCleanup(arena, finding.id)
      .then((cleaned) => setOutcome({ tone: cleaned.ran.succeeded ? 'good' : 'warn', text: describeCleaned(cleaned, basis) }))
      .catch((cause: unknown) => setOutcome({ tone: 'warn', text: `${finding.cleanup?.line ?? 'The cleanup'} did not run: ${String(cause)}` }))
      .finally(() => setAsked(null))
  }, [arena, asked, basis])

  const remove = useCallback(() => {
    setConfirming('running')
    deleteFindings(
      arena,
      picks.map(({ finding, way }) => ({ id: finding.id, way })),
    )
      .then((deleted) => {
        const whole = deleted.items.every((item) => item.removed.left.length === 0) && !deleted.stopped && deleted.unlogged === null
        setOutcome({ tone: whole ? 'good' : 'warn', text: describeDeleted(deleted, basis) })
      })
      .catch((cause: unknown) =>
        setOutcome({ tone: 'warn', text: isStale(cause) ? 'The folder changed before anything was deleted; pick again.' : `Nothing was deleted: ${String(cause)}` }),
      )
      .finally(() => {
        setConfirming(null)
        setPicked({ arena, ids: new Set() })
      })
  }, [arena, picks, basis])

  const keep = useCallback(
    (finding: FindingRow) => {
      keepEntry(arena, finding.id, null)
        .then(() =>
          setOutcome({
            tone: 'good',
            text: `${relativeTo(root, finding.path)} is kept: it is not listed again, nor anything in it. Stop keeping it under Kept, below.`,
          }),
        )
        .catch((cause: unknown) =>
          setOutcome({ tone: 'warn', text: isStale(cause) ? 'The folder changed; keep it once it is listed again.' : `It was not kept: ${String(cause)}` }),
        )
    },
    [arena, root],
  )

  if (failure !== null) return <p className="p-4 text-sm text-bad">{failure}</p>

  const forGood = picks.filter((held) => held.way === 'for-good')
  const binned = picks.filter((held) => held.way === 'recycle-bin')
  const frees = (of: Picked[]) => of.reduce((sum, held) => sum + pick(held.finding.frees, basis), 0)

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-6 py-5">
          <header className="min-w-0">
            <h2 className="truncate text-lg font-semibold" title={root}>
              What can go in {root}
            </h2>
            <p className="text-sm text-dim">
              What a tool makes again or nothing needs, each with what is lost · {basis === 'allocated' ? 'on disk' : 'by size'}
            </p>
          </header>

          {outcome !== null && <Alert tone={outcome.tone}>{outcome.text}</Alert>}

          {summary !== null && summary.count > 0 && (
            <StatRow>
              <StatTile
                label="Can be freed"
                tone="accent"
                value={formatBytes(pick(summary.frees, basis))}
                delta={`${formatCount(summary.count)} found, holding ${formatBytes(pick(summary.holds, basis))}`}
              />
              <StatTile label="Nothing needed is lost" value={formatBytes(pick(summary.safe, basis))} delta="comes back, or nothing uses it" />
              <StatTile label="Look first" value={formatBytes(pick(summary.caution, basis))} delta="may not come back as it was" />
            </StatRow>
          )}

          {summary !== null && summary.count === 0 && (
            <p className="text-sm text-dim">
              Nothing here that a rule knows can go: no installed dependencies, build output or caches, nothing in a Recycle Bin, temporary folder, browser
              cache or crash report, nothing Windows keeps that it no longer needs, no old downloads, empty folders or broken shortcuts - or nothing you
              have not kept.
            </p>
          )}

          {summary?.groups.map((group) => (
            <Findings
              key={group.group}
              group={group}
              basis={basis}
              root={root}
              mode={modes[group.group] ?? 'each'}
              picked={picked.arena === arena ? picked.ids : new Set()}
              onMode={(mode) => setModes((held) => ({ ...held, [group.group]: mode }))}
              onPick={toggle}
              onOpen={onOpen}
              onClean={ask}
              onKeep={keep}
              onWhole={() => setWhole(group.group)}
            />
          ))}

          <OwnRules version={version} onChanged={() => setOutcome(null)} />

          {log !== null && log.count > 0 && <Deleted log={log} root={root} />}

          <VirtualDisks arena={arena} version={version} root={root} onOpen={onOpen} />
        </div>
      </div>

      {picks.length > 0 && (
        <ActionBar position="bottom" justify="between" aria-label="Picked to delete" className="px-6">
          <p className="tabular min-w-0 text-sm">
            <span className="font-medium">{formatCount(picks.length)} picked</span>
            <span className="text-dim">
              {forGood.length > 0 && ` · ${formatCount(forGood.length)} for good, up to ${formatBytes(frees(forGood))} back now`}
              {binned.length > 0 && ` · ${formatCount(binned.length)} to the Recycle Bin, ${formatBytes(frees(binned))} once it is emptied`}
            </span>
          </p>
          <div className="flex shrink-0 gap-2">
            <Button variant="ghost" size="sm" onClick={() => setPicked({ arena, ids: new Set() })}>
              Clear
            </Button>
            <Button variant="primary" size="sm" onClick={() => setConfirming('review')} disabled={deleting !== null}>
              Delete…
            </Button>
          </div>
        </ActionBar>
      )}

      <CleanupDialog asked={asked} basis={basis} onRun={run} onClose={() => setAsked(null)} />
      <DeleteDialog
        stage={confirming}
        picks={picks}
        basis={basis}
        root={root}
        deleting={deleting}
        onAgain={() => setConfirming('again')}
        onBack={() => setConfirming('review')}
        onDelete={remove}
        onClose={() => setConfirming(null)}
      />
    </div>
  )
}

interface FindingsProps {
  group: GroupRow
  basis: SizeBasis
  root: string
  mode: GroupWay
  picked: ReadonlySet<number>
  onMode: (mode: GroupWay) => void
  onPick: (ids: number[], on: boolean) => void
  onOpen: (id: number) => void
  onClean: (finding: FindingRow) => void
  onKeep: (finding: FindingRow) => void
  onWhole: () => void
}

/** One group: its findings, the most freed first, what is not listed, and
 * the switch that sends what is picked of it one way. */
function Findings({ group, basis, root, mode, picked, onMode, onPick, onOpen, onClean, onKeep, onWhole }: FindingsProps) {
  const ids = group.rows.map((finding) => finding.id)
  const count = ids.filter((id) => picked.has(id)).length
  // The Recycle Bin's own contents go nowhere but for good.
  const recyclable = group.rows.some((finding) => finding.binRefused === null)
  return (
    <section>
      <div className="mb-2 flex items-center gap-3">
        <Checkbox
          checked={count > 0 && count === ids.length}
          indeterminate={count > 0 && count < ids.length}
          onCheckedChange={(on) => onPick(ids, on)}
          aria-label={`Pick every ${GROUP_WORDS[group.group].toLowerCase()} entry listed`}
        />
        <h3 className="flex min-w-0 flex-1 items-baseline gap-2 text-sm font-semibold">
          {GROUP_WORDS[group.group]}
          <span className="font-normal text-dim">· {formatCount(group.count)}</span>
        </h3>
        {count > 0 && recyclable && (
          <SegmentedControl
            aria-label={`How the picked ${GROUP_WORDS[group.group].toLowerCase()} go`}
            value={mode}
            onValueChange={(next) => {
              if (next === 'each' || next === 'for-good' || next === 'recycle-bin') onMode(next)
            }}
          >
            <span className="inline-flex" title="What comes back as it was is deleted for good; what may not goes to the Recycle Bin.">
              <Segment value="each">Each its way</Segment>
            </span>
            <span className="inline-flex" title="Everything picked here goes to the Recycle Bin: it can be restored, and the space comes back once the bin is emptied.">
              <Segment value="recycle-bin">Recycle Bin</Segment>
            </span>
            <span className="inline-flex" title="Everything picked here is deleted for good; the careful ones are asked about again.">
              <Segment value="for-good">For good</Segment>
            </span>
          </SegmentedControl>
        )}
        <span className="tabular font-medium text-accent">{formatBytes(pick(group.frees, basis))}</span>
      </div>
      <ul className="divide-y divide-line border-y border-line">
        {group.rows.map((finding) => (
          <Finding
            key={finding.id}
            finding={finding}
            basis={basis}
            root={root}
            way={picked.has(finding.id) ? wayOf(finding, mode) : null}
            onPick={(on) => onPick([finding.id], on)}
            onOpen={onOpen}
            onClean={onClean}
            onKeep={onKeep}
          />
        ))}
        {group.more > 0 && (
          <li className="flex items-center gap-3 px-3 py-2 text-sm text-dim">
            <span className="tabular w-24 shrink-0 text-right">{formatBytes(pick(group.moreFrees, basis))}</span>
            <span>{formatCount(group.more)} more</span>
            <Button variant="link" className="ml-auto" onClick={onWhole}>
              Show all
            </Button>
          </li>
        )}
      </ul>
    </section>
  )
}

interface FindingProps {
  finding: FindingRow
  basis: SizeBasis
  root: string
  /** How it goes, when it is picked. */
  way: Way | null
  onPick: (on: boolean) => void
  onOpen: (id: number) => void
  onClean: (finding: FindingRow) => void
  onKeep: (finding: FindingRow) => void
}

/** One finding: whether it is picked and how it would go, what it frees,
 * where it is, how careful to be, and why - and Keep, for one that is the
 * reader's and is never to be listed again. */
function Finding({ finding, basis, root, way, onPick, onOpen, onClean, onKeep }: FindingProps) {
  const kept = describeKept(pick(finding.holds, basis), pick(finding.frees, basis))
  const written = describeLastWritten(finding.subtreeModified)
  const relative = relativeTo(root, finding.path)
  return (
    <li className="grid grid-cols-[1rem_5rem_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 px-3 py-2.5">
      <Checkbox className="mt-0.5" checked={way !== null} onCheckedChange={onPick} aria-label={`Pick ${relative}`} />
      <span className="tabular pt-px text-right text-sm font-medium">{formatBytes(pick(finding.frees, basis))}</span>
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <Button
            variant="link"
            className="block min-w-0 truncate text-left font-mono text-xs text-text"
            title={`${finding.path}\nOpen it in the tree`}
            onClick={() => onOpen(finding.id)}
          >
            {relative}
          </Button>
          {/* Said in words as well as in colour: a badge's colour is
              emphasis, never the message. */}
          <Badge variant={finding.risk === 'safe' ? 'good' : 'warn'}>{finding.risk === 'safe' ? 'Safe' : 'Caution'}</Badge>
          {way !== null && <span className="shrink-0 text-xs text-dim">{way === 'for-good' ? '→ deleted for good' : '→ Recycle Bin'}</span>}
        </div>
        <p className="mt-1 text-xs text-dim">{finding.explain}</p>
        {/* What only this one says: where a shortcut points, what of an
            installer is installed, how much of a folder goes. */}
        {finding.detail !== null && <p className="mt-0.5 text-xs text-text">{finding.detail}</p>}
        <p className="mt-0.5 text-xs text-dim">
          {[
            finding.command === null ? null : (
              <span key="command">
                Comes back with <code className="font-mono text-text">{finding.command}</code>
              </span>
            ),
            kept === null ? null : <span key="kept">{kept}</span>,
            written === null ? null : <span key="written">{written}</span>,
          ]
            .filter((part) => part !== null)
            .flatMap((part, index) => (index === 0 ? [part] : [<span key={`dot${index}`}> · </span>, part]))}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {finding.cleanup !== null && (
          <Button variant="soft" size="sm" onClick={() => onClean(finding)} title={`${finding.cleanup.explain} midda runs ${finding.cleanup.line} rather than deleting the folder.`}>
            Clean with {finding.cleanup.line.split(' ')[0]}
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          onClick={() => onKeep(finding)}
          aria-label={`Keep ${relative}`}
          title="Mine, and never to go: midda does not list it again, nor anything in it."
        >
          Keep
        </Button>
      </div>
    </li>
  )
}

interface DeleteDialogProps {
  stage: 'review' | 'again' | 'running' | null
  picks: Picked[]
  basis: SizeBasis
  root: string
  deleting: DeletingRow | null
  onAgain: () => void
  onBack: () => void
  onDelete: () => void
  onClose: () => void
}

/**
 * The question before anything is deleted: every entry picked, by the way it
 * goes - for good, or into the Recycle Bin - each named, with what it frees
 * and how it comes back. A careful one going for good is asked about a
 * second time, on its own: it may not come back as it was, and nothing will
 * restore it. While it runs: how far, and Stop.
 */
function DeleteDialog({ stage, picks, basis, root, deleting, onAgain, onBack, onDelete, onClose }: DeleteDialogProps) {
  // The most freed first, as they go.
  const ordered = [...picks].sort((a, b) => pick(b.finding.frees, basis) - pick(a.finding.frees, basis))
  const forGood = ordered.filter((held) => held.way === 'for-good')
  const binned = ordered.filter((held) => held.way === 'recycle-bin')
  const careful = forGood.filter((held) => held.finding.risk === 'caution')
  const one = careful.length === 1
  const frees = (of: Picked[]) => of.reduce((sum, held) => sum + pick(held.finding.frees, basis), 0)
  const running = stage === 'running'
  return (
    <ConfirmDialog open={stage !== null} onOpenChange={(next) => !next && !running && onClose()}>
      <ConfirmDialogPopup size="lg">
        <ConfirmDialogHeader>
          <ConfirmDialogTitle>
            {stage === 'again' ? `Delete for good what may not come back?` : `Delete ${formatCount(picks.length)} picked?`}
          </ConfirmDialogTitle>
          <ConfirmDialogDescription>
            {stage === 'again'
              ? `${one ? 'This one is' : `These ${formatCount(careful.length)} are`} marked Caution: ${one ? 'it' : 'they'} may not come back as ${one ? 'it was' : 'they were'}, or may be the only copy. Deleted for good, nothing restores ${one ? 'it' : 'them'} - not the Recycle Bin, not midda.${picks.length > careful.length ? ` The other ${formatCount(picks.length - careful.length)} picked go as listed before.` : ''}`
              : 'What is deleted for good gives its space back now. What goes to the Recycle Bin, Explorer restores; its space comes back once the bin is emptied. midda measures each again afterwards and writes down how it comes back.'}
          </ConfirmDialogDescription>
        </ConfirmDialogHeader>
        <ConfirmDialogBody className="flex max-h-[50vh] flex-col gap-4 overflow-y-auto">
          {running ? (
            <Running deleting={deleting} root={root} />
          ) : stage === 'again' ? (
            <Listed title="Deleted for good, marked Caution" picks={careful} basis={basis} root={root} />
          ) : (
            <>
              {forGood.length > 0 && (
                <Listed title={`Deleted for good · ${formatBytes(frees(forGood))} back now`} picks={forGood} basis={basis} root={root} />
              )}
              {binned.length > 0 && (
                <Listed title={`To the Recycle Bin · ${formatBytes(frees(binned))} back once the bin is emptied`} picks={binned} basis={basis} root={root} />
              )}
            </>
          )}
        </ConfirmDialogBody>
        <ConfirmDialogActions>
          {running ? (
            <Button variant="ghost" onClick={() => void cancelDelete()}>
              Stop
            </Button>
          ) : stage === 'again' ? (
            <>
              <Button variant="ghost" onClick={onBack}>
                Back
              </Button>
              <Button variant="danger" onClick={onDelete}>
                {one ? 'Delete it for good' : 'Delete them for good'}
              </Button>
            </>
          ) : (
            <>
              <Button variant="ghost" render={<ConfirmDialogClose />}>
                Cancel
              </Button>
              {careful.length > 0 ? (
                <Button variant="primary" onClick={onAgain}>
                  Continue
                </Button>
              ) : (
                <Button variant="primary" onClick={onDelete}>
                  {binned.length === 0 ? 'Delete for good' : forGood.length === 0 ? 'Move to the Recycle Bin' : 'Delete and move to the bin'}
                </Button>
              )}
            </>
          )}
        </ConfirmDialogActions>
      </ConfirmDialogPopup>
    </ConfirmDialog>
  )
}

/** The entries one way takes, each with what it frees and how it comes back. */
function Listed({ title, picks, basis, root }: { title: string; picks: Picked[]; basis: SizeBasis; root: string }) {
  return (
    <section>
      <h4 className="mb-1.5 text-sm font-semibold">
        {title} <span className="font-normal text-dim">· {formatCount(picks.length)}</span>
      </h4>
      <ul className="divide-y divide-line border-y border-line">
        {picks.map(({ finding }) => (
          <li key={finding.id} className="grid grid-cols-[5rem_minmax(0,1fr)] gap-x-3 px-2 py-1.5">
            <span className="tabular text-right text-xs font-medium">{formatBytes(pick(finding.frees, basis))}</span>
            <div className="min-w-0">
              <p className="flex min-w-0 items-center gap-2">
                <span className="truncate font-mono text-xs" title={finding.path}>
                  {relativeTo(root, finding.path)}
                </span>
                {finding.risk === 'caution' && <Badge variant="warn">Caution</Badge>}
              </p>
              <p className="text-xs text-dim">
                {finding.command !== null ? (
                  <>
                    Comes back with <code className="font-mono text-text">{finding.command}</code>
                  </>
                ) : finding.binRefused !== null && finding.way === 'for-good' ? (
                  `Not to the Recycle Bin: ${finding.binRefused}.`
                ) : (
                  finding.name
                )}
              </p>
            </div>
          </li>
        ))}
      </ul>
    </section>
  )
}

/** How far a deletion is, as the session says. */
function Running({ deleting, root }: { deleting: DeletingRow | null; root: string }) {
  if (deleting === null) return <Progress label="Deleting" />
  return (
    <div className="flex flex-col gap-2">
      <Progress value={deleting.of > 0 ? Math.min(deleting.bytes, deleting.of) : null} max={Math.max(deleting.of, 1)} label="Deleting" />
      <p className="tabular text-xs text-dim">
        {formatCount(deleting.done)} of {formatCount(deleting.items)} done · {formatCount(deleting.files)} {deleting.files === 1 ? 'file' : 'files'} ·{' '}
        {formatBytes(deleting.bytes)} of {formatBytes(deleting.of)}
      </p>
      <p className="truncate font-mono text-xs text-dim" title={deleting.path}>
        {relativeTo(root, deleting.path)}
      </p>
    </div>
  )
}

/** What midda deleted, the latest first, each with how it comes back. */
function Deleted({ log, root }: { log: DeletionLog; root: string }) {
  return (
    <section>
      <h3 className="mb-2 flex items-baseline gap-2 text-sm font-semibold">
        Deleted lately
        <span className="font-normal text-dim">
          · {formatCount(log.count)} in all, {formatBytes(log.freed)} given back for good
        </span>
      </h3>
      <ul className="divide-y divide-line border-y border-line">
        {log.rows.map((deletion) => (
          <li key={`${deletion.at}-${deletion.path}`} className="grid grid-cols-[5rem_minmax(0,1fr)] gap-x-3 px-3 py-2">
            <span className="tabular pt-px text-right text-sm">{formatBytes(deletion.gone)}</span>
            <div className="min-w-0">
              <p className="flex min-w-0 items-center gap-2 text-xs">
                <span className="truncate font-mono text-text" title={deletion.path}>
                  {relativeTo(root, deletion.path)}
                </span>
                <span className="shrink-0 text-dim">
                  {deletion.way === 'for-good' ? 'deleted for good' : 'moved to the Recycle Bin'} {formatAgo(deletion.at)}
                </span>
              </p>
              <p className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1 text-xs text-dim">
                {deletion.way === 'recycle-bin' ? (
                  'In the Recycle Bin: Explorer restores it.'
                ) : deletion.command !== null ? (
                  <>
                    Comes back with
                    <Copyable label={`Copy ${deletion.command}`} copiedLabel="Copied" className="font-mono text-text">
                      {deletion.command}
                    </Copyable>
                    {deletion.runIn !== null && (
                      <>
                        run in
                        <Copyable label="Copy the folder" copiedLabel="Copied" className="font-mono text-text">
                          {deletion.runIn}
                        </Copyable>
                      </>
                    )}
                  </>
                ) : (
                  deletion.explain
                )}
                {deletion.left > 0 && <span> · {formatCount(deletion.left)} stayed</span>}
              </p>
            </div>
          </li>
        ))}
      </ul>
      <p className="mt-1.5 text-xs text-dim">
        Kept in <span className="font-mono">{log.file}</span>
      </p>
    </section>
  )
}

interface CleanupDialogProps {
  asked: Asked | null
  basis: SizeBasis
  onRun: () => void
  onClose: () => void
}

/**
 * The question before a cache's own tool runs: what it does, how much can
 * come back, and - once the tool has answered - whether this is the cache it
 * cleans. The command runs only on a yes, and only when it is.
 */
function CleanupDialog({ asked, basis, onRun, onClose }: CleanupDialogProps) {
  const finding = asked?.finding ?? null
  const reach = asked?.reach ?? null
  const line = reach?.line ?? finding?.cleanup?.line ?? ''
  const ready = reach !== null && reach.refused === null && !(asked?.running ?? false)
  return (
    <ConfirmDialog open={asked !== null} onOpenChange={(next) => !next && !(asked?.running ?? false) && onClose()}>
      <ConfirmDialogPopup size="lg">
        <ConfirmDialogHeader>
          <ConfirmDialogTitle>Run {line}?</ConfirmDialogTitle>
          <ConfirmDialogDescription>
            {finding?.cleanup?.explain} Up to {finding === null ? '' : formatBytes(pick(finding.frees, basis))} can come back: what only this folder holds. midda
            measures the folder again afterwards.
          </ConfirmDialogDescription>
        </ConfirmDialogHeader>
        <ConfirmDialogBody>
          {reach === null ? (
            <p className="text-sm text-dim">Asking {line.split(' ')[0]} which folder it cleans…</p>
          ) : reach.refused !== null ? (
            <Alert tone="warn" title="Not run on this folder">
              {reach.refused}
            </Alert>
          ) : (
            <p className="text-sm text-dim">
              {line.split(' ')[0]} cleans <span className="font-mono text-xs text-text">{reach.cleans}</span>, which is this folder.
            </p>
          )}
        </ConfirmDialogBody>
        <ConfirmDialogActions>
          <Button variant="ghost" render={<ConfirmDialogClose />} disabled={asked?.running ?? false}>
            Cancel
          </Button>
          <Button variant="primary" onClick={onRun} disabled={!ready}>
            {asked?.running ? 'Running…' : `Run ${line}`}
          </Button>
        </ConfirmDialogActions>
      </ConfirmDialogPopup>
    </ConfirmDialog>
  )
}
