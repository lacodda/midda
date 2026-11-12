import { open, save } from '@tauri-apps/plugin-dialog'
import { useCallback, useEffect, useState } from 'react'

import { Alert } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
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
import {
  addRulesFile,
  letGo,
  openRulesFolder,
  ownRules,
  readRulesAgain,
  removeRulesFile,
  saveRulesFile,
  type OwnRules as Own,
  type RulesFileRow,
} from '@/core'
import { describeRulesFile, formatCount } from '@/format'

interface OwnRulesProps {
  /** Moves whenever what is on screen changed - a keep among it. */
  version: number
  /** Something here changed what is listed: what the page said of it before
   * is out of date. */
  onChanged: () => void
}

/**
 * What the reader keeps, and their own rules (ADR 0019).
 *
 * A keep says "this one is mine": nothing it names is a finding again, nor
 * anything in it. Keep on a finding writes one into the window's own file;
 * Stop keeping lets it go. Beside them, the reader's rules files - written as
 * midda's own are, tried before them - with how each read: one that does not
 * read is listed with why, and costs only itself. A file can be added from
 * anywhere (it is read with the rest first), a copy saved to hand on, and one
 * removed to the Recycle Bin.
 */
export function OwnRules({ version, onChanged }: OwnRulesProps) {
  const [own, setOwn] = useState<Own | null>(null)
  const [said, setSaid] = useState<{ tone: 'good' | 'warn'; text: string } | null>(null)
  const [removing, setRemoving] = useState<RulesFileRow | null>(null)

  useEffect(() => {
    let current = true
    ownRules()
      .then((next) => current && setOwn(next))
      .catch((cause: unknown) => current && setSaid({ tone: 'warn', text: String(cause) }))
    return () => {
      current = false
    }
  }, [version])

  const act = useCallback((doing: Promise<Own>, done: (next: Own) => string | null) => {
    doing
      .then((next) => {
        setOwn(next)
        onChanged()
        const text = done(next)
        setSaid(text === null ? null : { tone: 'good', text })
      })
      .catch((cause: unknown) => setSaid({ tone: 'warn', text: String(cause) }))
  }, [onChanged])

  const add = useCallback(async () => {
    const path = await open({ filters: [{ name: 'Rules', extensions: ['toml'] }] })
    if (typeof path !== 'string') return
    act(addRulesFile(path), () => `${path.split(/[\\/]/).at(-1) ?? path} was added; its rules are tried before midda's.`)
  }, [act])

  const copy = useCallback(async (name: string) => {
    const to = await save({ defaultPath: name, filters: [{ name: 'Rules', extensions: ['toml'] }] })
    if (typeof to !== 'string') return
    saveRulesFile(name, to)
      .then(() => setSaid({ tone: 'good', text: `A copy of ${name} was saved to ${to}.` }))
      .catch((cause: unknown) => setSaid({ tone: 'warn', text: String(cause) }))
  }, [])

  if (own === null) return null
  const keeps = own.keeps
  return (
    <>
      {keeps.length > 0 && (
        <section aria-label="Kept">
          <h3 className="mb-2 flex items-baseline gap-2 text-sm font-semibold">
            Kept
            <span className="font-normal text-dim">· {formatCount(keeps.length)}, never listed here, nor anything in them</span>
          </h3>
          <ul className="divide-y divide-line border-y border-line">
            {keeps.map((keep) => (
              <li key={`${keep.file}-${keep.paths.join('|')}`} className="flex items-start gap-3 px-3 py-2">
                <div className="min-w-0 flex-1">
                  {keep.paths.map((path) => (
                    <p key={path} className="truncate font-mono text-xs text-text" title={path}>
                      {path}
                    </p>
                  ))}
                  <p className="mt-0.5 text-xs text-dim">{keep.why === null ? `In ${keep.file}` : `${keep.why} · in ${keep.file}`}</p>
                </div>
                {keep.mine ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => act(keep.paths.reduce<Promise<Own>>((done, path) => done.then(() => letGo(path)), Promise.resolve(own)), () => `${keep.paths.join(', ')} is no longer kept.`)}
                  >
                    Stop keeping
                  </Button>
                ) : (
                  <span className="shrink-0 text-xs text-dim" title="A keep in a file of your own: edit the file to let it go">
                    yours
                  </span>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      <section aria-label="Your rules">
        <div className="mb-2 flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <h3 className="text-sm font-semibold">Your rules</h3>
          <span className="min-w-0 flex-1 text-xs text-dim">
            Files written as midda's own, tried before them: what is yours to delete, what is never to go.
          </span>
          <div className="flex shrink-0 gap-2">
            <Button variant="ghost" size="sm" onClick={() => void add()}>
              Add a file…
            </Button>
            <Button variant="ghost" size="sm" onClick={() => act(readRulesAgain(), () => 'The rules folder was read again.')}>
              Read again
            </Button>
            <Button variant="ghost" size="sm" onClick={() => void openRulesFolder().catch((cause: unknown) => setSaid({ tone: 'warn', text: String(cause) }))}>
              Open the folder
            </Button>
          </div>
        </div>
        {said !== null && (
          <Alert tone={said.tone} className="mb-2">
            {said.text}
          </Alert>
        )}
        {own.files.length === 0 ? (
          <p className="text-xs text-dim">
            None yet. A file put in <span className="font-mono">{own.folder}</span> is read with midda's own; Keep on a finding writes one there.
          </p>
        ) : (
          <ul className="divide-y divide-line border-y border-line">
            {own.files.map((file) => (
              <li key={file.name} className="flex items-start gap-3 px-3 py-2">
                <div className="min-w-0 flex-1">
                  <p className="flex min-w-0 items-center gap-2">
                    <span className="truncate font-mono text-xs text-text" title={`${own.folder}\\${file.name}`}>
                      {file.name}
                    </span>
                    {file.refused !== null && <Badge variant="bad">Not read</Badge>}
                  </p>
                  <p className={`mt-0.5 text-xs ${file.refused === null ? 'text-dim' : 'whitespace-pre-line text-bad'}`}>{file.refused ?? describeRulesFile(file)}</p>
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button variant="ghost" size="sm" onClick={() => void copy(file.name)}>
                    Save a copy…
                  </Button>
                  <Button variant="danger" size="sm" onClick={() => setRemoving(file)}>
                    Remove…
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <ConfirmDialog open={removing !== null} onOpenChange={(next) => !next && setRemoving(null)}>
        <ConfirmDialogPopup>
          <ConfirmDialogHeader>
            <ConfirmDialogTitle>Remove {removing?.name}?</ConfirmDialogTitle>
            <ConfirmDialogDescription>
              {removing === null
                ? ''
                : removing.refused === null
                  ? `It goes to the Recycle Bin, and what it says stops applying: ${describeRulesFile(removing)}. Explorer restores it from the bin.`
                  : 'It goes to the Recycle Bin. It was not read, so nothing listed changes; Explorer restores it from the bin.'}
            </ConfirmDialogDescription>
          </ConfirmDialogHeader>
          <ConfirmDialogActions>
            <Button variant="ghost" render={<ConfirmDialogClose />}>
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                const file = removing
                setRemoving(null)
                if (file !== null) act(removeRulesFile(file.name), () => `${file.name} was moved to the Recycle Bin.`)
              }}
            >
              Move to the Recycle Bin
            </Button>
          </ConfirmDialogActions>
        </ConfirmDialogPopup>
      </ConfirmDialog>
    </>
  )
}
