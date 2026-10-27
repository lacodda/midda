import { useCallback, useEffect, useState } from 'react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { disks, dockerUsage, isStale, type DiskRow, type Disks, type DockerRow, type PruneRow } from '@/core'
import { OWNER_WORDS, describeDiskFile, formatBytes, formatCount, relativeTo } from '@/format'

interface VirtualDisksProps {
  arena: number
  /** Moves whenever the folder's tree changed: the disks are asked again. */
  version: number
  /** The folder opened, as the page's title names it. */
  root: string
  /** A disk was chosen: the window shows its file in the tree. */
  onOpen: (id: number) => void
}

/**
 * The virtual disks in the folder - WSL's, Docker's, a Hyper-V machine's -
 * each read inside: what it holds, what compacting it gives back, and the
 * commands that compact it (ADR 0015). Nothing here is deleted or run: a disk
 * is compacted with its machine stopped and with an administrator's rights,
 * so the page says how, and copies it.
 *
 * Every figure is on disk, whatever size the page above shows: what
 * compacting gives back is room in the file, which is clusters.
 */
export function VirtualDisks({ arena, version, root, onOpen }: VirtualDisksProps) {
  const [summary, setSummary] = useState<Disks | null>(null)
  const [failure, setFailure] = useState<string | null>(null)

  useEffect(() => {
    let current = true
    disks(arena)
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
  }, [arena, version])

  if (failure !== null) return <p className="text-sm text-bad">{failure}</p>
  if (summary === null || summary.disks.length === 0) return null

  const count = summary.disks.length
  return (
    <section aria-label="Virtual disks">
      <h3 className="flex items-baseline gap-2 text-sm font-semibold">
        Virtual disks
        <span className="font-normal text-dim">· {formatCount(count)}</span>
        {summary.givesBack > 0 && (
          <span className="tabular ml-auto font-medium text-accent" title="What compacting gives back, on disk">
            {formatBytes(summary.givesBack)}
          </span>
        )}
      </h3>
      <p className="mb-2 text-xs text-dim">
        Each a whole disk in one file, which grows as the machine inside writes and does not shrink when it deletes. The figure is what compacting gives back,
        on disk; the header adds up the disks worth compacting.
      </p>
      <ul className="divide-y divide-line border-y border-line">
        {summary.disks.map((disk) => (
          <Disk key={disk.id} disk={disk} root={root} onOpen={onOpen} />
        ))}
      </ul>
      {summary.disks.some((disk) => disk.owner === 'docker') && <Docker />}
    </section>
  )
}

/** Copies `text`, and says so for a moment. */
function useCopy(): [string | null, (text: string) => void] {
  const [copied, setCopied] = useState<string | null>(null)
  const copy = useCallback((text: string) => {
    navigator.clipboard
      .writeText(text)
      .then(() => setCopied('Copied'))
      .catch(() => setCopied('Could not copy'))
  }, [])
  useEffect(() => {
    if (copied === null) return
    const timer = window.setTimeout(() => setCopied(null), 2_000)
    return () => window.clearTimeout(timer)
  }, [copied])
  return [copied, copy]
}

interface DiskProps {
  disk: DiskRow
  root: string
  onOpen: (id: number) => void
}

/** One disk: what compacting gives back, what it is, what is inside, how. */
function Disk({ disk, root, onOpen }: DiskProps) {
  const [shown, setShown] = useState(false)
  const [copied, copy] = useCopy()
  return (
    <li className="grid grid-cols-[5rem_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 px-3 py-2.5">
      {/* A figure the line calls next to nothing is said quietly: the
          header adds up only what is worth compacting for. */}
      <span className={`tabular pt-px text-right text-sm ${disk.worth ? 'font-medium' : 'text-dim'}`} title="What compacting gives back, on disk">
        {disk.givesBack === null ? '-' : formatBytes(disk.givesBack)}
      </span>
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm font-medium">{disk.name}</span>
          {disk.owner !== null && <Badge variant="info">{OWNER_WORDS[disk.owner]}</Badge>}
        </div>
        <p className="mt-0.5 flex min-w-0 items-baseline gap-1 text-xs text-dim">
          <Button
            variant="link"
            className="block min-w-0 truncate text-left font-mono text-xs text-text"
            title={`${disk.path}\nOpen it in the tree`}
            onClick={() => onOpen(disk.id)}
          >
            {relativeTo(root, disk.path)}
          </Button>
          <span className="shrink-0">· {describeDiskFile(disk.file.allocated, disk.format)}</span>
        </p>
        <p className="mt-1 text-xs text-text">{disk.inside}</p>
        {disk.explain !== '' && <p className="mt-0.5 text-xs text-dim">{disk.explain}</p>}
        {disk.script !== null && shown && (
          <div className="mt-2 flex flex-col gap-1">
            <p className="text-xs text-dim">
              {disk.first === null ? '' : `${disk.first} `}Then, in PowerShell run as administrator:
            </p>
            <pre className="overflow-x-auto rounded-md bg-soft px-3 py-2 font-mono text-xs text-text">{disk.script}</pre>
          </div>
        )}
        {disk.script !== null && (
          <Button variant="link" className="mt-1 text-xs" aria-expanded={shown} onClick={() => setShown((was) => !was)}>
            {shown ? 'Hide how to compact it' : 'How to compact it'}
          </Button>
        )}
      </div>
      {disk.script !== null ? (
        <span className="flex flex-col items-end gap-1">
          <Button variant="soft" size="sm" onClick={() => copy(disk.script ?? '')} title="Copy the commands that compact it, for PowerShell run as administrator">
            Copy commands
          </Button>
          {copied !== null && (
            <span aria-live="polite" className="text-xs text-dim">
              {copied}
            </span>
          )}
        </span>
      ) : (
        <span />
      )}
    </li>
  )
}

/**
 * What Docker keeps in its disk that nothing uses, as its engine says - asked
 * once when the page opens and again on request: Docker's images do not move
 * with the folder's tree, and its engine takes a moment to measure volumes.
 */
function Docker() {
  const [usage, setUsage] = useState<DockerRow | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [asking, setAsking] = useState(true)
  /** How many times the reader asked again: each is a new question. */
  const [asked, setAsked] = useState(0)

  useEffect(() => {
    let current = true
    dockerUsage()
      .then((next) => {
        if (current) {
          setUsage(next)
          setFailure(null)
        }
      })
      .catch((cause: unknown) => {
        if (current) setFailure(String(cause))
      })
      .finally(() => {
        if (current) setAsking(false)
      })
    return () => {
      current = false
    }
  }, [asked])

  const ask = useCallback(() => {
    setAsking(true)
    setAsked((times) => times + 1)
  }, [])

  const total = usage?.prunes.reduce((sum, prune) => sum + prune.bytes, 0) ?? 0
  return (
    <div className="mt-4">
      <h4 className="flex items-baseline gap-2 text-sm font-semibold">
        Inside Docker's disk
        {usage !== null && <span className="font-normal text-dim">· {formatBytes(usage.size)} as Docker counts it</span>}
        {total > 0 && <span className="tabular ml-auto font-medium text-accent">{formatBytes(total)}</span>}
      </h4>
      <p className="mb-2 flex items-baseline gap-2 text-xs text-dim">
        <span>
          {asking
            ? 'Asking Docker what it keeps…'
            : failure !== null
              ? `Docker did not say: ${failure}.`
              : usage !== null && usage.prunes.length === 0
                ? 'Docker uses everything it keeps.'
                : 'What Docker keeps that nothing uses, as Docker reckons it.'}
        </span>
        {!asking && (
          <Button variant="link" className="text-xs" onClick={ask}>
            Ask again
          </Button>
        )}
      </p>
      {usage !== null && usage.prunes.length > 0 && (
        <>
          <ul className="divide-y divide-line border-y border-line">
            {usage.prunes.map((prune) => (
              <Prune key={prune.what} prune={prune} />
            ))}
          </ul>
          <p className="mt-2 text-xs text-dim">{usage.after}</p>
        </>
      )}
    </div>
  )
}

/** One kind of thing nothing uses: how much, how careful, and the command. */
function Prune({ prune }: { prune: PruneRow }) {
  const [copied, copy] = useCopy()
  return (
    <li className="grid grid-cols-[5rem_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 px-3 py-2.5">
      <span className="tabular pt-px text-right text-sm font-medium">{formatBytes(prune.bytes)}</span>
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <span className="text-sm">
            {prune.what} <span className="text-dim">· {formatCount(prune.count)}</span>
          </span>
          {/* Said in words as well as in colour, as a finding's is. */}
          <Badge variant={prune.risk === 'safe' ? 'good' : 'warn'}>{prune.risk === 'safe' ? 'Safe' : 'Caution'}</Badge>
        </div>
        <p className="mt-1 text-xs text-dim">{prune.explain}</p>
        <p className="mt-0.5 text-xs text-dim">
          <code className="font-mono text-text">{prune.command}</code>
        </p>
      </div>
      <span className="flex flex-col items-end gap-1">
        <Button variant="ghost" size="sm" onClick={() => copy(prune.command)} title={`Copy ${prune.command}`}>
          Copy
        </Button>
        {copied !== null && (
          <span aria-live="polite" className="text-xs text-dim">
            {copied}
          </span>
        )}
      </span>
    </li>
  )
}
