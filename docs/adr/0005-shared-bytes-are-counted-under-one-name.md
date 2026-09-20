# 5. Shared bytes are counted under one name

Date: 2026-09-20

## Status

Accepted

## Context

A hard link is not a pointer to a file. It is a *name* for one, and a file with
three names is three directory entries over one set of bytes. Deleting one name
frees nothing; the bytes go when the last name does.

A scanner that measures each name and adds the results up therefore reports the
payload once per name. Measured on an NTFS volume on 2026-09-20: one 50 000-byte
file under three names, each reporting 53 248 bytes allocated, for a naive total
of 159 744 bytes over 53 248 bytes of disk. This is not a rare shape. It is what
a package manager's content-addressed store looks like — pnpm, cargo, nix — and
on a developer's machine it is tens of gigabytes.

The failure is invisible, which is what makes it worth an ADR: every individual
number the scanner printed was correct, the totals are internally consistent, and
nothing looks wrong. It is simply false.

Windows will say so for free. `FILE_STANDARD_INFO` — which midda already reads for
`AllocationSize` — carries `NumberOfLinks` in the same struct, so noticing that a
file has several names costs no extra call. Saying *which* names are the same
bytes needs one more query through the handle that is already open,
`FILE_ID_INFO`, giving the volume serial and the 128-bit file id.

The question this decision settles is not how to detect sharing. It is what to do
about it.

## Decision

**The bytes are charged to one name, and the other names are marked as what they
are.**

The owner is the **first name in node order**. The walk reads the tree in levels,
breadth first, so a lower node id means a shallower name, and within one level it
means earlier in the volume's own listing. The rule is therefore statable in
words — *the shallowest name, and among equals the first the volume lists* — and
identical across two scans of the same disk.

Deduplication runs **after the walk and before the roll-up**, as a pass of its own
over the arena.

The fact that a subtree contains a second name **rolls upward** onto the
directories above it, and it is the only trait that does.

## Consequences

### Why one name rather than none

Charging the bytes to nobody keeps every folder's number defensible in isolation
and breaks the one promise that matters: that the folders under a volume add up to
the volume. That sum is checked byte for byte against Windows itself, and it is
the product's whole claim. So the bytes stay somewhere, and the reader is told
where.

### Why node order rather than "first seen"

"The first name the walk reaches" was the obvious cheap answer and it is wrong.
The walk runs on a work-stealing pool (ADR 0001), so which name a thread reaches
first is a race, and two scans of one disk would disagree about which folder is
the large one. That is worse than being unable to say at all — it is the line's
own rule that a wrong pick beats no pick only when the pick is stated.

Note what this rule does *not* promise. Deterministic is not predictable: `read_dir`
on NTFS returns a directory in the volume's index order, not in creation order, so
which of two sibling folders ends up holding the payload cannot be guessed in
advance. Measured on 2026-09-20: a fixture that created `store` before `project`
had the walk return `project` first. The rule is safe to rely on across scans and
must not be described to a reader as "the first one you made".

### Why after the walk rather than inside it

Recognising a duplicate needs a set shared by every thread. A lock around that set
is a lock on the whole scan — the one thing the parallel walk exists to avoid. A
separate pass over the finished arena is single-threaded, linear, and touches only
the handful of nodes whose link count is above one.

### Why before the roll-up

The pass rewrites leaf sizes. A roll-up that had already run would leave every
ancestor holding the doubled total while the leaves read correctly — the worst of
both, and a state no later pass can repair. Held by a test rather than by a
comment.

### Why an unknown identity is not a match

A volume may refuse to report a file id. Two refusals are not evidence of
sharing, and treating them as equal would zero out a real file's bytes. Unknown
is kept distinct from one-name throughout: `Option<u32>` and `Option<FileIdentity>`
rather than defaults. This is the direction the product must never err in — a
false "you can delete this" costs the reader data.

The volume serial is part of the identity for the same reason. A scan can cross a
mount point, and two files on different volumes may share a file id without
sharing a byte.

### Why the fact travels up to folders, and only this fact

A folder holding nothing but second names reports `0 B`. On screen — seen during
the live run of 2026-09-20, not reasoned about — that is the most alarming row a
disk analyzer can draw: a folder the reader can see is full, rendered as empty,
with nothing said. So a directory inherits *"something beneath me is named
elsewhere"* and can account for itself.

Nothing else rolls up. "Compressed" and "sparse" are properties of a file and say
nothing true about the folder above it, whereas "there are shared bytes in here"
is precisely a statement about the folder.

### What the reader is told

Three places, strongest claim first:

- the second name carries a badge and, on hover, *"deleting this frees nothing"*;
- the folder above it says its contents are named elsewhere and counted there;
- the scan as a whole reports how many entries were second names and how many
  bytes were not counted twice — so a reader comparing midda against a tool that
  reports the larger number can see which of the two is explaining itself.

### What this does not do

It does not deduplicate *identical content* — two real copies of the same bytes
are two copies, and deleting one frees its own. Only entries the filesystem itself
says are one file are treated as one file.
