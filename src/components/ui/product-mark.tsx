import { useId } from 'react'
import { cn } from 'dowel-ui'
import { markLevel, marks, type MarkName } from 'dowel-ui/marks'

/*
 * ProductMark - a product's mark from the lacodda line, drawn at the level its
 * size calls for.
 *
 * Every product of the line has one mark, a hexagonal tile with a two-letter
 * code, in three levels that are a rule rather than options: from 64px it
 * carries the product's metaphor under the code, from 28px it is the code
 * alone and larger, and at 27px and under it is the tile filled with the
 * product's colour and a heavy code - a metaphor at sixteen pixels is noise,
 * and a thin outline disappears. A product that drew its own mark picked one
 * level and used it at every size, so the same mark was a smudge in the tab
 * strip and a bare tile on the About screen. Here the size is the only input,
 * and the level follows.
 *
 * The masters are the line's, drawn by its mark generator and shipped in the
 * package (`dowel-ui/marks`); nothing is redrawn here. The tile is dark in
 * both themes, as the line draws it everywhere - the mark is the product's
 * signature, not part of the screen's palette.
 *
 * A mark beside the product's name is decoration and is hidden from a reader,
 * who already has the name. Given `label`, it becomes an image with that name,
 * for the places where the mark stands alone.
 */

export interface ProductMarkProps {
  /** Which mark: a product of the line, or `lacodda` for the line itself. */
  product: MarkName
  /** Its width and height in CSS pixels, which also choose its level. */
  size?: number
  /** Its name, when nothing next to it says which product it is. */
  label?: string
  className?: string
}

export function ProductMark({ product, size = 24, label, className }: ProductMarkProps) {
  // SVG ids are global to the document, and a two-colour mark names its
  // gradient: two such marks on one screen would share the first one's. The
  // id React makes is unique, but carries characters an `url(#...)` reference
  // does not tolerate everywhere.
  const id = `mark-${useId().replace(/[^a-zA-Z0-9_-]/g, '')}`
  const markup = marks[product][markLevel(size)].replaceAll('{{id}}', id)

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 100 100"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      data-mark={product}
      data-level={markLevel(size)}
      className={cn('shrink-0', className)}
      dangerouslySetInnerHTML={{ __html: markup }}
    />
  )
}

/** The line's own mark - λ on a graphite tile - for the places that say a
 * product belongs to the lacodda line. */
export function LineMark(props: Omit<ProductMarkProps, 'product'>) {
  return <ProductMark product="lacodda" {...props} />
}
