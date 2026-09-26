import {
  useCallback,
  useEffect,
  useState,
  type CSSProperties,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
} from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { cn } from 'dowel-ui'

/** The eight compass names Tauri resizes by. Read off the method rather than
 * imported: the package declares the type without exporting it. */
type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>['startResizeDragging']>[0]

/*
 * The window's own frame, for a window that has no system frame.
 *
 * With `decorations: false` the system draws nothing, so everything it used
 * to do is the page's: dragging the window by its title bar, double-click to
 * maximise, the three buttons, and the edges you grab to resize. Each is
 * small; the reason to take them on at all is that a system title bar over an
 * application title bar costs a strip of every laptop screen for nothing.
 * scheda made the trade first and kilna copied it, which is the second
 * consumer the line asks for before anything becomes shared.
 *
 * `TitleBar` is the bar itself, assembled: the product's mark, whatever the
 * window shows at the top (its open documents as `Tabs variant="bar"`, or a
 * trail), a stretch that exists only to be grabbed, the product's own actions,
 * and the three buttons. `ResizeEdges` goes once at the root beside it. The
 * parts are exported too - `WindowButtons`, `useTitleBarGestures()` spread on
 * a bar of your own, and `useMaximized()` for anything else that changes shape
 * with the window - for a bar the assembled one does not fit.
 *
 * Outside Tauri - a browser, a test, the stand - there is no window to drive.
 * Every call goes through `currentWindow()`, which answers null when the
 * Tauri bridge is absent, so the chrome renders and does nothing rather than
 * throwing on the first click. A product's own storybook runs in a browser
 * too, and a title bar that crashes it is a title bar nobody previews.
 */

/** The Tauri window, or null where there is none to drive. The bridge is what
 * `getCurrentWindow` reads its label from, so its absence is the test. */
function currentWindow() {
  return '__TAURI_INTERNALS__' in window ? getCurrentWindow() : null
}

/** Whether the window is maximised, kept current as the window changes.
 *
 * The window can be maximised without our buttons - a drag to the top edge,
 * the keyboard, a snap layout - so the answer follows the window rather than
 * our own last click. */
export function useMaximized(): boolean {
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    const target = currentWindow()
    if (!target) return
    const read = () => {
      target.isMaximized().then(setMaximized).catch(() => undefined)
    }
    read()
    const unlisten = target.onResized(read)
    return () => {
      unlisten.then((stop) => stop()).catch(() => undefined)
    }
  }, [])

  return maximized
}

export interface WindowButtonsProps {
  /** What each button is called. Required, and deliberately without a
   * default: a string this component invents is a string the product cannot
   * translate. `restore` replaces `maximize` while the window is maximised. */
  labels: { minimize: string; maximize: string; restore: string; close: string }
  className?: string
}

type Control = keyof WindowButtonsProps['labels']

/** The four glyphs, drawn in one stroke on a ten-pixel grid - the size the
 * system's own were, so the bar reads as the window's and not as a toolbar. */
const GLYPH: Record<Control, ReactNode> = {
  minimize: <path d="M0 5h10" />,
  maximize: <rect x="0.5" y="0.5" width="9" height="9" />,
  restore: <path d="M2.5 2.5V0.5h7v7h-2M0.5 2.5h7v7h-7z" />,
  close: <path d="M0 0l10 10M10 0L0 10" />,
}

/** The window controls, in the order Windows puts them.
 *
 * Close asks rather than closes: Tauri's `close()` emits `closeRequested`
 * before anything happens, so a product's unsaved-work guard listening for
 * that request sees this button exactly as it sees the system's own close.
 * scheda once took an `onClose` of its own to get that; it was never needed,
 * and a second way to close is a second place for the guard to be missed. */
export function WindowButtons({ labels, className }: WindowButtonsProps) {
  const maximized = useMaximized()
  const controls: [Control, () => unknown][] = [
    ['minimize', () => currentWindow()?.minimize()],
    [maximized ? 'restore' : 'maximize', () => currentWindow()?.toggleMaximize()],
    ['close', () => currentWindow()?.close()],
  ]

  return (
    <div className={cn('flex h-full shrink-0 items-stretch', className)}>
      {controls.map(([name, act]) => (
        <button
          key={name}
          type="button"
          aria-label={labels[name]}
          title={labels[name]}
          onClick={() => void act()}
          className={cn(
            'flex h-full w-window-button cursor-default items-center justify-center text-dim transition-colors',
            'hover:bg-soft hover:text-text',
            // The close button is the one that must not be mistaken for its
            // neighbours: it goes red under the pointer, as on every desktop.
            name === 'close' && 'hover:bg-bad hover:text-on-bad',
          )}
        >
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" aria-hidden>
            {GLYPH[name]}
          </svg>
        </button>
      ))}
    </div>
  )
}

/** A press on a control has already been handled by the control. */
const shouldHandle = (target: EventTarget | null) =>
  !(target as HTMLElement | null)?.closest(
    'button, a, input, textarea, [role="menu"], [role="menuitem"], [role="tab"], [role="dialog"]',
  )

/** How far the pointer moves before a press becomes a drag, in pixels. */
const THRESHOLD = 4

/** Makes an element behave like a title bar: drag to move, double-click to
 * maximise. Both are what the system used to do for free. Spread the result
 * on the bar: `<header {...useTitleBarGestures()}>`.
 *
 * Two handlers rather than one. A `pointerdown` cannot recognise a double
 * click: its `detail` counts clicks of the *mouse* event sequence, and the
 * second press still arrives as 1 - reading it there fired `startDragging`
 * three times over a double click and toggled nothing. So the press starts a
 * drag, and `dblclick`, which the browser is the one qualified to detect,
 * maximises.
 *
 * Dragging starts on the first movement, not on the press. `startDragging`
 * hands the window over to the system - which is what keeps snap layouts and
 * drag-to-edge working - but from that moment the webview stops seeing the
 * mouse. Calling it on `pointerdown` ate the second click of every double
 * click, and maximising never happened. */
export function useTitleBarGestures() {
  const onPointerDown = useCallback((event: PointerEvent) => {
    if (event.button !== 0 || !shouldHandle(event.target)) return

    const start = { x: event.clientX, y: event.clientY }
    const onMove = (move: globalThis.PointerEvent) => {
      if (Math.abs(move.clientX - start.x) < THRESHOLD && Math.abs(move.clientY - start.y) < THRESHOLD) {
        return
      }
      stop()
      void currentWindow()?.startDragging()
    }
    const stop = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', stop)
      window.removeEventListener('pointercancel', stop)
    }

    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', stop)
    window.addEventListener('pointercancel', stop)
  }, [])

  const onDoubleClick = useCallback((event: MouseEvent) => {
    if (event.button !== 0 || !shouldHandle(event.target)) return
    void currentWindow()?.toggleMaximize()
  }, [])

  return { onPointerDown, onDoubleClick }
}

/** The eight edges and corners a frameless window still has to offer. */
const RESIZE_HANDLES: readonly ResizeDirection[] = [
  'North',
  'South',
  'East',
  'West',
  'NorthEast',
  'NorthWest',
  'SouthEast',
  'SouthWest',
]

/* The width of a strip and of a corner, as the theme states them. Read off
 * the tokens rather than written here: a window's chrome is shared with the
 * products that draw the rest of their own frame, and two numbers for one
 * edge is how the title bar ended up 40px in one product and 2.4rem in the
 * next. */
const EDGE = 'var(--spacing-resize-edge)'
const CORNER = 'var(--spacing-resize-corner)'

/** Where each strip sits and which cursor it shows. Inline styles rather than
 * classes: eight positions of a few pixels each are geometry, not design. */
const EDGE_STYLE: Record<ResizeDirection, CSSProperties> = {
  North: { top: 0, left: CORNER, right: CORNER, height: EDGE, cursor: 'ns-resize' },
  South: { bottom: 0, left: CORNER, right: CORNER, height: EDGE, cursor: 'ns-resize' },
  East: { top: CORNER, bottom: CORNER, right: 0, width: EDGE, cursor: 'ew-resize' },
  West: { top: CORNER, bottom: CORNER, left: 0, width: EDGE, cursor: 'ew-resize' },
  NorthEast: { top: 0, right: 0, width: CORNER, height: CORNER, cursor: 'nesw-resize' },
  NorthWest: { top: 0, left: 0, width: CORNER, height: CORNER, cursor: 'nwse-resize' },
  SouthEast: { bottom: 0, right: 0, width: CORNER, height: CORNER, cursor: 'nwse-resize' },
  SouthWest: { bottom: 0, left: 0, width: CORNER, height: CORNER, cursor: 'nesw-resize' },
}

export interface ResizeEdgesProps {
  /** Merged into every strip. `fixed` to the viewport by default, which is
   * where a window's edges are; `absolute` puts them on the nearest
   * positioned box instead, for a frame drawn inside a page. */
  className?: string
}

/** Invisible strips along the window's edges.
 *
 * A frameless window has no border to grab, so these put one back. They sit
 * outside the flow, above everything, and are only a few pixels wide -
 * enough to hit, not enough to steal a click meant for the text. A maximised
 * window has no edges to drag, and leaving the strips in place would mean
 * the top few pixels of the title bar stop taking clicks. */
export function ResizeEdges({ className }: ResizeEdgesProps) {
  const maximized = useMaximized()
  if (maximized) return null

  return (
    <>
      {RESIZE_HANDLES.map((direction) => (
        <div
          key={direction}
          aria-hidden
          data-resize-edge={direction}
          className={cn('fixed [z-index:var(--z-floating)]', className)}
          style={EDGE_STYLE[direction]}
          onPointerDown={(event) => {
            if (event.button !== 0) return
            event.preventDefault()
            void currentWindow()?.startResizeDragging(direction)
          }}
        />
      ))}
    </>
  )
}

export interface TitleBarProps {
  /** The window buttons' names; see `WindowButtons`. */
  labels: WindowButtonsProps['labels']
  /** The product's mark, at the left edge, where the system put the icon. */
  mark?: ReactNode
  /** What the window shows at the top: its open documents as
   * `<Tabs><TabsList variant="bar">`, a trail, or a title. */
  children?: ReactNode
  /** The product's own controls, between the handle and the window buttons. */
  actions?: ReactNode
  /** Something that belongs to the whole window rather than to either end -
   * the search box, most often - held at the window's centre. */
  center?: ReactNode
  className?: string
}

/** A frameless window's title bar, assembled.
 *
 * Its height is `--spacing-titlebar`, and everything in it that is not a
 * control is a handle. The stretch between the content and the actions is
 * there for that alone: a window with twenty documents open would otherwise
 * have no bar left to drag by, so it never shrinks below a minimum, and the
 * content scrolls instead.
 *
 * **`center` is held at the window's centre, not at the centre of what is
 * left.** A search box placed between the two ends of a flex row moves every
 * time a tab opens, and a thing the hand reaches for without looking has to
 * stay where it was. So with a centre the bar is three columns - two equal
 * sides around the middle - and each side keeps a handle of its own, so the
 * bar can still be grabbed on either side of the search. Without one the bar
 * stays a single row, and the window's own content keeps the whole width it
 * had. */
export function TitleBar({ labels, mark, children, actions, center, className }: TitleBarProps) {
  const gestures = useTitleBarGestures()
  const bar = 'h-titlebar shrink-0 items-stretch border-b border-line bg-raise select-none'

  const start = (
    <>
      {mark && <span className="flex shrink-0 items-center pr-2 pl-3">{mark}</span>}
      <div className="flex min-w-0 items-stretch">{children}</div>
    </>
  )
  const end = (
    <>
      {actions && <div className="flex shrink-0 items-center gap-1 px-1">{actions}</div>}
      <WindowButtons labels={labels} />
    </>
  )

  if (center === undefined) {
    return (
      <header className={cn('flex', bar, className)} {...gestures}>
        {start}
        <div aria-hidden data-titlebar-handle className="min-w-4 flex-1" />
        {end}
      </header>
    )
  }

  return (
    <header className={cn('grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)]', bar, className)} {...gestures}>
      <div className="flex min-w-0 items-stretch">
        {start}
        <div aria-hidden data-titlebar-handle className="min-w-4 flex-1" />
      </div>
      <div data-titlebar-center className="flex items-center">
        {center}
      </div>
      <div className="flex min-w-0 items-stretch">
        <div aria-hidden data-titlebar-handle className="min-w-4 flex-1" />
        {end}
      </div>
    </header>
  )
}
