# 0003 — Every entry carries both sizes, and the occupied one is the default

Date: 2026-09-18
Status: accepted

## Context

A file has two sizes. The *logical* size is how many bytes a program reads out
of it. The *allocated* size is how much of the volume it holds. They are rarely
equal:

- a 1-byte file occupies a whole cluster — or, on NTFS, 8 bytes inside its own
  MFT record, because small files are stored resident;
- an NTFS-compressed file occupies less than it reads;
- a sparse file — a VHDX, a database — can read as 100 GB and occupy 12;
- a cloud placeholder reads as its full size and occupies almost nothing.

Most disk analyzers show the logical size, because it is the one
`std::fs::Metadata` and every `stat` equivalent hands over for free. That makes
them wrong in the direction that matters here: a user who opens a disk analyzer
came to free space, and freeing a sparse 100 GB file returns 12.

Storing only one of the two is cheap now and expensive later — answering the
other question would mean a second full scan of the volume.

## Decision

`Size` holds both numbers, and every node in the tree carries one. Folder totals
sum both.

**The allocated size is the default** everywhere: sorting, folder totals, the
treemap's areas, the number in the toolbar. The logical size is a switch in the
window, never a setting to configure.

On Windows the allocated size comes from `GetFileInformationByHandleEx` with
`FileStandardInfo`, which reports `AllocationSize`. On other platforms it is
`st_blocks × 512`, which is the same answer. Where neither is available it falls
back to rounding the logical size up to the cluster size — an approximation that
is announced as such in the code rather than presented as measured.

## Consequences

Positive:

- The number midda shows is the number the volume gives back, which is the one
  promise the product is built on.
- The question "why does this folder read as 30 bytes and cost 8 KB" is
  answerable without rescanning, and becomes a visible explanation in the window.
- Sparse and compressed files — the cases naive scanners get most wrong — are
  right from the first version, not retrofitted.

Negative:

- Sixteen bytes per node instead of eight. On a five-million-entry volume that
  is 40 MB more, which is the right trade for an index held in memory.
- One extra syscall per file on Windows: the allocated size needs a handle,
  where the logical size comes with the directory entry. This is the walk
  scanner's cost, and the MFT scanner of v0.5.0 does not pay it — the MFT record
  holds both numbers already.

Rejected alternatives:

- **Logical only, allocated added later** — half the memory and half the
  syscalls. Rejected because "later" means a second scan of the volume, and
  because every rule, total and treemap written in between would have been
  written against the wrong number.
- **Allocated only** — one number, no switch. Rejected because the logical size
  is the right answer to "will this fit on that drive", which is a real question,
  and because it is free once the handle is open.
- **`GetCompressedFileSizeW` for the allocated size** — the obvious-looking API,
  and wrong: for an ordinary file it returns the logical size unrounded. It
  agrees with `AllocationSize` only on compressed and sparse files, which is
  exactly the fixture a test would have used. Measured, not assumed; the numbers
  are in `crates/midda-core/src/platform.rs`.
