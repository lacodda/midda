/**
 * The static half of the splash: what `index.html` paints before the bundle.
 *
 * The page cannot read the theme at that moment - the stylesheet arrives with
 * the bundle, which is what is being waited for - so the splash's colours,
 * mark, words and version are written into the page when it is built. They
 * come from where the rest of the window gets them: the colours from dowel's
 * palette for midda (the theme's own formulas, resolved), the mark from the
 * line's masters in `dowel-ui/marks`, the words from `splash.json`, which the
 * React half reads too. kilna wrote its splash's colours and mark out by hand,
 * which is a second copy of each that nothing holds to the first.
 *
 * Imported by `vite.config.ts`, so only packages and relative paths: the `@`
 * alias belongs to the bundle, not to the config.
 */

import { markLevel, marks } from 'dowel-ui/marks'
import palette from 'dowel-ui/palettes/midda.json' with { type: 'json' }
import words from './splash.json' with { type: 'json' }

/** How big the splash draws the mark, in CSS pixels. Splash's `size-14`. */
export const SPLASH_MARK_SIZE = 56

/** The theme colours the static half paints with, in both themes. */
const COLOURS = ['bg', 'text', 'dim', 'faint', 'line', 'accent'] as const

/** Text as HTML text: the words are prose, and prose has apostrophes and may
 * one day have an ampersand. */
function escape(text: string): string {
  return text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
}

/** Everything `index.html` may ask for, by the name it asks with. */
export function splashValues(version: string): Record<string, string> {
  const values: Record<string, string> = {
    version: escape(version),
    tagline: escape(words.tagline),
    status: escape(words.status),
    tip: escape(words.tip),
    // A two-colour mark names its gradient `{{id}}`; midda's is one colour
    // today, and the id is filled in anyway so a new master cannot leave the
    // placeholder in the page.
    mark: marks.midda[markLevel(SPLASH_MARK_SIZE)].replaceAll('{{id}}', 'splash-mark'),
  }
  for (const theme of ['dark', 'light'] as const) {
    for (const name of COLOURS) {
      values[`${theme}.${name}`] = palette.color[theme][name].$value
    }
  }
  return values
}

/**
 * Fills every `{{name}}` in the page from `values`.
 *
 * Strict both ways, because either mistake is silent on screen. A name the
 * values do not have would ship the braces into the window; a value the page
 * never asks for is a line the React half draws and the static half does not,
 * and the picture jumps by that line's height when React takes over.
 */
export function fillSplashPage(html: string, values: Record<string, string>): string {
  const used = new Set<string>()
  const filled = html.replace(/\{\{([\w.-]+)\}\}/g, (_, name: string) => {
    const value = values[name]
    if (value === undefined) throw new Error(`index.html asks for {{${name}}}, which the splash does not know`)
    used.add(name)
    return value
  })

  if (filled.includes('{{')) throw new Error('index.html has a {{ that is not a name the splash fills')
  const unused = Object.keys(values).filter((name) => !used.has(name))
  if (unused.length > 0) throw new Error(`index.html never asks for ${unused.join(', ')}`)
  return filled
}
