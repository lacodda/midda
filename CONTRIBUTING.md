# Contributing to midda

## Building

midda is a Cargo workspace and a Vite frontend. The engine is `midda-core`; the
window is `src-tauri`, a thin door onto it.

```
pnpm install
pnpm tauri dev
```

The gate, which has to be green before anything is committed:

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
pnpm lint
```

Requires Rust 1.90 or newer — the number in the workspace manifest is measured
against the crates actually resolved, and a CI job builds on it so it cannot
silently rot.

The brand assets are generated, not edited: `lyrn assets` draws the `.ico`,
the PNGs, the desktop icons and the docs copies from the three master SVGs.
`src-tauri/tests/brand_assets.rs` reads each image in the `.ico` back and holds
it to the line's rule about which master a given size comes from.

## Principles

**It always works.** Missing administrator rights slows the scan down; it never
blocks it.

**The number is what the volume gives back.** Every entry carries both its
logical and its allocated size, and the allocated one is what is shown. A tool
that reports the logical size is answering a question nobody asked it.

**Nothing is deleted without asking.** The tool shows and explains; the decision
is the user's, and the recycle bin is the default.

**Every rule justifies itself.** A finding without an explanation of what
regenerates it is not a finding.

Architecture decisions are recorded in [`docs/adr/`](docs/adr/).

## Adding a rule

The rules midda ships are files in `crates/midda-core/rules/`, in the format a
reader's own are written in ([docs/rules.md](docs/rules.md)). A rule that would
help anyone - a tool's cache, a build folder midda does not know - is a pull
request that adds it there:

1. Put it in the file it belongs to: `dev.toml` for what a developer's tools
   make again, `system.toml` for what Windows, a browser and downloads leave
   behind, `places.toml` for what something is without saying it can go. A
   rule that names something by what it is, not where, goes after the ones
   that name it by where: the first rule that names an entry is the one that
   applies.
2. Say why it exists in a comment, and in `explain` what is lost: one or two
   sentences to the reader, ending with a full stop. When in doubt, the risk
   is `caution` - a false "safe" costs someone their data.
3. Add a test beside the others (`src/rules/system_tests.rs`,
   `src/rules/own_tests.rs`) that builds a small tree, finds what the rule
   names, and shows what it must not name. A test that would still pass with
   the rule's condition removed is not yet the test.
4. A new file in `rules/` is shipped once `SHIPPED` in `src/rules/mod.rs`
   names it, in the order it is tried; a test fails until it does.
