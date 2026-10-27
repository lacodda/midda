import { useCallback, useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { RowButton } from '@/components/ui/list-row'
import { fills, isStale, pick, type Consumer, type FillShare, type Fills, type SizeBasis } from '@/core'
import { describeFillLine, formatBytes, formatCount, formatShare } from '@/format'

/** How many places each consumer lists before saying how many more. The
 * rule names about fifty at most; five is what a reader of a summary reads. */
const LINES = 5

interface WhatFillsProps {
  arena: number
  /** Moves whenever the folder's tree changed: the page is asked again. */
  version: number
  basis: SizeBasis
  /** The folder opened, as the title names it. */
  root: string
  /** A place was chosen: the window shows it in the tree. */
  onOpen: (id: number) => void
  /** The reader asked what of it can go. */
  onSafe: () => void
}

/**
 * What fills the folder, and who for: every byte given to one consumer -
 * Windows, programs, games, development, browsers, virtual disks, media,
 * documents, or what midda does not recognise - the most first, each with
 * the places that hold the most of it (ADR 0014).
 *
 * The paragraph at the top is the answer to "why is it full", said once in
 * the core so the window, and later the CLI and an assistant, say the same
 * sentence. It copies as Markdown with every consumer's places, to paste into
 * a chat as it is.
 */
export function WhatFills({ arena, version, basis, root, onOpen, onSafe }: WhatFillsProps) {
  const [summary, setSummary] = useState<Fills | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  /** The consumer listed whole, when the reader asked for one. */
  const [whole, setWhole] = useState<Consumer | null>(null)
  const [copied, setCopied] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    fills(arena, basis, LINES, whole)
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

  const copy = useCallback(() => {
    if (summary === null) return
    navigator.clipboard
      .writeText(summary.brief)
      .then(() => setCopied('Copied'))
      .catch(() => setCopied('Could not copy'))
  }, [summary])

  // The confirmation clears itself, as the report's does.
  useEffect(() => {
    if (copied === null) return
    const timer = window.setTimeout(() => setCopied(null), 2_000)
    return () => window.clearTimeout(timer)
  }, [copied])

  if (failure !== null) return <p className="p-4 text-sm text-bad">{failure}</p>

  const total = summary === null ? 0 : pick(summary.total, basis)

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-6 py-5">
        <header className="flex flex-wrap items-start gap-x-4 gap-y-2">
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-lg font-semibold" title={root}>
              What fills {root}
            </h2>
            <p className="text-sm text-dim">Everything in it, by what it is for · {basis === 'allocated' ? 'on disk' : 'by size'}</p>
          </div>
          <span className="flex items-center gap-2">
            {copied !== null && (
              <span aria-live="polite" className="text-xs text-dim">
                {copied}
              </span>
            )}
            <Button
              variant="ghost"
              size="sm"
              disabled={summary === null}
              onClick={copy}
              title="Copy the paragraph and every consumer's places as Markdown, to ask an assistant or keep in a note"
            >
              Copy as Markdown
            </Button>
          </span>
        </header>

        {summary !== null && (
          <section aria-label="Why it is full" className="flex flex-col gap-1">
            <p className="max-w-prose text-base leading-relaxed">{summary.story}</p>
            {/* How full a volume is, is a matter of clusters: the paragraph
                stays on disk when the figures below are switched to size. */}
            {basis === 'logical' && <p className="text-xs text-dim">The paragraph counts on disk; the figures below, by size.</p>}
          </section>
        )}

        {summary !== null && summary.shares.length === 0 && <p className="text-sm text-dim">The folder is empty.</p>}

        {summary !== null && summary.shares.length > 0 && (
          <ul className="flex flex-col gap-5">
            {summary.shares.map((share) => (
              <ShareOf
                key={share.consumer}
                share={share}
                basis={basis}
                total={total}
                root={root}
                onOpen={onOpen}
                onSafe={onSafe}
                onWhole={() => setWhole(share.consumer)}
              />
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}

interface ShareOfProps {
  share: FillShare
  basis: SizeBasis
  /** What the folder holds: what the bar is a share of. */
  total: number
  root: string
  onOpen: (id: number) => void
  onSafe: () => void
  onWhole: () => void
}

/** One consumer: how much and what share, what it is, and its places. */
function ShareOf({ share, basis, total, root, onOpen, onSafe, onWhole }: ShareOfProps) {
  const size = pick(share.size, basis)
  const frees = pick(share.frees, basis)
  return (
    <li>
      <div className="grid grid-cols-[6rem_minmax(0,1fr)] items-baseline gap-x-3">
        <span className="tabular text-right text-base font-semibold">{formatBytes(size)}</span>
        <div className="min-w-0">
          <h3 className="flex items-baseline gap-2 text-sm font-semibold">
            {share.label}
            <span className="tabular font-normal text-dim">{formatShare(size, total)}</span>
          </h3>
          {/* The share of the whole, one colour for every consumer: the
              label says which it is, the length how much of the folder. */}
          <span
            className="mt-1.5 block h-1 w-full overflow-hidden rounded-full bg-soft"
            role="img"
            aria-label={`${formatShare(size, total)} of the folder`}
            title={`${formatBytes(size)} of ${formatBytes(total)}`}
          >
            <span className="block h-full rounded-full bg-accent" style={{ width: total > 0 ? `${(size / total) * 100}%` : '0%' }} />
          </span>
          <p className="mt-1.5 text-xs text-dim">
            {share.about}
            {frees > 0 && (
              <>
                {' '}
                A tool makes <span className="tabular font-medium text-text">{formatBytes(frees)}</span> of it again.{' '}
                <Button variant="link" className="text-xs" onClick={onSafe}>
                  See what can go
                </Button>
              </>
            )}
            {/* What compacting gives back is room in a file: clusters, on
                disk whichever size the figures above are in. */}
            {share.compacts !== null && (
              <>
                {' '}
                Compacting gives back about <span className="tabular font-medium text-text">{formatBytes(share.compacts)}</span> on disk.{' '}
                <Button variant="link" className="text-xs" onClick={onSafe}>
                  See how
                </Button>
              </>
            )}
          </p>
        </div>
      </div>

      {share.lines.length > 0 && (
        <ul className="mt-2 ml-27 divide-y divide-line border-y border-line">
          {share.lines.map((line) => {
            const { title, path } = describeFillLine(line, root)
            return (
              <li key={`${line.loose ? 'files' : 'entry'}:${line.id}`}>
                <RowButton
                  onClick={() => onOpen(line.id)}
                  title={`${line.path}\nOpen it in the tree`}
                  start={<span className="tabular w-16 text-right font-medium text-text">{formatBytes(line.bytes)}</span>}
                  description={path === null ? undefined : <span className="font-mono">{path}</span>}
                >
                  <span className={path === null ? 'font-mono text-xs' : 'text-sm'}>{title}</span>
                </RowButton>
              </li>
            )
          })}
          {share.more > 0 && (
            <li className="flex items-center gap-3 px-3 py-2 text-sm text-dim">
              <span>
                {formatCount(share.more)} more {share.more === 1 ? 'place' : 'places'}
              </span>
              <Button variant="link" className="ml-auto" onClick={onWhole}>
                Show all
              </Button>
            </li>
          )}
          {/* What no place names, said rather than dropped: without it the
              places would read as all of it. */}
          {share.rest > 0 && (
            <li className="px-3 py-2 text-sm text-dim">
              <span className="tabular">{formatBytes(share.rest)}</span> more, in pieces under {formatBytes(share.threshold)} each
            </li>
          )}
        </ul>
      )}
    </li>
  )
}
