# 4. The treemap is laid out in the core, in fractions

Date: 2026-09-19

## Status

Accepted

## Context

v0.3.0 draws a folder as a treemap: one rectangle per child, area proportional
to what it occupies. Something has to turn a list of children into a list of
rectangles, and there were two places it could live.

**In the window**, as a TypeScript module beside `windowFor`. The canvas is
there, the pixel sizes are there, and the arithmetic would be testable without
a browser in exactly the way `windowFor` is.

**In the core**, as a Rust module beside `order`. The children, their two sizes
and the rule about which of them come first are already there.

Two facts constrain the choice.

The first is volume. A folder on a system volume holds hundreds of thousands of
children — the scan of this machine's `C:` found 2.4 million entries, with
945,796 under one directory. Whoever decides which of them are too small to draw
needs their areas; whoever does not decide it has to be sent all of them.
Serializing a hundred thousand children so the window can discard 99% of them is
megabytes across the bridge to draw two hundred rectangles.

The second is that a treemap is not the only door. A CLI and the MCP door of
v0.19 will want the same picture — as text, as an exported PNG — and a layout
implemented in the window is one they cannot reach.

The argument for the window was that layout is about pixels. It is not: it is
about *proportions*. Only two things about the window matter to it, and both
are a single number.

## Decision

`midda_core::treemap` lays the rectangles out, in **fractions of the box** —
`0.0..=1.0` on both axes. The window multiplies by whatever size it happens to
be, and a resize does not need to ask again unless the shape changed.

The window passes two numbers it alone knows:

- `aspect` — its width over its height. Squaring needs it: a rectangle that is
  square in fraction space is a 3:1 sliver in a box three times as wide as it is
  tall, and the row-breaking decision has to see what the reader sees.
- `min_area` — the smallest fraction worth its own tile, which the window gets
  by dividing its own smallest useful rectangle (24×24 px) by its area.

The core returns tiles largest-first, which is both what squarified layout needs
and the order the table already shows by default — so the two views agree about
which thing is the big one.

Everything under `min_area`, and everything past `max_tiles`, is gathered into
one final tile with `id: None` that carries the summed bytes and the count of
what it stands for. It is not dropped: a picture that silently omitted the tail
would not add up to the folder it claims to show.

The algorithm is squarified layout (Bruls, Huizing, van Wijk, 2000) rather than
slice-and-dice. Slice-and-dice is four lines and produces tiles a reader cannot
hit with a pointer; avoiding that is the entire reason the paper exists.

What stays in the window is everything between a fraction and a pixel:
`src/treemap-layout.ts` — where a tile lands at a given size, which tile is
under the pointer, which token a category paints in, and which tile an arrow key
moves to. That is kept out of the component for the reason `windowFor` is: a
canvas cannot be asked what it drew, so the rules have to be plain functions
over numbers to be checkable at all.

## Consequences

**Good.** One implementation of the picture, reachable by every door. The
bridge carries a few hundred tiles rather than a folder. The cut-off is made
where the areas are known. Both sizes are already here, so switching basis is a
different layout rather than a different renderer.

**Bad.** The window cannot re-lay-out during a drag-resize without a round trip;
each resize is one `invoke`. Measured against a scan that takes minutes, an
`invoke` that takes microseconds is not the cost worth optimising, and the
fractions mean a *scale* change needs no call at all.

**Watch.** Two places decide which way a row runs — the one that judges whether
a row is still improving, and the one that lays it out. When they disagree the
layout stays valid by every property a test checks (exact areas, no overlaps,
the box filled) and silently stops being square. `the_row_that_is_judged_is_the_row_that_is_placed`
holds them together; it needs an uneven distribution in a non-square box, because
on equal values in a square the two are symmetric.
