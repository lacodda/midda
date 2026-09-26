import type { ReactNode } from 'react'
import { Radio } from '@base-ui/react/radio'
import { RadioGroup } from '@base-ui/react/radio-group'
import { cn } from 'dowel-ui'

/*
 * SegmentedControl.
 *
 * Two to five options, one of them chosen, all of them in view - a radio group
 * in a segment's clothes.
 *
 * The theme picker, a list shown as rows or cards, "all / mine / shared". The
 * shape settles three things a Select cannot: every option is on screen, the
 * choice is one press rather than two, and switching back and forth is the
 * same press again. kilna put a Select where a segment belongs - the theme in
 * its settings - and drew the segment by hand where it had one, which is the
 * gap this closes.
 *
 * **It is a radio group, not a row of toggles.** Exactly one option is ever
 * chosen, which is what a radio says and a toggle does not: a toggle group
 * lets the last pressed button go, and "no theme" is not a theme. Base UI's
 * RadioGroup underneath gives the rest - one stop on the Tab key, the arrows
 * move and choose at once, and a reader hears "Dark, radio button, 2 of 3" -
 * so the segment is only the clothes.
 *
 * The clothes are the mockup's `.seg`: a sunken track, and the chosen segment
 * lifted out of it on the raised surface. Lifted rather than filled with the
 * accent, because a segment is a setting and not an action - an accent fill
 * reads as "press me", which the chosen option is the one thing you need not.
 *
 * Its height is `h-control-sm`, so the control sits level with the small
 * buttons and fields beside it and follows `data-density` with them.
 */

export interface SegmentedControlProps {
  /** What is being chosen: "Theme", "View". Required - a row of words with
   * no name for the question is a riddle to anyone who cannot see the
   * heading above it. */
  'aria-label'?: string
  'aria-labelledby'?: string
  value?: string
  defaultValue?: string
  onValueChange?: (value: string) => void
  disabled?: boolean
  /** The field name, for a form that posts it. */
  name?: string
  className?: string
  children: ReactNode
}

export function SegmentedControl({ onValueChange, className, children, ...props }: SegmentedControlProps) {
  return (
    <RadioGroup
      onValueChange={onValueChange ? (next) => onValueChange(next as string) : undefined}
      // `w-fit`: in a column - a FieldGroup, a form - a flex child is stretched
      // to the column's width, and the track ran on past its last segment.
      className={cn('inline-flex h-control-sm w-fit shrink-0 items-stretch gap-0.5 rounded-md bg-soft p-0.5', className)}
      {...props}
    >
      {children}
    </RadioGroup>
  )
}

export interface SegmentProps {
  value: string
  /** The option's words. An icon alone needs `aria-label` beside it. */
  children: ReactNode
  'aria-label'?: string
  disabled?: boolean
  className?: string
}

/** One option of a SegmentedControl. */
export function Segment({ value, children, className, ...props }: SegmentProps) {
  return (
    <Radio.Root
      value={value}
      className={cn(
        'inline-flex cursor-pointer items-center justify-center gap-1.5 rounded-sm px-3 text-sm whitespace-nowrap text-dim',
        'transition-colors hover:text-text',
        '[&_svg]:shrink-0 [&_svg:not([class*=size-])]:size-3.5',
        // Chosen: lifted out of the track. The weight carries it too, so the
        // choice survives a screen with no shadows and a reader who does not
        // see the difference between two greys.
        'data-[checked]:bg-raise data-[checked]:font-semibold data-[checked]:text-text data-[checked]:shadow-lift',
        'focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent',
        'data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50',
        className,
      )}
      {...props}
    >
      {children}
    </Radio.Root>
  )
}
