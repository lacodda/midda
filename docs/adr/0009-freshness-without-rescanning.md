# 0009 — Freshness: one way to apply a change, three ways to hear of one

Date: 2026-10-04
Status: accepted

## Context

Until v0.7 a scan was a picture of a moment: the window showed the disk as it
was when the walk or the MFT read finished, and the only way to see it again
was to read it again — six minutes for a walk of a system volume. The vision
promises the opposite: open midda and the picture is already current.

The owner settled the shape on 2026-09-11. Freshness comes in steps, as far as
the process's rights allow: without elevation, a watcher while the window is
open and a rescan on start; with elevation, the volume's change journal (USN);
later, a service that holds the index for good (v0.21). Three questions were
open:

- **How a change reaches the tree.** A watcher reports paths, with an action
  per path that Windows documents loosely (a rename is two events, a save is
  several, a name may come in its DOS short form). The journal reports file
  references and reasons, many records per operation. Applying either
  literally means two interpreters of two vocabularies, each wrong in its own
  corner cases.
- **What the arena does under change.** The tree is a flat arena in scan
  order; the order decides who keeps shared bytes (ADR 0005), and the window
  holds ids into it.
- **What is saved between runs, and in what form.**

## Decision

**A change is a path to look at again.** Whatever reports it — watcher or
journal — is reduced to a set of paths, and each path is looked at on the
volume *now*: a file is measured as the walk measures it, a folder that
appeared is read whole and grafted in, one that went is taken out, a folder
whose identity changed under the same name is read again. The reports are
never trusted for what they say changed, only for where to look. One
interpreter, the volume's own answer, and correct for anything the volume
can be asked about. The step is split in two — looking (reads the disk, under
a read lock) and applying (holds the tree briefly) — so the window keeps
drawing meanwhile.

**Ids stay put; the arena is laid out again only when saved.** A node that
goes leaves a marked slot; one that arrives is appended and inserted into its
parent's listing. A parent still precedes its children, so totals still roll
up, and the totals above a change are recomputed from children rather than
nudged by differences. Shared bytes are settled per file by the scan order —
shallowest name, then path in listing order — stated without the arena, so a
tree kept up to date charges the same name a fresh scan would. Directories now
carry their identity in both scanners; it is what tells a folder written to
from one replaced by another of the same name.

**The proof is equality with a fresh scan.** Every update test changes a
folder on disk, applies the reported paths, lays the arena out again and
compares it with a fresh scan node for node, with the scanners' own
comparison. Mutating each key rule — other names of a linked file not
re-measured, folder identity ignored, parent times not refreshed, owner not by
scan order, listing order not kept, a new branch not rolled up — fails a named
test.

**The watcher** is `ReadDirectoryChangesW` on the scanned folder, subtree
included, on a thread of its own, with a 1 MiB buffer (64 KiB on a share).
`start` returns only once its first read is in place. A buffer overflow is
reported as lost track, and the folder is read again under the picture.

**The journal** is read with `FSCTL_READ_USN_JOURNAL` from a saved position;
paths come from opening each record's folder by id. A position the ring has
come round past, or one from a journal made again, is a gap, and the folder is
read again. The same records answer "what was written lately": a tree of the
files created or written since a moment, at what they hold now, each marked
new or written to. The journal does not record how much a file grew, so the
window says the sizes are what files hold now.

**The index is saved** when a read finishes, every five minutes while it
changes, and when the window closes. It is written into memory under the lock
— 170 ms for 715 000 entries — and put on disk by a thread of its own: on a
live run, flushing 31 MB and having it read through by an antivirus held the
keeper for ten seconds while a change waited. The format is midda's own
(version 1):
nodes in scan order with only what a scan measures — totals, entry counts and
subtree times are worked out again on reading — and a trailer that refuses a
file cut short. Another version is replaced, never migrated. A folder made
again at the same path is told from the saved one by identity.

**The window** polls one state with two numbers: the version of what is on
screen, which moves with every change, and the arena of its ids, which moves
when the tree is read again. Ids from an older arena are refused as stale; the
window finds its place again by path. The title bar says how current the
picture is and what keeps it so: `fresh · USN`, `watching`, `saved 2 h ago ·
reading again`, `catching up · USN`, `scanned at 14:02`.

## Consequences

Positive:

- One code path applies every change, and its correctness is a property of the
  whole tree, tested against the scanner rather than against expectations.
- An elevated window opens on the saved index and is current in seconds, with
  everything that changed while midda was closed applied.
- The saved index costs about 45 bytes an entry — 31.6 MB for the 715 000
  entries of a developer's projects folder, read back in 0.4 s — some 130 MB
  for a system volume of three million.
- A change reaches the window in under a second: measured on that folder, a
  new 30 MB file in the total after 0.43 s and on screen after 0.7 s.

Negative:

- A renamed folder is read again rather than moved: correct, and slower for a
  large one.
- Without elevation the start still reads the folder again; the saved index
  only fills the screen meanwhile.
- A path the process cannot look at — another user's profile, `System Volume
  Information` — is left as the scan read it.

Rejected alternatives:

- **Applying the events as reported.** Two vocabularies, each with corner
  cases (renames in two halves, short names, case-only renames, overflow), and
  no way to test against the volume.
- **Re-laying out the arena after every batch.** Keeps scan order at all times,
  at the cost of a pass over millions of nodes every second on a busy volume,
  and renumbers every id the window holds.
- **`ntfs-reader`'s journal reader.** It came with version 0.6, which hides
  the attribute headers the MFT reader needs (ADR 0008); two NTFS stacks for
  one volume were not worth one module.
- **serde with a binary codec for the saved index.** Stores the derived
  totals along with the measurements, and makes the format whatever the
  structs serialise to; the format is to be frozen before 1.0.
