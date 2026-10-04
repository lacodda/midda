# 0008 — The NTFS format is read in the core

Date: 2026-10-04
Status: accepted (amends ADR 0006)

## Context

ADR 0006 streams the Master File Table and uses `ntfs-reader` for the format
only: the boot sector, the map of where `$MFT` lives, and the attributes inside
a record. What midda takes from a record is narrow — names and their parents,
the write time and attribute bits, the reparse tag, and both sizes of the
unnamed `$DATA` stream and of `WofCompressedData`.

`ntfs-reader` 0.5 made its modules private, and 0.6 reports a stream by its
logical size alone. The attribute header — where a compressed or sparse stream
keeps the clusters it really holds, the number `AllocationSize` reports and
midda shows — is no longer reachable. Version 0.6 also adds a streamed scan of
its own, but it reads the table twice and still sizes streams logically.

## Decision

midda reads the format itself (`crates/midda-core/src/mft/format.rs`): the boot
sector's geometry (including clusters above 64 KiB), the record header, the
update sequence fix-up, the attribute walk, `$STANDARD_INFORMATION`,
`$FILE_NAME`, `$REPARSE_POINT`, the sizes of a `$DATA` stream, and the run list
of `$MFT`. Every field is read by offset from a byte slice and bounds-checked;
nothing is cast to a packed struct, so the module needs no `unsafe`. It is pure
and builds on every platform; its tests write records byte by byte and read
them back.

`ntfs-reader` is no longer a dependency.

## Consequences

Positive:

- The number the product exists to show does not depend on another crate's
  idea of its public surface.
- One fewer dependency, and with it the `windows` crate it pulled in beside
  `windows-sys`.
- The format code is tested on the Linux runner as well as on Windows.

Negative:

- About four hundred lines to own, including their tests. The structures have
  not changed since NTFS 3.1 (Windows XP), so they are unlikely to move.
- A table whose own map continues in another record is still not followed: the
  walk reads such a volume and says why, as before.

Held to: `tests/scanners_agree.rs`, which compares the MFT tree with the walk
entry for entry on the elevated CI runner, on a fixture and on the checkout.
