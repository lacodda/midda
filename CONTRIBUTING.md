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

Requires Rust 1.88 or newer — the number in the workspace manifest is measured
against the crates actually resolved, and a CI job builds on it so it cannot
silently rot.

The brand assets are generated, not edited: `node docs/export-assets.mjs`
rewrites the `.ico`, the PNGs, the desktop icons and the docs copies from the
three master SVGs. `src-tauri/tests/brand_assets.rs` holds it to the line's rule
about which master a given size comes from.

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
