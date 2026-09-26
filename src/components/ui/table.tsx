import type { HTMLAttributes, TdHTMLAttributes, ThHTMLAttributes } from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from 'dowel-ui'
import { ariaSort, type Sort } from './table-sort'

/*
 * A table of rows, and the heading that reorders them.
 *
 * Parts rather than a `columns` prop, and that is the decision worth stating:
 * a `<DataTable columns={…} rows={…} />` is quicker to write for the first
 * table and then owns every cell in the product forever. The moment one column
 * needs a Badge, another a link, and a third the row's own menu, the prop
 * grows a `render` for each - at which point it is JSX with extra steps, spelt
 * in a shape only this component understands.
 *
 * So the parts are the ones HTML already has, dressed: a caller writes
 * `<TableRow>` and `<TableCell>` and puts whatever it likes inside. What is
 * bought by having them here rather than in the product is that every table of
 * the line has the same row height, the same heading, the same hairline and
 * the same behaviour when a column is sorted - and that a heading that sorts
 * announces it, which is the part hand-rolled tables get wrong.
 *
 * The sorting arithmetic is next door in `table-sort`, with no React in it -
 * a product that sorts on the server imports that and never this.
 *
 * `<TableScroll>` exists because a table cannot scroll itself: `overflow` on a
 * `<table>` does nothing, so the wrapper is not decoration but the only place
 * a scrollbar can live. It is also what makes the sticky heading work, since
 * `position: sticky` needs a scroll container to stick inside.
 */

/*
 * Row height follows the density of the region, like every other control.
 *
 * This had a `density` prop of its own, with its own words - `base` and
 * `dense` - which made "density" mean two different things in one set: an
 * attribute on a container for fields, a prop on this one element for rows. A
 * product wanting a tight screen had to know both and set both, and a table
 * inside a compact form stayed comfortable unless somebody remembered.
 *
 * Now it reads `--row-cell`, so `data-density="compact"` on anything above it
 * tightens the rows with everything else. The font does not shrink with it:
 * shrinking the text too is how a dense table becomes an unreadable one.
 */
export const tableVariants = cva(
  'w-full border-collapse text-left text-sm [&_td]:py-row [&_th]:py-row',
)

export interface TableProps
  extends HTMLAttributes<HTMLTableElement>,
    VariantProps<typeof tableVariants> {}

export function Table({ className, ...props }: TableProps) {
  return <table className={cn(tableVariants(), className)} {...props} />
}

/** The scroll container a table needs, and the one a sticky heading sticks in.
 *
 * The scrollbar is the theme's - an overlay that takes no room in the layout,
 * so a table that grows past the fold does not shift the column beside it. */
export function TableScroll({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cn('w-full overflow-x-auto', className)} {...props} />
}

export interface TableHeadProps extends HTMLAttributes<HTMLTableSectionElement> {
  /** Keep the heading in view while the body scrolls under it.
   *
   * Off by default. A sticky heading needs a container with a height to stick
   * inside; switched on by default it would silently do nothing in the common
   * case - a table that scrolls with the page - and look broken in the other. */
  sticky?: boolean
}

export function TableHead({ sticky = false, className, ...props }: TableHeadProps) {
  return (
    <thead
      className={cn(
        'text-xs text-dim',
        // The heading needs its own ground when it is sticky: transparent, it
        // would have the body's rows sliding visibly beneath its text.
        sticky && 'sticky top-0 z-sticky bg-bg',
        className,
      )}
      {...props}
    />
  )
}

export function TableBody({ className, ...props }: HTMLAttributes<HTMLTableSectionElement>) {
  return <tbody className={cn('', className)} {...props} />
}

export interface TableRowProps extends HTMLAttributes<HTMLTableRowElement> {
  /** The row the reader has picked out - the one open in the panel beside the
   * table, the one a keyboard cursor is on. Not a hover state. */
  selected?: boolean
}

export function TableRow({ selected = false, className, ...props }: TableRowProps) {
  return (
    <tr
      // Announced rather than only drawn: a row picked out by colour alone is
      // a row nobody using a screen reader knows about.
      aria-selected={selected || undefined}
      className={cn(
        'border-b border-line transition-colors last:border-b-0',
        selected ? 'bg-accent-soft' : 'hover:bg-soft',
        className,
      )}
      {...props}
    />
  )
}

export interface TableCellProps extends TdHTMLAttributes<HTMLTableCellElement> {
  /** Right-align and use the lining figures. For a column of numbers, and the
   * reason it is a prop rather than a class the caller adds: a column of
   * numbers that is not aligned is the most common defect in a table, and the
   * one nobody files a bug about. */
  numeric?: boolean
}

export function TableCell({ numeric = false, className, ...props }: TableCellProps) {
  return (
    <td
      className={cn('px-3 align-middle', numeric && 'text-right tabular-nums', className)}
      {...props}
    />
  )
}

export interface TableHeaderProps extends ThHTMLAttributes<HTMLTableCellElement> {
  numeric?: boolean
}

/** A plain heading, for a column that does not sort. */
export function TableHeader({ numeric = false, className, ...props }: TableHeaderProps) {
  return (
    <th
      scope="col"
      className={cn(
        'px-3 font-medium',
        numeric && 'text-right tabular-nums',
        className,
      )}
      {...props}
    />
  )
}

export interface TableSortHeaderProps extends Omit<TableHeaderProps, 'onClick'> {
  /** This column's id - the key the accessor sorts by. */
  column: string
  /** What is sorted now, across the whole table. */
  sort: Sort | undefined
  onSortChange: (column: string) => void
}

/** A heading that reorders the table when clicked.
 *
 * A real `<button>` inside the `<th>`, not a click handler on the cell: the
 * cell is not focusable, gets no keyboard, and announces nothing. This is the
 * part a hand-rolled table almost always gets wrong, and it is invisible to
 * everyone who reorders with a mouse.
 *
 * The arrow is `aria-hidden`, because `aria-sort` on the cell already says
 * which way the column points - a reader would otherwise hear the direction
 * twice, once as a word and once as a character nobody meant to publish. */
export function TableSortHeader({
  column,
  sort,
  onSortChange,
  numeric = false,
  className,
  children,
  ...props
}: TableSortHeaderProps) {
  const active = sort?.column === column
  const direction = sort?.direction

  return (
    <th
      scope="col"
      // Only on the sorted column. `aria-sort="none"` on every other heading
      // is valid and gets announced by some readers on every cell.
      aria-sort={ariaSort(sort, column)}
      className={cn('px-3 font-medium', numeric && 'tabular-nums', className)}
      {...props}
    >
      <button
        type="button"
        onClick={() => onSortChange(column)}
        className={cn(
          'group -mx-1 inline-flex w-[calc(100%+0.5rem)] cursor-pointer items-center gap-1 rounded-sm px-1',
          'transition-colors hover:text-text',
          'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
          active && 'text-text',
          numeric && 'justify-end',
        )}
      >
        {children}
        <span
          aria-hidden
          className={cn(
            'text-2xs transition-opacity',
            // The arrow of the unsorted column appears on hover rather than
            // never: a heading that gives no sign it can be clicked is one
            // nobody clicks. It stays out of the way until the pointer is on
            // it, so a table of eight columns is not a row of eight arrows.
            active ? 'opacity-100' : 'opacity-0 group-hover:opacity-40',
          )}
        >
          {direction === 'desc' && active ? '▼' : '▲'}
        </span>
      </button>
    </th>
  )
}

/** What the table says when it has no rows.
 *
 * A row spanning every column rather than a block below the table, so the
 * heading stays where it is and the table does not change shape between having
 * rows and not. `colSpan` is required: without it the message sits under the
 * first column and the rest of the header hangs over nothing. */
export interface TableEmptyProps extends TdHTMLAttributes<HTMLTableCellElement> {
  colSpan: number
}

export function TableEmpty({ colSpan, className, ...props }: TableEmptyProps) {
  return (
    <tr>
      <td
        colSpan={colSpan}
        className={cn('px-3 py-10 text-center text-sm text-dim', className)}
        {...props}
      />
    </tr>
  )
}
