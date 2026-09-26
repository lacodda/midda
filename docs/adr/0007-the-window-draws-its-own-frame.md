# 0007 — The window draws its own frame, and opens on a splash

Date: 2026-09-26
Status: accepted

## Context

Until v0.6.0 midda was the one Tauri product of the lacodda line that still had
the system's title bar, with an application bar under it holding the mark, the
actions and the size switch, and a third strip under that holding the trail and
the total. Three strips before the first row of a table that exists to be read.
kilna and scheda draw their own frame, and dowel ships the parts as registry
components (`window-frame`, `splash`, `breadcrumbs`) since 0.29.

A frameless window takes on what the system did for free: dragging, the three
buttons, resizing from the edges, and not being white before the page paints.

## Decision

**One bar.** `decorations: false`; dowel's `TitleBar` holds the mark, the trail
from the scan root to what is on screen, and the window's actions (Accelerate,
the On disk / Size switch, Choose a folder / Stop), then the window buttons.
`ResizeEdges` restores the border. The total that used to sit beside the trail
joins the notes strip under the bar, so a scan result stands under two strips
rather than three.

**Hidden until painted.** The window is created hidden on the theme's dark
background and the page shows it once the static splash has painted. If the page
never gets that far, the core shows the window after three seconds: a window
that appears late is a better failure than a process with no window.

**A splash in two halves, from one source.** `index.html` paints the splash in
inline CSS before the bundle; `Splash.tsx` takes over with the same geometry.
The static half's colours come from dowel's resolved palette for midda, its mark
from the line's masters (`dowel-ui/marks`), its words from `src/splash.json`,
which the React half reads too — all filled in at build time by a Vite plugin
that fails on a name it does not know and on a value the page leaves out. kilna
writes its splash's colours and mark by hand, and nothing holds them to the
theme.

**Compact density for the whole window**, as in kilna: controls stand on 28px
rows for the small size, which is what midda's buttons were before dowel 0.32.

**Registry copies are held to the registry.** `tools/check-registry.mjs`, in
`pnpm lint`, fails on any file in `src/components/ui` that differs from the
installed dowel-ui or has no twin in it.

## Consequences

Positive:

- The system title bar's strip is the application's; a result stands under two
  strips instead of three.
- The splash cannot drift from the theme, the mark or its own React half.
- A dowel upgrade that is not followed by re-copying fails the gate.

Negative:

- Window behaviour the system gave for free is now ours to keep working: drag,
  double-click to maximise, edges, the buttons. The parts are dowel's and shared
  with kilna, so a fix lands once.
- The right-click menu of the webview is turned off outside text fields, so a
  right-click on the bar opens nothing rather than "Back / Reload / Inspect".
