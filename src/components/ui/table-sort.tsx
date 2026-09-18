export type SortDirection = 'asc' | 'desc'

/** A column, and which way it points. `column` is the caller's own key - the
 * id it gave the column, not an index, so reordering columns cannot silently
 * change what is sorted. */
export interface Sort<Column extends string = string> {
  column: Column
  direction: SortDirection
}

/** What can be compared. `null` and `undefined` both mean absent - a product
 * gets whichever its data uses, and having to normalise them before sorting
 * is the kind of step that gets forgotten in one branch of a switch. */
export type SortValue = string | number | boolean | null | undefined

/** Reads the value of a column out of a row.
 *
 * Given rather than inferred: a column id is not always a field name, and a
 * sortable column is often a computed one - a duration that is stored as two
 * timestamps, a name that sorts by surname. */
export type SortAccessor<Row, Column extends string = string> = (
  row: Row,
  column: Column,
) => SortValue

export interface SortOptions<Row> {
  /** The tiebreaker: what decides the order of rows that compare equal.
   *
   * Without one, `Array.prototype.sort` is stable and therefore leaves ties in
   * input order - which sounds fine until the input is re-fetched and arrives
   * in a different order, and a column of ties reshuffles under the reader
   * with no click. Pass the row's id. */
  tiebreak?: (row: Row) => SortValue
  /** The locale text is compared in. Passed to `Intl.Collator`, so `ä` sorts
   * where the reader expects rather than after `z`. */
  locale?: string
}

/** Order the rows. Returns a new array; the input is not touched, because a
 * component that sorts its own prop in place mutates the caller's state. */
export function sortRows<Row, Column extends string = string>(
  rows: readonly Row[],
  sort: Sort<Column>,
  accessor: SortAccessor<Row, Column>,
  options: SortOptions<Row> = {},
): Row[] {
  const sign = sort.direction === 'asc' ? 1 : -1
  // One collator for the whole sort rather than one `localeCompare` per
  // comparison: building it is the expensive half, and a sort of n rows asks
  // for it n log n times.
  const collator = new Intl.Collator(options.locale)

  return [...rows].sort((a, b) => {
    const left = accessor(a, sort.column)
    const right = accessor(b, sort.column)

    // Presence first, and outside the sign. This is the whole point of the
    // file: absence is last in both directions.
    const absent = presence(left, right)
    if (absent !== 0) return absent

    const ranked = compareValues(left, right, collator) * sign
    if (ranked !== 0) return ranked

    if (options.tiebreak) {
      const tied = compareValues(options.tiebreak(a), options.tiebreak(b), collator)
      // Deliberately not multiplied by the sign. The tiebreaker is there to
      // make the order stable, and an order that reverses with the column is
      // not stable - the rows that tie would swap places on every click.
      if (tied !== 0) return tied
    }
    return 0
  })
}

/** Whether a value counts as absent.
 *
 * Both spellings of nothing, and neither `0` nor `''` nor `false`, which are
 * values a row genuinely has.
 *
 * `NaN` counts too, and that is not tidiness. Subtraction with it returns
 * `NaN`, which `sort` reads as "these two are equal" - so a row whose number
 * is not a number takes whatever place the input happened to give it, and the
 * order changes when the data is re-fetched. A number that is not a number is
 * not a small number; it is unknown, which is what this file already has a
 * place for. */
export function isAbsent(value: SortValue): boolean {
  if (value === null || value === undefined) return true
  return typeof value === 'number' && Number.isNaN(value)
}

/** Which of the two lacks a value; absent sorts last, always. */
function presence(left: SortValue, right: SortValue): number {
  const leftHas = !isAbsent(left)
  const rightHas = !isAbsent(right)
  if (leftHas === rightHas) return 0
  return leftHas ? -1 : 1
}

/** Compare two present values of the same column.
 *
 * Numbers by subtraction, text by collator, and booleans as false-then-true.
 * A column whose values are of mixed type compares as text: that is a data
 * problem the table cannot fix, and ordering by `String` at least gives the
 * same answer twice. */
function compareValues(left: SortValue, right: SortValue, collator: Intl.Collator): number {
  if (isAbsent(left) || isAbsent(right)) return presence(left, right)

  // NaN never reaches here: `isAbsent` counts it as absent, so `presence`
  // settles it above, outside the sign.
  if (typeof left === 'number' && typeof right === 'number') return left - right
  if (typeof left === 'boolean' && typeof right === 'boolean') {
    return Number(left) - Number(right)
  }
  return collator.compare(String(left), String(right))
}

/** What clicking a column heading does.
 *
 * Three states rather than two, and the third is the reason this is a function
 * and not `direction === 'asc' ? 'desc' : 'asc'`: clicking a *different*
 * column starts it ascending rather than inheriting the direction of the one
 * before. Inheriting is what a two-state toggle does, and it means the first
 * click on a new column can hand back an order nobody asked for. */
export function toggleSort<Column extends string>(
  sort: Sort<Column>,
  column: Column,
): Sort<Column> {
  if (sort.column !== column) return { column, direction: 'asc' }
  return { column, direction: sort.direction === 'asc' ? 'desc' : 'asc' }
}

/** What a column heading announces: `ascending`, `descending`, or nothing.
 *
 * The value belongs in `aria-sort` on the `<th>`, and only on the one that is
 * sorted - `aria-sort="none"` on every other heading is allowed by the spec
 * and read out by some screen readers on every cell, which turns a table into
 * a recital. `undefined` removes the attribute. */
export function ariaSort(
  sort: Sort<string> | undefined,
  column: string,
): 'ascending' | 'descending' | undefined {
  if (!sort || sort.column !== column) return undefined
  return sort.direction === 'asc' ? 'ascending' : 'descending'
}
