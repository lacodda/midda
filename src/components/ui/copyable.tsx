import { useCallback, useEffect, useId, useRef, useState, type HTMLAttributes } from 'react'
import { cn } from 'dowel-ui'

/*
 * Copyable.
 *
 * Any text that someone will eventually want to copy - an id, a path, a hash,
 * a token - copied with one click. The rule comes from nitid: if a value is
 * worth showing, it is worth being able to take away, and selecting a
 * monospaced id by hand is a small daily tax.
 *
 * Two things this gets right that the hand-written version usually does not.
 * It is a `<button>`, so the keyboard can reach it and a screen reader says
 * what it does. And the confirmation is announced, not only drawn: a tick that
 * appears silently tells a sighted user it worked and tells nobody else.
 *
 * It is named by its label and then by the value it shows - "Copy a1b2c3".
 * The label alone was a name without the words drawn on the button, and
 * someone telling voice control what they see says those words (WCAG 2.5.3).
 *
 * The clipboard can refuse - it needs a secure context and, in some browsers,
 * a permission. A refusal is reported rather than swallowed, because a button
 * that looks like it worked and did not is worse than one that says it failed.
 */
export interface CopyableProps extends Omit<HTMLAttributes<HTMLButtonElement>, 'onCopy'> {
  /** What lands on the clipboard. Defaults to the visible text. */
  value?: string
  /** The visible text. */
  children: string
  /** What the button is called, for a screen reader. Required, and
   * deliberately without a default: a string the component invents is a
   * string the product cannot translate, and it would ship in English to
   * every reader who does not read English. */
  label: string
  /** What is announced after a successful copy. Required for the same
   * reason. */
  copiedLabel: string
  /** Told what happened, for a product that wants its own toast. */
  onCopy?: (ok: boolean) => void
}

export function Copyable({
  value,
  children,
  label,
  copiedLabel,
  onCopy,
  id,
  className,
  ...props
}: CopyableProps) {
  const [copied, setCopied] = useState(false)
  // An element listed in its own `aria-labelledby` is read by its
  // `aria-label`: the label first, then the value.
  const ownId = useId()
  const buttonId = id ?? ownId
  const valueId = useId()
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)

  // A component that sets state on a timer has to stop when it goes away, or
  // it wakes up in a tree that no longer exists.
  useEffect(() => () => clearTimeout(timer.current), [])

  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(value ?? children)
      setCopied(true)
      onCopy?.(true)
      clearTimeout(timer.current)
      timer.current = setTimeout(() => setCopied(false), 1600)
    } catch {
      // No secure context, or permission refused. Say so rather than pretend.
      onCopy?.(false)
    }
  }, [value, children, onCopy])

  return (
    <button
      type="button"
      onClick={copy}
      id={buttonId}
      aria-label={copied ? copiedLabel : label}
      aria-labelledby={`${buttonId} ${valueId}`}
      className={cn(
        'group inline-flex max-w-full items-center gap-1.5 rounded-sm px-1 py-0.5 text-left target-min',
        'font-mono text-xs text-dim transition-colors',
        'hover:bg-soft hover:text-text',
        'focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent',
        className,
      )}
      {...props}
    >
      <span id={valueId} className="truncate">
        {children}
      </span>

      <span className={cn('shrink-0 transition-colors', copied ? 'text-good' : 'text-faint')} aria-hidden>
        {copied ? <Tick /> : <Clipboard />}
      </span>

      {/* Drawn confirmation is invisible to a screen reader; this is the part
          that actually says it worked. */}
      <span role="status" aria-live="polite" className="sr-only">
        {copied ? copiedLabel : ''}
      </span>
    </button>
  )
}

function Clipboard() {
  return (
    <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden>
      <rect x="5.5" y="2.5" width="8" height="10" rx="1.5" />
      <path d="M10.5 2.5v-.5a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h.5" />
    </svg>
  )
}

function Tick() {
  return (
    <svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
      <path d="M3 8.5l3.5 3.5L13 5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}
