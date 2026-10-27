import { useCallback, useEffect, useState } from 'react'

import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
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
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import {
  cleanupReach,
  findings,
  isStale,
  pick,
  runCleanup,
  type FindingRow,
  type FindingsSummary,
  type Group,
  type GroupRow,
  type Reach,
  type SizeBasis,
} from '@/core'
import { GROUP_WORDS, describeCleaned, describeKept, describeLastWritten, formatBytes, formatCount, relativeTo } from '@/format'
import { VirtualDisks } from '@/VirtualDisks'

/** How many findings each group lists before adding up the rest. A build
 * folder per project is a few dozen on a working disk; bytecode folders can
 * be hundreds, and the rest of them is one line until asked for. */
const LINES = 25

interface SafeToDeleteProps {
  arena: number
  /** Moves whenever the folder's tree changed: the findings are asked again. */
  version: number
  basis: SizeBasis
  /** The folder opened, as the title names it. */
  root: string
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

/**
 * What can go: the folders the rules find, grouped, the most freed first,
 * each with how it comes back, what is lost and how careful to be.
 *
 * Every figure is what deleting would give back, not what the folder holds:
 * a `node_modules` linked from a package store holds a gigabyte and frees a
 * few megabytes, and its line says both. Nothing is deleted from here - that
 * is the next version's, with its own confirmation (v0.15). What a cache's
 * own tool can clean, it cleans: midda asks the tool where its cache is,
 * runs its command only when that is the folder found, and reads the folder
 * again to say how much came back (ADR 0012). Below them, the virtual disks:
 * what is inside each, and what compacting it gives back (ADR 0015).
 */
export function SafeToDelete({ arena, version, basis, root, onOpen }: SafeToDeleteProps) {
  const [summary, setSummary] = useState<FindingsSummary | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  /** The group listed whole, when the reader asked for one. */
  const [whole, setWhole] = useState<Group | null>(null)
  const [asked, setAsked] = useState<Asked | null>(null)
  /** What the last cleanup did, said at the top until the next one. */
  const [outcome, setOutcome] = useState<{ tone: 'good' | 'warn'; text: string } | null>(null)

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

  if (failure !== null) return <p className="p-4 text-sm text-bad">{failure}</p>

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-6 py-5">
        <header className="min-w-0">
          <h2 className="truncate text-lg font-semibold" title={root}>
            What can go in {root}
          </h2>
          <p className="text-sm text-dim">
            Folders a tool makes again, each with how it comes back · {basis === 'allocated' ? 'on disk' : 'by size'}
          </p>
        </header>

        {outcome !== null && <Alert tone={outcome.tone}>{outcome.text}</Alert>}

        {summary !== null && summary.count > 0 && (
          <StatRow>
            <StatTile
              label="Can be freed"
              tone="accent"
              value={formatBytes(pick(summary.frees, basis))}
              delta={`${formatCount(summary.count)} ${summary.count === 1 ? 'folder' : 'folders'}, holding ${formatBytes(pick(summary.holds, basis))}`}
            />
            <StatTile label="Comes back as it was" value={formatBytes(pick(summary.safe, basis))} delta="by a build or an install" />
            <StatTile label="Look first" value={formatBytes(pick(summary.caution, basis))} delta="may not come back as it was" />
          </StatRow>
        )}

        {summary !== null && summary.count === 0 && (
          <p className="text-sm text-dim">
            Nothing here that a rule knows a tool makes again: no installed dependencies, build output or package caches in this folder.
          </p>
        )}

        {summary?.groups.map((group) => (
          <Findings key={group.group} group={group} basis={basis} root={root} onOpen={onOpen} onClean={ask} onWhole={() => setWhole(group.group)} />
        ))}

        <VirtualDisks arena={arena} version={version} root={root} onOpen={onOpen} />
      </div>

      <CleanupDialog asked={asked} basis={basis} onRun={run} onClose={() => setAsked(null)} />
    </div>
  )
}

interface FindingsProps {
  group: GroupRow
  basis: SizeBasis
  root: string
  onOpen: (id: number) => void
  onClean: (finding: FindingRow) => void
  onWhole: () => void
}

/** One group: its findings, the most freed first, and what is not listed. */
function Findings({ group, basis, root, onOpen, onClean, onWhole }: FindingsProps) {
  return (
    <section>
      <h3 className="mb-2 flex items-baseline gap-2 text-sm font-semibold">
        {GROUP_WORDS[group.group]}
        <span className="font-normal text-dim">· {formatCount(group.count)}</span>
        <span className="tabular ml-auto font-medium text-accent">{formatBytes(pick(group.frees, basis))}</span>
      </h3>
      <ul className="divide-y divide-line border-y border-line">
        {group.rows.map((finding) => (
          <Finding key={finding.id} finding={finding} basis={basis} root={root} onOpen={onOpen} onClean={onClean} />
        ))}
        {group.more > 0 && (
          <li className="flex items-center gap-3 px-3 py-2 text-sm text-dim">
            <span className="tabular w-20 shrink-0 text-right">{formatBytes(pick(group.moreFrees, basis))}</span>
            <span>
              {formatCount(group.more)} more {group.more === 1 ? 'folder' : 'folders'}
            </span>
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
  onOpen: (id: number) => void
  onClean: (finding: FindingRow) => void
}

/** One finding: what it frees, where it is, how careful to be, and why. */
function Finding({ finding, basis, root, onOpen, onClean }: FindingProps) {
  const kept = describeKept(pick(finding.holds, basis), pick(finding.frees, basis))
  const written = describeLastWritten(finding.subtreeModified)
  return (
    <li className="grid grid-cols-[5rem_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 px-3 py-2.5">
      <span className="tabular pt-px text-right text-sm font-medium">{formatBytes(pick(finding.frees, basis))}</span>
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <Button
            variant="link"
            className="block min-w-0 truncate text-left font-mono text-xs text-text"
            title={`${finding.path}\nOpen it in the tree`}
            onClick={() => onOpen(finding.id)}
          >
            {relativeTo(root, finding.path)}
          </Button>
          {/* Said in words as well as in colour: a badge's colour is
              emphasis, never the message. */}
          <Badge variant={finding.risk === 'safe' ? 'good' : 'warn'}>{finding.risk === 'safe' ? 'Safe' : 'Caution'}</Badge>
        </div>
        <p className="mt-1 text-xs text-dim">{finding.explain}</p>
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
      {finding.cleanup !== null ? (
        <Button variant="soft" size="sm" onClick={() => onClean(finding)} title={`${finding.cleanup.explain} midda runs ${finding.cleanup.line} rather than deleting the folder.`}>
          Clean with {finding.cleanup.line.split(' ')[0]}
        </Button>
      ) : (
        <span />
      )}
    </li>
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
