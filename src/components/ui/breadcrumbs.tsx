import { Fragment, type HTMLAttributes, type ReactNode } from 'react'
import { useRender } from '@base-ui/react/use-render'
import { cn } from 'dowel-ui'

/*
 * Breadcrumbs.
 *
 * Where you are, as a trail rather than as a word. On a list screen the
 * screen's own name is enough; on something opened from it, it is not -
 * "Works" says nothing about which work, and the way back to the list is
 * otherwise the browser's back button alone, which a desktop window does not
 * visibly have.
 *
 * The last crumb is where you stand and is not a link. That is not styling:
 * it carries `aria-current="page"`, so a reader hears which of the four is
 * the destination rather than a list of four places to go. Everything before
 * it is the product's own link element, given through `render` for the reason
 * the rest of the set gives it - the registry has no business installing a
 * router.
 *
 * The trail truncates from the middle when it is longer than it is wide: the
 * first crumb and the last are the two that matter, and dropping the middle
 * into an ellipsis keeps the way home and the place you are. Letting it wrap
 * onto a second line would change the height of a title bar.
 */

export interface Crumb {
  id: string
  label: ReactNode
}

export interface BreadcrumbsProps extends Omit<HTMLAttributes<HTMLElement>, 'children' | 'onSelect'> {
  /** What the trail is called for a reader - "Breadcrumb" by convention, but
   * a product with two of them on one screen should say which. */
  label: string
  items: readonly Crumb[]
  /** The element a crumb before the last is drawn as. Without it a crumb is a
   * button and `onSelect` says which was pressed. */
  render?: (item: Crumb) => useRender.RenderProp
  onSelect?: (id: string) => void
  /**
   * How many crumbs are drawn before the middle is folded away. The first and
   * the last are always kept, so this is a floor of three.
   *
   * The default is four, which is one more than any screen in the line
   * actually reaches: a trail deeper than that is a sign the screens are
   * nested more deeply than anyone will navigate.
   */
  max?: number
}

/** The trail with its middle folded away, and where the fold happened.
 *
 * The first crumb is kept because it is the way out, and the last because it
 * is where you are; the ellipsis stands for everything between. It is not a
 * menu: a product whose trail is regularly too long has a navigation problem
 * that a disclosure on a crumb would only hide. */
function fold(items: readonly Crumb[], max: number): (Crumb | 'ellipsis')[] {
  if (items.length <= max) return [...items]
  // `max - 2` for the first crumb and the ellipsis, so the count the caller
  // asked for is the count that is drawn.
  return [items[0]!, 'ellipsis', ...items.slice(items.length - (max - 2))]
}

export function Breadcrumbs({
  label,
  items,
  render,
  onSelect,
  max = 4,
  className,
  ...props
}: BreadcrumbsProps) {
  const shown = fold(items, Math.max(3, max))

  return (
    <nav aria-label={label} className={cn('flex min-w-0 items-center', className)} {...props}>
      {/* An ordered list, because the order is the meaning: a reader is told
          "list, 3 items" and then hears them from the root inwards. */}
      <ol className="flex min-w-0 items-center gap-1.5 text-sm">
        {shown.map((entry, index) => (
          <Fragment key={entry === 'ellipsis' ? 'ellipsis' : entry.id}>
            {index === 0 ? null : (
              // Hidden from a reader: the separator is punctuation drawn
              // between list items that are already announced as a list, and
              // read aloud it becomes "Works, greater than, Harbour lights".
              <li aria-hidden className="shrink-0 text-faint" role="presentation">
                ›
              </li>
            )}
            {entry === 'ellipsis' ? (
              <li className="shrink-0 text-faint">…</li>
            ) : (
              <BreadcrumbItem
                item={entry}
                last={index === shown.length - 1}
                render={index === shown.length - 1 ? undefined : render?.(entry)}
                onSelect={onSelect}
              />
            )}
          </Fragment>
        ))}
      </ol>
    </nav>
  )
}

function BreadcrumbItem({
  item,
  last,
  render,
  onSelect,
}: {
  item: Crumb
  last: boolean
  render?: useRender.RenderProp
  onSelect?: (id: string) => void
}) {
  const crumb = useRender({
    render,
    // Where you stand is not somewhere to go: the last crumb is text, not a
    // control, so it is neither focusable nor announced as a link.
    defaultTagName: last ? 'span' : 'button',
    props: {
      ...(render === undefined && !last ? { type: 'button' } : {}),
      'aria-current': last ? ('page' as const) : undefined,
      onClick: last ? undefined : () => onSelect?.(item.id),
      className: cn(
        'block truncate rounded-xs no-underline',
        'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
        last
          ? 'font-semibold text-text'
          : 'cursor-pointer text-dim transition-colors hover:text-text',
      ),
      children: item.label,
    },
  })

  return (
    // The crumbs before the last keep their width; the last is the one that
    // gives, because it is the one that is a title and may be a sentence
    // long. `min-w-0` on it so `truncate` inside has something to shrink to.
    <li className={last ? 'min-w-0' : 'shrink-0'}>{crumb}</li>
  )
}
