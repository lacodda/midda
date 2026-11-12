import { Toolbar as Base } from '@base-ui/react/toolbar'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from 'dowel-ui'

/*
 * ActionBar.
 *
 * The strip of actions that belongs to what is on screen: Save and Cancel at
 * the foot of a form, the bulk actions above a table, the formatting buttons
 * over an editor. `sticky` is the point of it - a long form whose Save button
 * is a thousand pixels below the field being edited has a Save button the
 * reader has to go looking for.
 *
 * Built on Base UI's Toolbar, which supplies the part that is easy to get
 * wrong: `role="toolbar"` and arrow-key navigation, so the whole bar is one
 * tab stop and the arrows move between its buttons. That matters more here
 * than anywhere else - an action bar sits at the end of a form, and without it
 * a reader tabbing out of the last field lands in a queue of five buttons
 * instead of on Save.
 *
 * The buttons are the product's own: this draws the strip, not what is in it.
 * `ActionBarButton` exists only to join a `Button` to the toolbar's keyboard
 * handling, via `render`.
 */

export const actionBarVariants = cva('flex items-center gap-2', {
  variants: {
    /* Where it sits, and therefore which edge grows a border.
     *
     * The border is on the side facing the content, so the bar reads as
     * attached to the page rather than floating over it - and only when it is
     * stuck, because a bar in the middle of a page needs no seam. */
    position: {
      bottom: 'sticky bottom-0 border-t border-line bg-bg/95 py-3 backdrop-blur',
      top: 'sticky top-0 border-b border-line bg-bg/95 py-3 backdrop-blur',
      /* Not stuck at all: a strip in the flow of the page. */
      static: '',
    },
    justify: {
      start: 'justify-start',
      end: 'justify-end',
      between: 'justify-between',
    },
  },
  defaultVariants: { position: 'static', justify: 'start' },
})

export interface ActionBarProps extends Base.Root.Props, VariantProps<typeof actionBarVariants> {}

/**
 * The bar.
 *
 * `aria-label` is worth passing when a page has more than one: "Formatting"
 * and "Bulk actions" are different toolbars, and a screen reader announcing
 * "toolbar" twice tells the reader nothing about which one they are in.
 */
export function ActionBar({ position, justify, className, ...props }: ActionBarProps) {
  return (
    <Base.Root
      className={cn(actionBarVariants({ position, justify }), className)}
      // A stuck bar has to clear what scrolls under it; the theme's own
      // stacking order is what says how high, rather than a number invented
      // here that would disagree with every overlay in the set.
      style={position === 'bottom' || position === 'top' ? { zIndex: 'var(--z-sticky)' } : undefined}
      {...props}
    />
  )
}

/** A button in the bar. Give it the set's own `Button` through `render`, so
 * the toolbar's arrow keys reach it and the button keeps its variants. */
export const ActionBarButton = Base.Button

/** A rule between groups of actions. Decorative, and announced as a separator
 * rather than as a control. */
export function ActionBarSeparator({ className, ...props }: Base.Separator.Props) {
  return <Base.Separator className={cn('mx-1 h-5 w-px shrink-0 bg-line', className)} {...props} />
}

/** A group of related actions inside the bar - the three alignment buttons, a
 * pair of toggles. Groups the arrows walk as one. */
export function ActionBarGroup({ className, ...props }: Base.Group.Props) {
  return <Base.Group className={cn('flex items-center gap-1', className)} {...props} />
}

/** Pushes what follows to the far end. A plain spacer, so a bar can put Cancel
 * beside Save and everything else to the left without a wrapper. */
export function ActionBarSpacer() {
  return <div className="flex-1" aria-hidden />
}
