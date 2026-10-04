import type { HTMLAttributes, ReactNode } from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from 'dowel-ui'

/*
 * StatusDot and StatusBadge.
 *
 * The smallest thing a screen can say about something's condition: a server is
 * up, a job failed, a person is away. Every product of the line drew its own
 * coloured circle, and every one of them drew it the same way - a `<span>` with
 * a background - which means the condition existed for exactly the readers who
 * could see it.
 *
 * The rule this pair is built on, and the reason a dot is not a component you
 * can call with a colour alone:
 *
 *   **Colour is emphasis. The word is the message.**
 *
 * That is not a preference. About one man in twelve does not separate red from
 * green, `--good` and `--bad` are the two hues most screens rest on, and a
 * printed or projected screen loses the distinction for everybody. The theme
 * already states the rule in prose; here it is structure: `StatusDot` will not
 * render without a label, and a label that is not shown is placed where a
 * screen reader will still read it.
 *
 * `StatusBadge` is the same fact when there is room to write it out. It exists
 * separately rather than as a variant because the choice between them is about
 * the space on the screen, not about the state - and a product that has the
 * room should be nudged to use the words.
 *
 * What neither does is carry a word of its own. "Online" is the product's
 * vocabulary in the product's language, and a primitive that shipped English
 * strings could not be translated.
 */

/** The conditions the line's vocabulary can name.
 *
 * `neutral` is not a fifth colour - it is the absence of a judgement, for the
 * states that are neither good nor bad: queued, archived, unknown. Drawing
 * those in one of the four would be a claim the product did not make. */
export type Status = 'good' | 'warn' | 'bad' | 'info' | 'neutral'

export const statusDotVariants = cva('inline-block shrink-0 rounded-full', {
  variants: {
    status: {
      good: 'bg-good',
      warn: 'bg-warn',
      bad: 'bg-bad',
      info: 'bg-info',
      neutral: 'bg-line-2',
    },
    size: {
      /* Beside `text-xs`, where a dot much larger than the x-height reads as a
       * bullet rather than as a state. */
      sm: 'size-1.5',
      md: 'size-2',
      lg: 'size-2.5',
    },
  },
  defaultVariants: { status: 'neutral', size: 'md' },
})

/** The status as ink rather than as a fill - what an icon standing in for the
 * dot is drawn in. Separate from the fill variants because the two are used in
 * different places and a single `cva` would have to undo one to get the
 * other. */
const statusInk: Record<Status, string> = {
  good: 'text-good',
  warn: 'text-warn',
  bad: 'text-bad',
  info: 'text-info',
  neutral: 'text-dim',
}

export interface StatusDotProps
  extends Omit<HTMLAttributes<HTMLSpanElement>, 'children'>,
    VariantProps<typeof statusDotVariants> {
  /**
   * What this condition is called, in the product's words. Required, and not
   * for decoration: it is what the dot means to anyone who does not see the
   * colour.
   */
  label: string
  /**
   * Whether to print the label beside the dot.
   *
   * `false` keeps it for a screen reader only - the right answer inside a
   * dense table cell, where the column heading already says what is being
   * judged. It is never dropped.
   */
  showLabel?: boolean
  /**
   * A shape drawn inside the dot's place instead of the circle - a tick, a
   * cross, an exclamation. The second channel for a reader who sees the dot
   * but not its hue, and the reason `pulse` is not the only way to differ.
   */
  icon?: ReactNode
}

/**
 * A dot, and the word it stands for.
 *
 * The word is always in the accessibility tree. When it is not printed the dot
 * is not `aria-hidden` and not `role="presentation"` - it is a labelled image,
 * because a status nobody can read is the defect this component exists to
 * prevent.
 */
export function StatusDot({
  status,
  size,
  label,
  showLabel = false,
  icon,
  className,
  ...props
}: StatusDotProps) {
  const mark = icon ? (
    // The icon stands where the dot would, so it takes the status as its ink
    // rather than as a fill.
    <span
      className={cn('inline-flex shrink-0 items-center justify-center', statusInk[status ?? 'neutral'])}
      aria-hidden
    >
      {icon}
    </span>
  ) : (
    <span className={statusDotVariants({ status, size })} aria-hidden />
  )

  return (
    <span
      className={cn('inline-flex items-center gap-1.5 whitespace-nowrap', className)}
      // Announced as one thing. Without this the dot and the word are two
      // nodes and a reader hears the label twice - once from the image, once
      // from the text beside it.
      role="img"
      aria-label={label}
      {...props}
    >
      {mark}
      {showLabel && (
        <span className="text-xs text-dim" aria-hidden>
          {label}
        </span>
      )}
    </span>
  )
}

export const statusBadgeVariants = cva(
  'inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium whitespace-nowrap',
  {
    variants: {
      status: {
        good: 'bg-good-soft text-good',
        warn: 'bg-warn-soft text-warn',
        bad: 'bg-bad-soft text-bad',
        info: 'bg-info-soft text-info',
        neutral: 'bg-soft text-dim',
      },
    },
    defaultVariants: { status: 'neutral' },
  },
)

export interface StatusBadgeProps
  extends HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof statusBadgeVariants> {
  /** A shape before the word - the second channel, where there is one. */
  icon?: ReactNode
  /** Draw the dot before the word. Off when an `icon` is given: two marks for
   * one state is noise. */
  dot?: boolean
}

/**
 * The state, written out, on a tinted ground.
 *
 * Distinct from `Badge`, which is a label of any kind: this one is about a
 * condition, takes the same `Status` vocabulary as the dot, and is what a
 * product should reach for when the answer is "failed" rather than "3".
 *
 * No `role="img"` here and no `aria-label`: the word is on the screen, and
 * naming the element as an image would replace the text a reader can already
 * hear with a duplicate of it.
 */
export function StatusBadge({
  status,
  icon,
  dot = true,
  className,
  children,
  ...props
}: StatusBadgeProps) {
  const showDot = dot && !icon
  return (
    <span className={cn(statusBadgeVariants({ status }), className)} {...props}>
      {icon && (
        <span className="inline-flex shrink-0 items-center justify-center" aria-hidden>
          {icon}
        </span>
      )}
      {showDot && (
        <span
          className={cn(
            'inline-block size-1.5 shrink-0 rounded-full',
            // `currentColor` rather than the status fill: inside the badge the
            // text already carries the hue, and a second token would drift
            // from it the first time either changed.
            'bg-current',
          )}
          aria-hidden
        />
      )}
      {children}
    </span>
  )
}
