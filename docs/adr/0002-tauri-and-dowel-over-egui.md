# 0002 — A Tauri window over the line's design system, not an immediate-mode one

Date: 2026-09-18
Status: accepted
Supersedes the GUI half of the 2026-08-09 technology choice

## Context

midda was founded with `egui` as its window: immediate mode, one Rust binary, no
web stack, and a redraw model that suits a treemap being dragged around.

Two things have changed since that choice was made.

The line now has a design system. `dowel` ships a token vocabulary, seventy-odd
primitives built on Base UI, a registry the products install from, and gates that
hold contrast and keyboard behaviour. It did not exist in August. Three products
of the line already run on Tauri v2 — kilna, scheda, furca — and share that
vocabulary, so a fourth written in `egui` would be the only one whose buttons,
tables, empty states and focus rings are its own.

And the screens midda actually needs are known now, from the mock-up approved in
August: an overview, a table of a million rows, a treemap, and a layers view. Of
those, one wants a canvas and three want a table with virtualization, sorting,
sticky headers and a breadcrumb — which is the part `egui` makes hardest and the
part `dowel` already has.

## Decision

The window is Tauri v2 with a React frontend on `dowel`.

The engine is a plain Rust crate, `midda-core`, which knows nothing about a
window: it scans and returns a tree. The desktop app is a thin door onto it.

The treemap, when it arrives in v0.3.0, is drawn on a `<canvas>` — the one place
retained-mode DOM would be the wrong tool, and the one place the browser gives
exactly the same primitive `egui` would have.

## Consequences

Positive:

- midda looks like the rest of the line without anyone maintaining a second
  interpretation of the line's design.
- The table that three of four screens are built on is a solved problem rather
  than the project's main engineering risk.
- `midda-core` is reusable by a CLI and by the MCP door of v0.19 without either
  dragging a GUI toolkit in.
- The frontend is testable without a window: the formatting rules — which are
  the product's argument about what a byte means — run under `vitest`.

Negative:

- A WebView2 dependency and a two-language build, where there was one binary.
- A million-row table needs virtualization to stay smooth, which immediate mode
  would have given for free.
- The bridge between core and window is serialization, so the tree cannot simply
  be handed over; rows are requested a level at a time.

Rejected alternatives:

- **Stay on `egui`** — one language, one binary, and a redraw model that fits a
  treemap. Rejected because it makes the product an outlier in the line and puts
  the table, not the scanning, at the centre of the work.
- **Tauri with hand-written CSS** — avoids the registry. Rejected for the same
  reason the design system exists: the second interpretation of a colour is the
  one that drifts.
