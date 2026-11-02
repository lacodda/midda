# 0017 — The age of the layers, and space back without deleting

Date: 2026-11-02
Status: accepted

## Context

The product is named for a midden: a disk lies in layers, and under this
week's files are folders nothing has touched in years. Up to v0.13 the tree
knew each layer's age (`subtree_modified`, ADR 0001's roll-up) and the rules
used it (`older`, ADR 0016), but the window never said it: nothing showed
how old the bytes on a disk are, or which folders have turned to stone.

An old folder is often not rubbish. The photos of 2019, a finished project,
a game nobody plays: the reader would rather keep them, and the answer
"delete it" is wrong for them. Compressing them gives space back with
nothing lost.

Two facts were missing on the way. When a file was last opened is a second
fact beside when it was written, and the owner decided on 2026-09-11 that it
is shown apart from the age and only where the volume keeps it. And files
Windows compresses itself (`compact /exe`, a CompactOS system volume) read
as plain and occupy a fraction of their size, with nothing to say why.

## Decision

**One measure of age.** `layers::untouched` is how long ago anything in a
node was written, from `subtree_modified`; a time the volume did not give,
or one after now, is no age. The rules' `older` asks the same function. A
fossil is a node untouched for at least a year of 365 days (a year as the
rules count one) — one boundary, not a setting: the strata say what is
older still.

**The strata, the fossils and the view of what is untouched add up.** The
strata are a folder's bytes by the age of each file, in bands of a month,
half a year, a year, two, five and more, and undated. The fossils are the
largest nodes untouched for a year inside no larger one: a folder named
whole when nothing in it moved, its parts when it is still in use around
them. Because a folder is never older than its newest file, the files
untouched for a year are exactly the files inside the fossils, so the old
bands and the fossils are one count — a test holds them to it.
`layers::Untouched` is the folder by those bytes alone: the same tree and
ids, a size per node, worked out in one pass per version of the tree.

**In the window, beside Grown and Last week.** "Untouched" is a view of the
folder as Grown and Last week are: its default shape is the fossils, with
the strata above them; its other is the folder's table and treemap by old
bytes, each tile coloured by its stratum — a fresh violet that fades, then
the ochre of the ground, deeper the older. The colours are dowel's tokens
mixed with the ground, so they hold in both themes, at mixes measured to
keep every label at 4.6:1 or better.

**When a file was opened, where the volume keeps it.** Both scanners read a
file's last access time (`$STANDARD_INFORMATION` at 0x18; `FILE_BASIC_INFO`
through the handle the walk already opens), but only where the volume keeps
it: NTFS with bit 0 of `NtfsDisableLastAccessUpdate` clear — the bit is
"disabled" in all four forms of the value — and ReFS with
`RefsDisableLastAccessUpdate` zero. Elsewhere, and where the setting cannot
be read, there is no time: a time read there is when the file was made, and
"opened in 2019" that means nothing is worse than nothing. The tree records
the verdict (`Tree::access_kept`), the index carries it (format 2). A
folder's time is the newest of its files', never its own, which moves
whenever it is listed — by midda's scan too. midda's own reads — a
shortcut, an installer, a virtual disk — put the time back
(`platform::keeping_access_time`), or every launch would make them all look
opened today.

**NTFS compression, in place.** `compress` marks a file or folder
compressed with `FSCTL_SET_COMPRESSION` (LZNT1, what `compact /c` and
Explorer's checkbox do) and the folder so that what is put in it later is
compressed too, then the entry is read again to measure what came back. It
leaves as they are: what is compressed already, cloud placeholders, sparse
files, virtual disks (WSL and Hyper-V will not mount a compressed one),
formats compressed by themselves, and encrypted files; what the volume
refuses is counted and said. Windows' own files are not offered at all. A
read-only file is compressed as `compact` does it, with the attribute taken
off for the moment and put back. Measured: neither the write time nor the
access time of a compressed file moves, so a fossil compressed is still a
fossil.

**The overlay explains itself in both scanners.** A file the Windows
Overlay Filter keeps compressed is `COMPRESSED` when it occupies less than
it reads and nothing else says why — the MFT reader by its reparse tag, the
walk by asking `WofIsExternalFile` of exactly those files. One test on both
sides, so the scanners name the same files.

## Consequences

Positive:

- The view the product is named for exists, and its numbers add up three
  ways.
- An old folder has an answer between keeping it as it is and deleting it.
- A CompactOS volume's files say why they occupy a fifth of their size.

Negative:

- An access time in a saved index is as of the last time midda looked at
  the file: neither the change journal nor a watcher reports a read. For a
  boundary of a year that is days at most.
- The age is a time of writing: a file unpacked from an archive keeps the
  archive's time, and a folder an editor rewrites without changing anything
  looks young.
- Indexes and snapshots of format 1 are not read; the folder is read again,
  and an earlier snapshot can no longer be compared with.
- How much compression gives back is not known until it has run.

Rejected alternatives:

- **The overlay's compression (`compact /exe`) for "compress".** It packs
  tighter, but it is invisible in Explorer, undone by the first write, and
  kept behind a reparse point other tools read their own way.
- **A filter over the folder's own tree.** Hiding what is young hides the
  living folders the old ones are inside, and with them the way to them.
- **The access time as the age.** Most volumes do not keep it, and where
  they do, every indexer and antivirus scan moves it.
- **A folder's own access time.** It is the time it was last listed.
- **A setting for the boundary of a fossil.** The strata already say how
  old; a setting would make the reader choose before seeing anything.
- **Estimating what compression gives back.** A ratio is the files' to say;
  midda measures it.

## Measured

On the owner's computer (`NtfsDisableLastAccessUpdate` = 0x80000002: the
system keeps access times on a 1 TB volume). A folder of projects, 1.26
million entries: the strata take 50 ms, the fossils 41 ms (15,870 of them,
1 GB), the view of what is untouched 58 ms. Ordering ties among the fossils
by the order a scan would list them in took 266 ms of the fossils' time;
it is kept for the rows shown. A cold test folder compressed from 880 KB to
112 KB on disk, still a fossil afterwards. `compact` and midda's own
compression both left a file's write and access times as they were, and a
read-only file read-only.
