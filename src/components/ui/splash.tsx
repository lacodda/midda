import type { HTMLAttributes, ReactNode } from 'react'
import { cn } from 'dowel-ui'

/*
 * What the window shows while the application is opening.
 *
 * A desktop product has a second or two between the window appearing and
 * the first screen being ready - a workspace to open, a database to migrate,
 * a plugin to start - and a blank window for that long reads as a crash. So
 * the window shows the product instead: the mark, the name, the promise, the
 * version, and a bar that sweeps until there is something to draw.
 *
 * It is the second half of a pattern, and the first half is not React. The
 * page paints the same picture in inline CSS before the bundle arrives, so
 * that the window is never blank at all; this component takes over from it
 * on the first render, with the same geometry so nothing jumps, and adds the
 * two lines only the application can say - what it is doing, in the person's
 * language, and one thing worth knowing. The static half is in the docs.
 *
 * It is a status region, not a dialog: the reader is told what is happening
 * and cannot act on it. The sweep is a `<style>` of its own rather than a
 * theme keyframe, because a product installs the theme on its first day and
 * this once; and under reduced motion the theme stops every animation dead,
 * which would leave the sweep parked off the end of its track - so the bar
 * is drawn full and still instead.
 */

export interface SplashProps extends Omit<HTMLAttributes<HTMLDivElement>, 'children'> {
  /** The product's mark, drawn at 56px. */
  mark?: ReactNode
  /** The product's name. */
  name: string
  /** The line under the name. */
  tagline?: string
  /** The version, drawn in the mono face; the `v` is the product's to add. */
  version?: string
  /** What the application is doing right now. */
  status?: string
  /** One thing worth knowing while it does it. */
  tip?: string
  /** Whether the bar is sweeping. Still and full when the application is
   * waiting on something that has no progress, such as a person. */
  busy?: boolean
}

/** The sweep, named with a prefix so a product's own `sweep` cannot collide
 * with it in the one document both end up in.
 *
 * The class below writes the name out rather than interpolating this
 * constant, and that is not carelessness: Tailwind finds its classes by
 * scanning the source text, and a class assembled from a template is one it
 * never sees - the bar rendered with the right class name and no CSS behind
 * it, and the stand showed an empty track. The test compiles the class to
 * make sure the two spellings agree. */
const SWEEP = 'dowel-splash-sweep'
const SWEEPING = 'animate-[dowel-splash-sweep_1.1s_ease-in-out_infinite]'

export function Splash({
  mark,
  name,
  tagline,
  version,
  status,
  tip,
  busy = true,
  className,
  ...props
}: SplashProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      className={cn(
        'fixed inset-0 flex flex-col items-center justify-center gap-2.5 bg-bg text-text select-none',
        className,
      )}
      {...props}
    >
      <style>{`@keyframes ${SWEEP} { to { left: 100%; } }`}</style>
      {mark ? <div className="mb-1.5 size-14 [&>svg:not([class*=size-])]:size-full">{mark}</div> : null}
      <div className="text-[26px] leading-none font-semibold tracking-[0.02em]">{name}</div>
      {tagline ? <div className="text-sm text-dim">{tagline}</div> : null}
      {version ? <div className="mt-1.5 font-mono text-2xs text-faint">{version}</div> : null}
      <div className="relative mt-4 h-0.5 w-40 overflow-hidden rounded-full bg-line">
        <div
          data-sweep={busy ? 'on' : 'off'}
          className={cn(
            'absolute top-0 h-full rounded-full bg-accent',
            busy
              ? cn(
                  '-left-2/5 w-2/5',
                  SWEEPING,
                  'motion-reduce:left-0 motion-reduce:w-full motion-reduce:animate-none',
                )
              : 'left-0 w-full',
          )}
        />
      </div>
      {status ? <p className="mt-3 text-xs text-dim">{status}</p> : null}
      {tip ? <p className="text-2xs text-faint">{tip}</p> : null}
    </div>
  )
}
