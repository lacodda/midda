import type { ReactNode } from 'react'
import { Checkbox as Base } from '@base-ui/react/checkbox'
import { CheckboxGroup as BaseGroup } from '@base-ui/react/checkbox-group'
import { cn } from 'dowel-ui'

/*
 * Checkbox - a box, its tick, and the words next to it.
 *
 * The interesting part is the words. A checkbox on its own is a nine-pixel
 * target that says nothing; wired to a label it is the whole row, and the row
 * is what a finger and a pointer both aim at. So the label is part of the
 * component rather than something a caller remembers to add - the commonest
 * bug in a hand-rolled checkbox is a `<label>` that is next to the input
 * instead of tied to it, which looks identical and does nothing.
 *
 * The box is drawn rather than native. `appearance: none` on a real
 * `<input type=checkbox>` is the other way, and it takes the indeterminate
 * state with it: the dash is not a character the input can be told to draw.
 * Base UI renders a button with the right role and state, and the tick and
 * the dash are ours, in the accent, sized to the text next to them.
 *
 * `indeterminate` is a real state, not a third value: it says "some of the
 * things below are checked", and clicking still means check-all. A tri-state
 * value would make every caller handle a case that does not exist.
 */

export interface CheckboxProps {
  /** The words next to the box. Omit only for a checkbox in a table cell,
   * and then give `aria-label` instead. */
  children?: ReactNode
  /** Controlled state. Pair it with `onCheckedChange`. */
  checked?: boolean
  /** Initial state when the checkbox is uncontrolled. */
  defaultChecked?: boolean
  /** Called with the new state when the box is toggled. */
  onCheckedChange?: (checked: boolean) => void
  /** Some but not all of what this stands for is checked. Draws a dash;
   * clicking still checks everything. */
  indeterminate?: boolean
  /** This is the box that stands for the whole group. Inside a
   * `CheckboxGroup` with `allValues`, its state follows the children - checked,
   * unchecked, or the dash in between - and clicking it sets all of them.
   * Working that out by hand is where the indeterminate state usually goes
   * wrong, so the group does it. */
  parent?: boolean
  /** Stops the box taking input; the label dims with it. */
  disabled?: boolean
  /** Marks the box as required for form validation. */
  required?: boolean
  /** The field name submitted with a form. */
  name?: string
  /** What this box contributes to the group's `value` array when checked, and
   * what a form submits for it. */
  value?: string
  /** The accessible name, for a checkbox with no `children`. */
  'aria-label'?: string
  className?: string
}

export function Checkbox({
  children,
  indeterminate = false,
  className,
  ...props
}: CheckboxProps) {
  const box = (
    <Base.Root
      indeterminate={indeterminate}
      className={cn(
        // `group` is what the two marks below hang their state off: the
        // indeterminate one belongs to the element, not to a prop.
        'group flex size-4 shrink-0 items-center justify-center rounded-xs border border-line-2 bg-transparent',
        'transition-colors',
        'hover:border-accent',
        'data-[checked]:border-accent data-[checked]:bg-accent',
        'data-[indeterminate]:border-accent data-[indeterminate]:bg-accent',
        'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
        'disabled:cursor-not-allowed disabled:opacity-50',
        // Without a label the box is the whole control, so the caller's
        // classes land on it.
        children === undefined && className,
      )}
      {...props}
    >
      {/* Both marks are rendered and the state picks one, rather than the
        * `indeterminate` prop picking it. A parent checkbox never receives
        * that prop - the group works its state out from the children and says
        * so on the element - so choosing in JavaScript would leave the parent
        * drawing a tick while announcing `mixed`. */}
      <Base.Indicator
        className="flex text-on-accent group-data-[indeterminate]:hidden"
        render={<span />}
      >
        <svg viewBox="0 0 16 16" className="size-3" aria-hidden>
          <path
            d="M3.5 8.5l3 3 6-6.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </Base.Indicator>
      <span className="hidden text-on-accent group-data-[indeterminate]:flex" aria-hidden>
        <svg viewBox="0 0 16 16" className="size-3">
          <path d="M4 8h8" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
        </svg>
      </span>
    </Base.Root>
  )

  if (children === undefined) return box

  return (
    <label
      className={cn(
        'flex cursor-pointer items-center gap-2 text-sm text-text',
        'has-[:disabled]:cursor-not-allowed has-[:disabled]:opacity-50',
        className,
      )}
    >
      {box}
      {children}
    </label>
  )
}

/*
 * A group of checkboxes that share a name and a value.
 *
 * It exists for the parent checkbox: given `allValues`, Base UI works out
 * whether the parent is checked, unchecked or indeterminate, and clicking it
 * sets all of them. Doing that by hand is where the indeterminate state
 * usually goes wrong.
 */
export interface CheckboxGroupProps {
  children: ReactNode
  /** The values of the checked boxes. Controlled; pair it with `onValueChange`. */
  value?: string[]
  /** The values checked at first, when the group is uncontrolled. */
  defaultValue?: string[]
  /** Called with the new list of checked values. */
  onValueChange?: (value: string[]) => void
  /** Every value in the group, which is what makes a parent checkbox work. */
  allValues?: string[]
  /** Disables every box in the group. */
  disabled?: boolean
  className?: string
}

export function CheckboxGroup({ children, className, ...props }: CheckboxGroupProps) {
  return (
    <BaseGroup className={cn('flex flex-col gap-2', className)} {...props}>
      {children}
    </BaseGroup>
  )
}
