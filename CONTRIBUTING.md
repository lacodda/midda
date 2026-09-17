# Contributing to midda

## Building

```
cargo run --release
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

Requires Rust 1.85 or newer.

## Principles

**It always works.** Missing administrator rights slows the scan down; it never blocks it.

**Nothing is deleted without asking.** The tool shows and explains; the decision is the user's, and the recycle bin is the default.

**Every rule justifies itself.** A finding without an explanation of what regenerates it is not a finding.

Architecture decisions are recorded in [`docs/adr/`](docs/adr/).
