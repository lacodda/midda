<!-- Banner is added in v0.8.0 together with the brand assets. -->

# midda

**A disk space analyzer for Windows that answers the question you actually have.**

Existing tools tell you *what is large*. That is the easy half. Looking at 40 GB spread across project folders, the real question is *which of this comes back with one build command, and which is the only copy*. `midda` answers that one.

The name comes from *midden* — the archaeological refuse layer. A disk is stratified the same way: under this week's files sit fossilized `target/` and `node_modules` directories from projects abandoned years ago.

## What makes it different

**Speed.** Reading the NTFS Master File Table directly scans a full volume in seconds instead of minutes — one sequential read instead of a syscall per directory.

**Freshness without rescanning.** The USN change journal updates the index incrementally. Open the app and the picture is already current. Neither WizTree nor TreeSize does this.

**Domain knowledge.** Not "here are big folders" but "here are 40 GB that a build regenerates": `node_modules`, `target/`, `.next`, `__pycache__`, Docker layers, package manager caches. Every finding explains itself — what recreates it, what is lost.

## Privileges

Reading the MFT means opening the volume as a device, bypassing the filesystem. Windows grants that to administrators only — otherwise any program could sidestep file permissions.

So `midda` runs both ways: without elevation it walks the filesystem normally and works everywhere; an "accelerate" button requests elevation and switches to the MFT path. You choose.

## Status

Early development. The version map to 1.0 is fixed:

| Version | What lands |
| --- | --- |
| v0.1.0 | Filesystem walk, size tree, sortable table |
| v0.2.0 | Treemap visualization, drill-down |
| v0.3.0 | MFT mode behind UAC — seconds instead of minutes |
| v0.4.0 | USN Journal — incremental index updates |
| v0.5.0 | "Safe to delete" rules engine |
| v0.6.0 | Deletion with recycle bin and confirmation |
| v0.7.0 | Duplicate detection |
| v0.8.0 | Shell integration, installer, auto-update |
| v1.0.0 | Public release |

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

## License

MIT — see [LICENSE](LICENSE).
