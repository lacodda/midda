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
import { Progress } from '@/components/ui/progress'
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import {
  cancelCompress,
  compressEntry,
  isFossil,
  isStale,
  layers,
  pick,
  type CompressingRow,
  type FossilRow,
  type Layer,
  type LayersSummary,
  type NotCompressed,
  type SizeBasis,
} from '@/core'
import {
  describeCompressed,
  describeLastOpened,
  describeLastWritten,
  describeStratum,
  formatBytes,
  formatCount,
  formatShare,
  relativeTo,
  STRATUM_WORDS,
} from '@/format'

/** How many fossils are listed before the rest is added up. A disk years old
 * has hundreds; the largest few dozen are where the space is. */
const LINES = 30

interface LayersProps {
  arena: number
  /** Moves whenever the folder's tree changed: the layers are asked again. */
  version: number
  basis: SizeBasis
  /** The folder opened, as the title names it. */
  root: string
  /** A compression running now, as the session says - followed here. */
  compressing: CompressingRow | null
  /** A fossil was chosen: the window shows it in the tree of what is untouched. */
  onOpen: (id: number) => void
}

/**
 * How old the layers of the folder are - the view the product is named for.
 *
 * A bar of its bytes by how long ago each file was written, the youngest
 * first, and under it the largest fossils: what nothing has written to for a
 * year, each named whole when nothing in it moved and inside no larger one,
 * so the list adds up to the old bands of the bar (ADR 0017). Each says when
 * it was written, when a file in it was last opened where the volume keeps
 * that, and whether it can be compressed in place - space back with nothing
 * deleted.
 */
export function Layers({ arena, version, basis, root, compressing, onOpen }: LayersProps) {
  const [summary, setSummary] = useState<LayersSummary | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  /** The fossil the reader asked to compress, while the question is open. */
  const [asked, setAsked] = useState<FossilRow | null>(null)
  const [running, setRunning] = useState(false)
  /** What the last compression did, said at the top until the next one. */
  const [outcome, setOutcome] = useState<{ tone: 'good' | 'warn'; text: string } | null>(null)

  useEffect(() => {
    let current = true
    layers(arena, basis, LINES)
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
  }, [arena, version, basis])

  const run = useCallback(() => {
    if (asked === null) return
    setRunning(true)
    compressEntry(arena, asked.id)
      .then((compressed) => {
        const back = compressed.after.allocated < compressed.before.allocated
        setOutcome({ tone: back && compressed.done.left.every((kept) => kept.why !== 'failed') ? 'good' : 'warn', text: describeCompressed(compressed) })
      })
      .catch((cause: unknown) =>
        setOutcome({ tone: 'warn', text: isStale(cause) ? 'The folder changed before it could be compressed; open this page again.' : `Not compressed: ${String(cause)}` }),
      )
      .finally(() => {
        setRunning(false)
        setAsked(null)
      })
  }, [arena, asked])

  if (failure !== null) return <p className="p-4 text-sm text-bad">{failure}</p>

  const total = summary === null ? 0 : summary.strata.reduce((sum, layer) => sum + pick(layer.size, basis), 0)
  const old = summary === null ? 0 : pick(summary.size, basis)
  const older = summary === null ? 0 : pick(summary.strata.find((layer) => layer.stratum === 'older')?.size ?? { logical: 0, allocated: 0 }, basis)
  const compresses = summary === null ? 0 : summary.fossils.reduce((sum, fossil) => sum + (fossil.compresses === null ? 0 : pick(fossil.compresses, basis)), 0)

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-6 py-5">
        <header className="min-w-0">
          <h2 className="truncate text-lg font-semibold" title={root}>
            The layers of {root}
          </h2>
          <p className="text-sm text-dim">
            How long ago what it holds was written, and what nothing has touched in a year · {basis === 'allocated' ? 'on disk' : 'by size'}
          </p>
        </header>

        {outcome !== null && <Alert tone={outcome.tone}>{outcome.text}</Alert>}

        {summary !== null && total > 0 && (
          <>
            <StatRow>
              <StatTile
                label="Untouched for a year"
                tone="accent"
                value={formatBytes(old)}
                delta={`${formatShare(old, total)} of the folder, in ${formatCount(summary.count)} ${summary.count === 1 ? 'fossil' : 'fossils'}`}
              />
              <StatTile label="Over five years" value={formatBytes(older)} delta={`${formatShare(older, total)} of the folder`} />
              <StatTile
                label="Compression works on"
                value={formatBytes(compresses)}
                delta={summary.compressRefused === null ? 'of the fossils listed; how much comes back, it measures' : 'nothing here: this volume does not compress'}
              />
            </StatRow>
            <Strata strata={summary.strata} basis={basis} total={total} />
          </>
        )}

        {summary !== null && total === 0 && <p className="text-sm text-dim">The folder is empty.</p>}

        {summary !== null && summary.compressRefused !== null && (
          <p className="text-xs text-dim">Nothing here can be compressed: {summary.compressRefused}.</p>
        )}

        {summary !== null && total > 0 && summary.count === 0 && (
          <p className="text-sm text-dim">Nothing here has gone a year without being written to: every layer is still in use.</p>
        )}

        {summary !== null && summary.fossils.length > 0 && (
          <section>
            <h3 className="mb-2 flex items-baseline gap-2 text-sm font-semibold">
              The largest fossils
              <span className="font-normal text-dim">· {formatCount(summary.count)}</span>
              <span className="tabular ml-auto font-medium text-accent">{formatBytes(old)}</span>
            </h3>
            <ul className="divide-y divide-line border-y border-line">
              {summary.fossils.map((fossil) => (
                <Fossil
                  key={fossil.id}
                  fossil={fossil}
                  basis={basis}
                  root={root}
                  accessKept={summary.accessKept}
                  busy={running || compressing !== null}
                  onOpen={onOpen}
                  onCompress={() => setAsked(fossil)}
                />
              ))}
              {summary.more > 0 && (
                <li className="flex items-center gap-3 px-3 py-2 text-sm text-dim">
                  <span className="tabular w-20 shrink-0 text-right">{formatBytes(pick(summary.moreSize, basis))}</span>
                  <span>
                    {formatCount(summary.more)} smaller {summary.more === 1 ? 'fossil' : 'fossils'}
                  </span>
                </li>
              )}
            </ul>
            {/* What the dates are, said once rather than on every row. */}
            <p className="mt-2 text-xs text-dim">
              A layer is as old as the newest thing written in it.{' '}
              {summary.accessKept
                ? 'When a file in it was last opened is said beside it: an old folder opened lately is still somebody’s.'
                : 'This volume does not keep the time files were last opened, so only when they were written is said.'}
            </p>
          </section>
        )}
      </div>

      <CompressDialog asked={asked} basis={basis} running={running} compressing={compressing} onRun={run} onClose={() => setAsked(null)} />
    </div>
  )
}

interface StrataProps {
  strata: Layer[]
  basis: SizeBasis
  /** What the folder holds: what the bands add up to. */
  total: number
}

/**
 * The bar of the strata: one band per age, the youngest at the left, each as
 * wide as what its files take, in the colours of the picture of what is
 * untouched - the same tokens, so the bar and the picture read alike. Under
 * it the bands with anything in them, by name and size.
 */
function Strata({ strata, basis, total }: StrataProps) {
  const shown = strata.filter((layer) => pick(layer.size, basis) > 0)
  return (
    <section aria-label="The strata">
      <span className="flex h-3 w-full overflow-hidden rounded-full border border-line" role="img" aria-label={shown.map((layer) => `${formatShare(pick(layer.size, basis), total)} ${STRATUM_WORDS[layer.stratum]}`).join(', ')}>
        {shown.map((layer) => (
          <span
            key={layer.stratum}
            className="block h-full"
            style={{ width: `${(pick(layer.size, basis) / total) * 100}%`, background: `var(--stratum-${layer.stratum})` }}
            title={`${describeStratum(layer.stratum)}: ${formatBytes(pick(layer.size, basis))}, ${formatCount(layer.files)} ${layer.files === 1 ? 'file' : 'files'}`}
          />
        ))}
      </span>
      <ul className="mt-2 flex flex-wrap gap-x-5 gap-y-1 text-xs text-dim">
        {shown.map((layer) => (
          <li key={layer.stratum} className="flex items-center gap-1.5">
            <span aria-hidden className="size-2.5 rounded-xs border border-line" style={{ background: `var(--stratum-${layer.stratum})` }} />
            <span className={isFossil(layer.stratum) ? 'text-text' : undefined}>{describeStratum(layer.stratum)}</span>
            <span className="tabular">{formatBytes(pick(layer.size, basis))}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}

interface FossilProps {
  fossil: FossilRow
  basis: SizeBasis
  root: string
  accessKept: boolean
  /** A compression is running, here or anywhere. */
  busy: boolean
  onOpen: (id: number) => void
  onCompress: () => void
}

/** One fossil: what it takes, where it is, when it was written and opened,
 * and whether it can be compressed. */
function Fossil({ fossil, basis, root, accessKept, busy, onOpen, onCompress }: FossilProps) {
  const written = describeLastWritten(fossil.written)
  const opened = accessKept ? describeLastOpened(fossil.opened) : null
  const why = whyNot(fossil.notCompressed)
  return (
    <li className="grid grid-cols-[5rem_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 px-3 py-2.5">
      <span className="tabular pt-px text-right text-sm font-medium">{formatBytes(pick(fossil.size, basis))}</span>
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <Button
            variant="link"
            className="block min-w-0 truncate text-left font-mono text-xs text-text"
            title={`${fossil.path}\nOpen it in the tree`}
            onClick={() => onOpen(fossil.id)}
          >
            {relativeTo(root, fossil.path)}
            {fossil.isDirectory && '/'}
          </Button>
          {/* A tool makes it again: deleting it is the better answer, and the
              page of what can go says how. Said in words, not only colour. */}
          {fossil.finding !== null && (
            <Badge variant="soft" title="A rule knows what this is: see Safe to delete">
              {fossil.finding}
            </Badge>
          )}
          {fossil.notCompressed?.why === 'nothing' && <Badge variant="soft">compressed</Badge>}
        </div>
        <p className="mt-0.5 text-xs text-dim">
          {[written, opened, fossil.isDirectory ? `${formatCount(fossil.entries - 1)} entries` : null]
            .filter((part) => part !== null)
            .join(' · ')}
        </p>
      </div>
      {fossil.compresses !== null ? (
        <Button
          variant="soft"
          size="sm"
          disabled={busy}
          onClick={onCompress}
          title={`NTFS compression in place: the files stay where they are and open as before. It works on ${formatBytes(pick(fossil.compresses, basis))} of it.`}
        >
          Compress
        </Button>
      ) : (
        <span className="pt-1 text-right text-xs text-faint" title={why ?? undefined}>
          {why === null ? '' : fossil.notCompressed?.why === 'windows' ? 'Windows’ own' : fossil.notCompressed?.why === 'nothing' ? '' : 'not here'}
        </span>
      )}
    </li>
  )
}

/** Why a fossil is not offered for compression, in a sentence. */
function whyNot(not: NotCompressed | null): string | null {
  if (not === null) return null
  switch (not.why) {
    case 'volume':
      return `Not compressed: ${not.detail}.`
    case 'windows':
      return 'Windows’ own files are left as Windows keeps them: a boot driver compressed is a machine that does not start. Windows compresses itself with compact /compactos.'
    case 'nothing':
      return 'Nothing in it would compress: it is compressed already, or made of formats that are.'
  }
}

interface CompressDialogProps {
  asked: FossilRow | null
  basis: SizeBasis
  running: boolean
  compressing: CompressingRow | null
  onRun: () => void
  onClose: () => void
}

/**
 * The question before a fossil is compressed: what compression does, what
 * it works on, what it leaves as it is, and how to undo it - and, once it
 * runs, how far it is, with a way to stop between two files.
 */
function CompressDialog({ asked, basis, running, compressing, onRun, onClose }: CompressDialogProps) {
  const reads = asked?.compresses === null || asked === null ? 0 : pick(asked.compresses, basis)
  return (
    <ConfirmDialog open={asked !== null} onOpenChange={(next) => !next && !running && onClose()}>
      <ConfirmDialogPopup size="lg">
        <ConfirmDialogHeader>
          <ConfirmDialogTitle>Compress {asked === null ? '' : asked.path.split(/[\\/]/).at(-1)}?</ConfirmDialogTitle>
          <ConfirmDialogDescription>
            NTFS compresses the files where they are: nothing is deleted or moved, they open as before - a little slower to read - and Explorer can undo it
            in the folder’s properties. It works on {formatBytes(reads)} of it; how much comes back is the files’ to say, and midda measures it afterwards.
          </ConfirmDialogDescription>
        </ConfirmDialogHeader>
        <ConfirmDialogBody>
          {running && compressing !== null ? (
            <div className="flex flex-col gap-2">
              <Progress value={compressing.of > 0 ? Math.min(compressing.bytes, compressing.of) : null} max={Math.max(compressing.of, 1)} label="Compressing" />
              <p className="tabular text-xs text-dim">
                {formatCount(compressing.files)} {compressing.files === 1 ? 'file' : 'files'} · {formatBytes(compressing.bytes)} of {formatBytes(compressing.of)}
              </p>
            </div>
          ) : running ? (
            <Progress label="Compressing" />
          ) : (
            <p className="text-sm text-dim">
              Left as they are: what is compressed already, photos, video and archives, which compress by their format, virtual disks, which WSL and Hyper-V
              refuse compressed, and files in the cloud.
            </p>
          )}
        </ConfirmDialogBody>
        <ConfirmDialogActions>
          {running ? (
            <Button variant="ghost" onClick={() => void cancelCompress()}>
              Stop
            </Button>
          ) : (
            <Button variant="ghost" render={<ConfirmDialogClose />}>
              Cancel
            </Button>
          )}
          <Button variant="primary" onClick={onRun} disabled={running}>
            {running ? 'Compressing…' : 'Compress'}
          </Button>
        </ConfirmDialogActions>
      </ConfirmDialogPopup>
    </ConfirmDialog>
  )
}
