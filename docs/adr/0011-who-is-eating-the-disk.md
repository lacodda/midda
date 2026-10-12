# 0011 — Who is eating the disk: hierarchical heavy hitters, and a pace

Date: 2026-10-12
Status: accepted

## Context

v0.8 made growth visible: a comparison of two pictures is a tree of what
differs, drawn by what moved (ADR 0010), and the change journal's view says
where recent bytes were written (ADR 0009). Both are trees, and a tree of a
volume's growth is still a question. "What is filling C:" wants a handful of
names with a number each, the numbers adding up — and "how long until it is
full" wants a pace.

Three things were open: which folders to name, how growth is measured for
naming them, and when a pace may be read off two pictures.

## Decision

**A grower is a folder that, apart from the growers named under it, grew by
at least a fiftieth of everything that grew.** This is how network traffic is
summarised by prefix — hierarchical heavy hitters — and it gives the list the
three properties it needs:

- *Specific.* A folder is named as deep as its growth is still one sizeable
  thing: `app\node_modules` when the dependencies are what grew, not `app`;
  `node_modules`, not each of twelve hundred packages none of which is
  sizeable alone.
- *Counted once.* A folder named under another is taken out of the other's
  share. Each line carries what it accounts for on its own; the lines and
  one more number — what grew in pieces too small to name, on balance — add
  up to the growth of the whole.
- *Short.* Every line accounts for a fiftieth, so there are about fifty at
  most whatever the size of the volume; the window shows twenty.

The threshold is a share, not a number of bytes: a day on a volume and an
afternoon in one project need different floors, and only a share scales.

**Growth is net.** What a folder holds now less what it held: a folder where
five gigabytes went and five came is where everything happened — the
comparison's picture shows it — and is not what is filling the disk. What was
freed is the same rule read the other way, as its own list.

**Files are never lines of their own.** Their growth belongs to their folder,
so a downloads folder that took four installers is one line, four files. The
root is never a line — it is the whole — but its own files are named apart
when they are sizeable: on a system volume they are the page file and the
hibernation file, and without a line of their own they would vanish into the
rest.

**The same rule reads the journal.** In the view of what was written lately
the value of a folder is what the files written to hold now. The list says
"where the most was written", never "what grew": the journal does not record
a size before.

**A pace needs a day.** Growth over a span is so much a day, and against the
volume's free space, how long that lasts. Only for pictures at least a day
apart — a pace read off an hour, one build in it, multiplied by twenty-four,
is noise stated as a forecast — and only when the later picture is now, since
free space is a fact about the present. The forecast is on disk whatever size
is shown, because free space is clusters, and it says so when the folder is
not the whole volume: it is that folder's pace alone.

**The window** opens a comparison and the journal's view on this list
("Top"), beside the tree and — for a comparison — the report. Each line opens
its folder in the tree.

## Consequences

Positive:

- One function in the core (`growth::growers`) answers for the window now,
  and for the CLI, the MCP door and the service's weekly toast later.
- Every line explains itself in one sentence, and the arithmetic is visible:
  the lines and the rest add up to the net growth on top of the page.
- No new data is kept: the list is computed from the comparison or the
  journal view already in memory.

Negative:

- The shape of a tree cannot tell that `target\debug` and `target\release`
  are one build and `data` and `target` are two things, so a large build
  folder can come out as two or three lines. The function takes a predicate
  for folders known to be one thing; the rules of v0.10, which know
  `node_modules` and `target`, will supply it.
- With the picture of last time replaced at every open, "since last time" may
  be an hour, and then there is no pace. A baseline that should stay is a
  taken snapshot (ADR 0010).

Rejected alternatives:

- **Descending while one child holds most of the growth.** Structurally
  `target{debug, release}` and `lyrid{data, target}` look the same; any
  threshold either splits the first or lumps the second.
- **Ranking by bytes written rather than net growth.** A rebuilt `target/`
  writes gigabytes and grows by nothing; it would fill the list.
- **A threshold in bytes.** No scale fits both a volume and a folder.
- **A kept history of folder totals for "this week".** A second format to
  freeze, and data kept unasked; ADR 0010 keeps nothing the reader did not
  ask for.
