import React from 'react'
import ReactDOM from 'react-dom/client'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { overlayScrollbars } from 'dowel-ui'
import App from '@/App'
import '@/styles.css'

// The line's scrollbar: nothing at rest, a thin line while scrolling, a wider
// thumb under the pointer, and no width taken from the content. The engine
// draws it once `OverlayScrollbar` is on its command line (tauri.conf.json,
// `additionalBrowserArgs`); this wakes the thumb when the pointer comes near.
// https://lacodda.github.io/dowel/concepts/scrollbars/
overlayScrollbars()

// The window is created hidden (tauri.conf.json) and shown from here, once the
// document has painted its splash in the right colours. Shown before that, the
// webview flashes white for as long as the bundle takes to arrive - kilna's
// lesson, taken here before midda had the flash. If this line never runs, the
// core shows the window itself after a few seconds (lib.rs). Outside Tauri
// (vitest, a browser tab) there is no window to show, and that is fine.
getCurrentWindow()
  .show()
  .catch(() => undefined)

/*
 * The webview's own right-click menu does not belong in a desktop window.
 *
 * "Back", "Reload", "Save as", "Print", "Inspect" are a browser's answers, and
 * midda is not a browser: none of them mean anything over a folder, and Reload
 * throws away a scan that took minutes. With the system frame gone it would
 * also be what a right-click on the title bar opens.
 *
 * Not everywhere, though: inside a text field the menu is the only way to cut,
 * copy and paste with the mouse, and a selection someone has just made is made
 * to be copied. Everywhere else the gesture is the application's, and a
 * component that wants it takes it with its own handler. The same rule as
 * kilna's.
 */
document.addEventListener('contextmenu', (event) => {
  const target = event.target
  if (!(target instanceof Element)) return
  if (target.closest('input, textarea, [contenteditable=""], [contenteditable="true"]')) return
  if ((window.getSelection()?.toString() ?? '') !== '') return
  event.preventDefault()
})

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
