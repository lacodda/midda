import { useRef } from 'react'
import { Popover as Base } from '@base-ui/react/popover'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from 'dowel-ui'
import { LayerProvider, usePopupContainer } from './layer'

/*
 * Popover.
 *
 * A panel anchored to the thing that opened it, for content that belongs
 * beside a control rather than in the middle of the screen: a filter, a small
 * form, a menu of settings that is not a menu.
 *
 * Where Dialog is centred and covers the page, this one is placed, and the
 * placing is the part that is easy to get wrong. It goes through a Positioner
 * between the Portal and the Popup - Base UI measures the trigger, measures
 * the popup, and flips or shifts it when the preferred side does not fit. So
 * `side` and `align` are a preference, not an instruction, and a popover near
 * the bottom of the window will come out above its trigger. That is the
 * behaviour worth having; a popover that stays where it was told is a popover
 * half off the screen.
 *
 * `sideOffset` is a distance from the anchor and belongs to the Positioner,
 * not the Popup, so it is taken here and passed inward. The Arrow is optional
 * and sits inside the Popup, where Base UI rotates it to whichever side the
 * popup actually landed on.
 *
 * It is not modal. The page underneath stays live, and focus is not trapped -
 * which is right for a panel beside a control and wrong for a decision.
 */

export const popoverPopupVariants = cva(
  [
    'rounded-lg border border-line bg-raise p-4 text-text shadow-raise',
    'focus-visible:outline-none',
    // The enter and the leave. `duration-*` reads the token directly because
    // Tailwind's own utility takes a literal number.
    '[transition:opacity_var(--duration-quick)_var(--ease-out),transform_var(--duration-quick)_var(--ease-out)]',
    'data-[closed]:scale-[0.97] data-[closed]:opacity-0',
    'data-[starting-style]:scale-[0.97] data-[starting-style]:opacity-0',
    // Grow out of the edge it is anchored to rather than out of its own
    // middle, so the motion points back at the trigger.
    'origin-[var(--transform-origin)]',
  ],
  {
    variants: {
      size: {
        sm: 'w-[min(16rem,calc(100vw-2rem))]',
        md: 'w-[min(20rem,calc(100vw-2rem))]',
        lg: 'w-[min(28rem,calc(100vw-2rem))]',
      },
    },
    defaultVariants: { size: 'md' },
  },
)

/** The root. Controlled with `open` and `onOpenChange`, or left to manage
 * itself around a `PopoverTrigger`. */
export const Popover = Base.Root

/** What opens it, and what the popup is measured against. */
export const PopoverTrigger = Base.Trigger

/** What closes it, for a button inside the panel. */
export const PopoverClose = Base.Close

export interface PopoverPopupProps
  extends Base.Popup.Props,
    VariantProps<typeof popoverPopupVariants> {
  /** Preferred side of the trigger. Base UI flips it when it does not fit,
   * and defaults it to the bottom. */
  side?: Base.Positioner.Props['side']
  /** Alignment along that side. Base UI centres it by default. */
  align?: Base.Positioner.Props['align']
  /** Distance from the trigger, in pixels. */
  sideOffset?: Base.Positioner.Props['sideOffset']
  /** Whether to draw the arrow pointing back at the trigger. */
  arrow?: boolean
  /** Where to portal to. Defaults to the raised host of the overlay this is
   * opened inside (`layer.tsx`), and to the document body when there is none -
   * either way not the element it was opened from, whose `overflow` would clip
   * it. Pass an element to put it somewhere else, such as a container being
   * screenshotted. */
  container?: Base.Portal.Props['container']
}

/** The panel. Portalled and positioned, so it is not clipped by an ancestor
 * with `overflow: hidden` - which is where most anchored popups go to die.
 *
 * `side` and `align` are passed straight through rather than defaulted here:
 * Base UI's own defaults are already bottom and centre, and writing them out
 * again would be one more place for the two to disagree. */
export function PopoverPopup({
  size,
  side,
  align,
  sideOffset = 8,
  arrow = true,
  container,
  className,
  children,
  ...props
}: PopoverPopupProps) {
  // Inside an overlay, the overlay's raised host rather than the body - or
  // this popup draws under the dialog, drawer or popover that opened it. See
  // `layer.tsx`. Outside every overlay the hook gives `undefined`: the body.
  const host = usePopupContainer()

  const portal = useRef<HTMLDivElement>(null)

  return (
    <Base.Portal ref={portal} container={container ?? host}>
      <Base.Positioner
        side={side}
        align={align}
        sideOffset={sideOffset}
        className="[z-index:var(--z-floating)]"
      >
        <Base.Popup className={cn(popoverPopupVariants({ size }), className)} {...props}>
          {arrow ? <PopoverArrow /> : null}
          {/* A popover is a layer of its own: a select or a date picker inside it
            * opens above it, not under it. */}
          <LayerProvider above="floating" mount={portal}>{children}</LayerProvider>
        </Base.Popup>
      </Base.Positioner>
    </Base.Portal>
  )
}

/** The notch pointing back at the trigger. Base UI rotates it to whatever side
 * the popup landed on, which is why the rotation is keyed off `data-side`
 * rather than off the `side` that was asked for. */
export function PopoverArrow({ className, ...props }: Base.Arrow.Props) {
  return (
    <Base.Arrow
      className={cn(
        'h-2 w-2 rotate-45 border border-line bg-raise',
        'data-[side=bottom]:-top-1 data-[side=bottom]:border-r-0 data-[side=bottom]:border-b-0',
        'data-[side=top]:-bottom-1 data-[side=top]:border-t-0 data-[side=top]:border-l-0',
        'data-[side=left]:-right-1 data-[side=left]:border-b-0 data-[side=left]:border-l-0',
        'data-[side=right]:-left-1 data-[side=right]:border-r-0 data-[side=right]:border-t-0',
        className,
      )}
      {...props}
    />
  )
}

/** The heading. Base UI points the popup's `aria-labelledby` at it, so a
 * popover with one is named for a screen reader without anyone arranging it. */
export function PopoverTitle({ className, ...props }: Base.Title.Props) {
  return <Base.Title className={cn('text-sm font-semibold', className)} {...props} />
}

/** The line under the heading, and the popup's `aria-describedby`. */
export function PopoverDescription({ className, ...props }: Base.Description.Props) {
  return <Base.Description className={cn('mt-1 text-sm text-dim', className)} {...props} />
}
