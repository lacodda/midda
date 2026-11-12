# 0018 — Deleting what the rules found, and writing down how it comes back

Date: 2026-11-12
Status: accepted

## Context

Up to v0.14 midda listed what can go, explained it, added it up, and cleaned
a cache through its own tool (ADR 0012, ADR 0016). It deleted nothing: the
Vision promises "picked, confirmed, the space came back", and the reader had
to go to Explorer for the last step, with the list in their head.

Three things were open. How an entry goes - for good or into the Recycle
Bin - and who decides. What the shell does when the bin will not take an
entry. And how a reader brings back, a month later, what midda deleted
because a tool makes it again.

The owner decided on 2026-09-11: what comes back as it was is deleted for
good, with a list; what may not goes to the Recycle Bin; a switch on the
group; both ways named in the confirmation; deleting a careful one for good
asked for separately; and a log of deletions with the command that brings
each back.

## Decision

**The way follows the risk, and the reader can move it.** A finding whose
rule says it comes back as it was (`safe`) is deleted for good: in the
Recycle Bin a `node_modules` only moves, and the volume gets nothing back
until the bin is emptied - the space the reader came for. One that may not
(`caution`) goes to the bin, where Explorer restores it. A group's switch
sends everything picked of it to the bin, or for good. What cannot go to the
bin - what is in it already, what is on a volume without one (not a fixed
drive) - goes for good or not at all, and `delete::Way::of_entry` says so
before the window asks.

**The confirmation names both ways, and asks twice for the dangerous one.**
Every entry picked is listed under the way it goes, with what it frees and
how it comes back. When any of them is `caution` and goes for good, the
first answer is "Continue", and a second screen lists only those, said as
what they are: nothing restores them. The core does what it is told - a
pick carries its way - and refuses a pick for the bin that cannot go there,
rather than choosing another way for it.

**For good is midda's own walk.** Not `remove_dir_all`: a link is removed
as a link and never followed (the junctions pnpm puts in a `node_modules`
point into the store every other project uses); a read-only folder loses
the attribute and goes, as Explorer deletes it (Rust deletes a read-only
file by itself; a read-only folder it refuses - measured); what the volume
refuses - a file held open, one this account may not change - stays, is
counted and named, and everything else still goes. It stops between two
files.

**The Recycle Bin is the shell's, and never means "for good".** The move is
`IFileOperation` with `FOFX_RECYCLEONDELETE`, no UI and the undo stack, so
Explorer lists and restores what midda recycled. The shell, though, deletes
for good what the bin will not take - too large for it, a bin turned off on
the volume - and says so only in a dialog that a silent operation answers
yes to. midda's progress sink is told, in `PreDeleteItem`, whether the shell
means to recycle (`TSF_DELETE_RECYCLE_IF_POSSIBLE`); when it does not, the
sink fails the call, which cancels the item. The entry stays where it was,
and the window says the bin would not take it. A deletion the shell reports
without an item in the bin is said as a failure, not as a move.

**Measured, as everything else is.** Each finding is read again as soon as
it has gone (`Index::read_again`), and what the window and the log say it
freed is before less after. What went to the bin is said apart: it is moved,
and the space comes back once the bin is emptied.

**Written down before the next goes.** `deletions.jsonl`, beside the
indexes in the local application data: one JSON object per line - when,
where, what the rule called it, its risk and way, what it took and what of
that went, how many entries stayed, and how it comes back: the command and
the folder beside the entry to run it in. Each line is flushed to the volume
before the next entry is deleted, starts on a line of its own after a line
cut short by a crash, and a line that does not read is passed over. The
window lists the latest with the command and the folder to copy.

## Consequences

Positive:

- The loop the Vision promises closes inside midda: picked, confirmed,
  measured, written down.
- "To the Recycle Bin" is a promise the code keeps: there is no path on
  which an entry picked for the bin is deleted for good.
- A month later, `pnpm install` and the folder to run it in are on the page
  that deleted the `node_modules`, and in a file a person can read.
- The same `delete::take` serves the CLI of v0.19.

Negative:

- What the shell refuses is refused whole: a folder too large for the bin
  stays, and the reader deletes it for good or leaves it.
- The log says how something comes back, not that it can: a lockfile edited
  since makes `pnpm install` bring other versions. The rule's risk already
  says how sure the way back is.
- The bin's own contents can only be deleted for good, and only with the
  second confirmation - emptying the bin is that.

Rejected alternatives:

- **Everything into the Recycle Bin.** It frees nothing until emptied, and a
  bin full of `node_modules` is the disk-full problem moved one folder over.
- **A move of midda's own into `$Recycle.Bin`.** Explorer would not know
  what it holds or how big the bin is, and the format of the bin's records
  is the shell's to change.
- **`SHFileOperation` with `FOF_WANTNUKEWARNING`.** A dialog of the shell's
  in the middle of a list, asking about one entry of forty.
- **`std::fs::remove_dir_all`.** It follows nothing, but stops at the first
  refusal and says nothing of what stayed.
- **The log inside the index.** The index is a picture of the disk and is
  replaced when a folder is read again; the log is a history and outlives
  every picture.

## Measured

On this machine, in temporary folders only: a folder recycled through the
shell is gone from its place and recorded in the account's `$Recycle.Bin`
(the `$I` record names the original path; the test takes it out of the bin
again). A junction inside a `node_modules` was removed and its target kept;
a file opened with no sharing stayed while the rest of its folder went; a
read-only folder was refused by `RemoveDirectoryW` until its attribute was
taken off.
