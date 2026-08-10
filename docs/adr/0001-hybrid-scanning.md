# 0001 — Two scanners behind one trait: filesystem walk and MFT

Date: 2026-08-09
Status: accepted

## Context

A recursive filesystem walk issues a syscall per directory: open, read entries, close, recurse. On a system volume with a million files that is a million-plus calls, each with a permission check — minutes of wall-clock time.

The NTFS Master File Table holds a record for every file on the volume: name, size, timestamps, parent. Reading it as one sequential stream produces the same information in seconds.

Reading the MFT requires opening the volume as a device (`\\.\C:`), bypassing the filesystem layer. That is access below file-level permissions — metadata for files the user cannot open becomes visible. Windows restricts it to administrators, and every Rust crate in this space confirms the requirement.

Requiring elevation on every launch has two costs: a UAC prompt each time, and total failure on a managed machine where the user has no administrator rights.

## Decision

Both scanners implement a common `Scanner` trait.

Without elevation, `midda` walks the filesystem with a progress indicator. An explicit "accelerate" action requests elevation and switches to the MFT scanner. The user decides whether the speed is worth the prompt.

A test asserts that both scanners produce identical trees for the same directory.

## Consequences

Positive:

- The product runs on any machine, elevated or not.
- Users who want the speed get it, on request rather than by ambush.
- The walk-based scanner is portable, so a Linux/macOS port stays possible without redesign.

Negative:

- Two implementations to maintain and to keep in agreement.
- A UI state machine that has to express "currently slow, could be fast".

Rejected alternatives:

- **MFT only, always elevated** — one code path, but a UAC prompt on every launch and no way to run on a locked-down machine.
- **Walk only** — no elevation ever and trivially portable, but it discards the product's main advantage and leaves `midda` indistinguishable from existing free tools.
