# 0010 — Snapshots are saved indexes; a comparison is a tree of what differs

Date: 2026-10-08
Status: accepted

## Context

A picture kept current (ADR 0009) says what a folder holds now. It cannot
say what grew: the change journal records that a file was written, not how
large it was before, and the view of what was written lately says so in as
many words. v0.8 is to answer the two questions that need a second moment:

- **What grew since last time** — in the table and in a picture where growth
  shows red;
- **Before and after a cleanup** — two pictures, one page: how much came
  back, and from where.

Three things were open: what a snapshot is and what keeps one; what comparing
two of them produces; and what the picture of a comparison is drawn by.

## Decision

**A snapshot is a saved index nobody brings up to date.** The same file in the
same format (`fresh/store`, version 1), kept in a folder of its own per
scanned folder: one reader, one writer, and one format to freeze before 1.0.
Two kinds:

- *Last time.* Opening a folder reads the index saved when it was last
  closed; before anything brings it up to date, that file is kept as the
  picture of last time — by a hard link, which costs nothing on disk until the
  index is next saved (the save replaces the file by a rename, so the link
  keeps the old bytes). One per folder, replaced at every open.
- *Taken.* Asked for by the reader — before a cleanup, say — and kept until
  they delete it.

Nothing else is kept unasked. A snapshot weighs what the index weighs, some
45 bytes an entry, 130 MB for a system volume of three million; a disk
analyzer that kept a week of those on its own would be its own best finding.
The window lists the snapshots of the open folder with what they take on
disk, and deletes one on request. A snapshot from another version of the
format is listed with the reason and cannot be compared.

**A comparison is a tree of what differs.** Two trees are matched by name
folder by folder from the root — both hold each folder's children in listing
order, so a folder is one merge of two sorted lists. What comes out is every
entry whose size is not what it was, with the folders above it and nothing
else: an entry that went is in it at zero now, one that came at nothing then,
a file and a folder of one name are one entry, replaced. A folder carries its
whole size at both moments, not the sum of what is shown under it; its growth
is therefore exact, and also exactly the sum of the growth shown under it,
because what stayed the same grew by nothing. Sizes are compared, not times: a
file saved again at the same size freed nothing and took nothing.

Each entry carries three numbers on each size: what it held then, what it
holds now, and how much *moved* under it — grown plus freed. Growth (now less
then) is what the table sorts by and shows. Moved is what the picture is drawn
by: a tile is as large as what changed there, and coloured by which way it
went on balance — growth red (`--bad`), freed green (`--good`). Moved adds up
from children to parent, so a folder's tile and the tiles inside it agree; net
growth does not, and a folder where five gigabytes went and five came would be
an invisible tile with large tiles inside it.

**Places are where the report says it came from.** A folder that went whole is
one place — `node_modules`, gone — not two hundred thousand files; one that
came whole likewise; files that changed loose in a folder are that folder's
place. Every byte that moved belongs to exactly one place, so the places add up
to the totals, and a report can say "from here" without counting anything
twice.

**The window** shows a comparison as a third tree beside the folder and the
journal's view of what was written: the same table (before, now, change on the
size chosen) and the same picture, drawn by what moved. A comparison with the
picture of last time is one press — "Grown" in the control that already
chooses between everything and what was written lately. Taking a snapshot,
comparing any two pictures and deleting one live in a popover beside it. The
report of a comparison is a page of its own: what came back, what was written
meanwhile, and the places each came from, with a copy as Markdown to paste
into a note.

## Consequences

Positive:

- Growth is exact, both ways, at every depth, and the window needs no
  elevation for it: the picture of last time is the saved index, which every
  window has.
- Comparing is cheap. Measured on the saved index of a developer's projects
  folder (715 841 entries): reading the snapshot back 0.27 s, comparing it
  with itself — a full merge that finds nothing — 23 ms, comparing after a
  31 000-entry folder of 143 GB went 27 ms, the places in microseconds.
- The comparison, its order and its picture are core functions, so the CLI and
  the MCP door get the same answers.

Negative:

- A comparison reads the older picture into memory whole, for as long as the
  comparison is computed: on a system volume, for a second or two, as much
  memory again as the open index.
- The picture of last time is replaced at every open, so a folder opened twice
  in an hour compares with an hour ago. A baseline that should stay is a taken
  snapshot.
- Taken snapshots are never deleted without the reader, and each weighs an
  index.

Rejected alternatives:

- **Snapshots of folder totals only.** A tenth of the size, and no answer to
  which file grew; a second format to freeze.
- **Keeping a snapshot a day and one a week on its own**, as the mockup
  sketched ("auto-weekly"). Gigabytes of the reader's disk, unasked, on the
  volume they came to free.
- **Drawing a comparison by current size, coloured by growth.** Hides the
  question: six gigabytes of growth inside a 400 GB folder is a red sliver.
- **Drawing it by net growth.** Not additive, so a folder of balanced churn
  vanishes from the picture while what is inside it is large.
- **Growth from the change journal alone.** The journal records writes, not
  previous sizes (ADR 0009).
- **A streaming comparison over the snapshot file** without reading it into a
  tree: a quarter of the memory, and an optimisation that can come later
  behind the same output.
