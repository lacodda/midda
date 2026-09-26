import type { CSSProperties, HTMLAttributes, ReactNode } from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from 'dowel-ui'

/*
 * AppShell.
 *
 * The frame a product draws once and then never thinks about: a bar across
 * the top, a rail down the left, and the screen in the corner they leave. It
 * is three boxes and a grid, which is exactly why every product wrote its own
 * - and why every one of them got the same two things wrong before getting
 * them right.
 *
 * Those two are the whole reason this is shared rather than copied. A grid
 * item's default `min-width` and `min-height` are `auto`, not zero, so a
 * track measures its content instead of its share of the window. Without
 * `min-h-0` on the rail, its border and footer stopped halfway down a tall
 * window; without `min-w-0` on the content column, one wide table pushed the
 * whole screen out from under the sidebar and there was nothing left to
 * scroll. Both were found in a shipped build, by eye, months apart.
 *
 * The other decision is that the window never scrolls, on either axis.
 * `Screen` hands its height down and each screen scrolls inside itself, so
 * the bottom edge of the content is always the bottom edge of the window and
 * a long page announces itself with a bar rather than hiding its end. A
 * desktop window has a bottom edge; content that runs past it the way a web
 * page does pretends it does not.
 *
 * What it does NOT decide: what is in the bar, what is in the rail, or how
 * you navigate. Those are the product's, and they differ more than they look
 * - one of the line's products draws its own title bar with the window
 * buttons in it, another draws a web header with a phone's bottom bar under
 * it. This is the geometry they share.
 */

export interface AppShellProps extends HTMLAttributes<HTMLDivElement> {
  /** The strip across the top, full width - a title bar, a header, a banner
   * row. Leave it out and the rail runs to the top of the window. */
  top?: ReactNode
  /** The rail down the left. Leave it out for a product that navigates from
   * the top bar alone, which is the usual shape on the web. */
  side?: ReactNode
  /** How wide the rail is. A number is pixels; a string is any CSS track
   * size, for a rail that collapses or is dragged. Defaults to the theme's
   * `--spacing-rail`, so four products stop each choosing their own. */
  sideWidth?: number | string
  /** How tall the top bar is, same rules, defaulting to
   * `--spacing-titlebar`. It is a fixed track rather than `auto` on purpose:
   * a bar that grows by a pixel when its content changes moves every screen
   * under it. */
  topHeight?: number | string
  children: ReactNode
}

/** A track size from a number of pixels or a CSS length the caller wrote. */
function track(value: number | string): string {
  return typeof value === 'number' ? `${value}px` : value
}

/**
 * The frame.
 *
 * `h-full` rather than `h-screen`: inside Tauri the window IS the viewport,
 * but on the web this often sits under a banner or inside a container that
 * has already taken its share, and `100vh` would push the bottom of the shell
 * that far below the fold. The host gives it a height; it fills it.
 */
export function AppShell({
  top,
  side,
  sideWidth = 'var(--spacing-rail)',
  topHeight = 'var(--spacing-titlebar)',
  className,
  style,
  children,
  ...props
}: AppShellProps) {
  // The tracks are built from what is actually there, so a shell without a
  // rail is a one-column grid rather than a two-column grid with an empty
  // column that still eats 216 pixels.
  const columns = side === undefined ? 'minmax(0, 1fr)' : `${track(sideWidth)} minmax(0, 1fr)`
  const rows = top === undefined ? 'minmax(0, 1fr)' : `${track(topHeight)} minmax(0, 1fr)`

  return (
    <div
      className={cn('grid h-full w-full overflow-hidden bg-bg text-text', className)}
      style={{ gridTemplateColumns: columns, gridTemplateRows: rows, ...style } as CSSProperties}
      {...props}
    >
      {/* The bar spans every column, including the rail's: the mark sits
          where a system title bar would print the name, and the window's own
          buttons sit at the far end of the same strip. A bar that started
          after the rail left a notch in the top-left corner that nothing
          could fill. */}
      {top === undefined ? null : (
        <div className="col-span-full min-w-0" style={{ gridColumn: '1 / -1' }}>
          {top}
        </div>
      )}
      {/* `min-h-0`: without it the rail measures its content rather than the
          track, and a short nav in a tall window ends its border in mid-air. */}
      {side === undefined ? null : <div className="min-h-0">{side}</div>}
      {/* `min-w-0` beside it, on the other axis and for the same reason: a
          wide table inside this column grew the column instead of scrolling
          inside it. `relative` so a product can portal something over the
          screen without covering the bar and the rail. */}
      <div className="relative flex min-h-0 min-w-0 flex-col">{children}</div>
    </div>
  )
}

export const screenVariants = cva('min-h-0 flex-1', {
  variants: {
    /*
     * Where the scrolling happens.
     *
     * `flow` is the common case and the safe one: the screen is as long as
     * its content and scrolls within the height it was handed.
     *
     * `held` is for a screen that lays its own boxes out against that height
     * - a table with a sticky header, two columns that each scroll - and so
     * must not grow. It scrolls nowhere itself; something inside it does.
     * Chosen by mistake, `held` clips; `flow` never does, which is why it is
     * the default.
     */
    scroll: {
      /* `scrollbar-gutter: stable` keeps the bar's width reserved whether or
       * not this screen needs one. Without it every navigation was a small
       * sideways jump, because a skeleton is short and what replaces it is
       * not. */
      flow: 'overflow-y-auto overflow-x-hidden [scrollbar-gutter:stable]',
      held: 'flex flex-col overflow-hidden',
    },
    /* The page's own margins. `none` is for a screen that draws edge to edge
     * - a map, a canvas, an image - where padding would show the page
     * underneath along every side. */
    pad: {
      default: 'px-6 pt-3 pb-6',
      tight: 'px-4 pt-2 pb-4',
      none: '',
    },
  },
  defaultVariants: { scroll: 'flow', pad: 'default' },
})

export interface ScreenProps
  extends HTMLAttributes<HTMLElement>,
    VariantProps<typeof screenVariants> {}

/**
 * One screen inside the frame, handed the height the window has left.
 *
 * A `<main>`, so a reader can jump to the content past the bar and the rail.
 * There is one per shell: it is what the router renders into, not what each
 * route wraps itself in.
 */
export function Screen({ scroll, pad, className, ...props }: ScreenProps) {
  return <main className={cn(screenVariants({ scroll, pad }), className)} {...props} />
}
