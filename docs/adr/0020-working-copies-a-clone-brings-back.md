# 0020 — Working copies a clone brings back

Date: 2026-11-22
Status: accepted

## Context

A developer's disk holds clones nobody has opened for a year: a library read
once, a project finished, an experiment. Each is a few hundred megabytes to a
few gigabytes - its sources, its history in `.git`, and the `node_modules`
and `target` its tools wrote. Until v0.17 midda named the `node_modules` and
the `target` inside it and nothing more, though the whole copy is often one
`git clone` away.

"One clone away" is true only when nothing in the copy is only here, and a
working copy hides what is only here well: a change not committed, a file git
does not track, a commit on a branch never pushed, a stash, a merge left
halfway - and, above all, what git ignores. `git status` calls a copy clean
with a `.env` in it, a local database, an editor's settings: ignored files are
never committed, so a clone never brings them back. A copy can be "clean and
pushed" and still be the only place a secret or a dataset lives.

## Decision

**A rule names a folder with a `.git` in it that nothing has been written to
for half a year (`git-working-copy`, group `repositories`), among the reader's
own folders only** (`within`: documents, media, development, unrecognised). A
copy a program keeps - a Scoop bucket, an editor's plugins - is the program's,
and breaks the program when it goes.

**Its check (`check = "repository"`) reads the repository with `gix`, after
the walk, from this machine alone.** It counts, as a clone of the copy's
remote would miss them:

- commits reachable from `HEAD`, any branch or any tag that no branch of the
  remote reaches, and the branches they are on;
- tracked paths that differ from `HEAD`, staged or not, and paths a merge left
  unresolved - read with untracked files switched off, since
  `status.showUntrackedFiles = no` hides them from `git status` and keeps
  none of them anywhere;
- what git does not track, and what it ignores, each folder once - read by a
  walk of its own;
- the stash, an operation left halfway, linked worktrees that share the
  copy's `.git` (they break when it goes), and submodules: each is read the
  same way, three levels down, and one with no remote of its own counts.

The remote is the one the current branch follows, else `origin`, else the
first. A remote that is a folder on this computer is not somewhere else. The
address is shown without its password, and a web address without its user,
where a token is written.

**What git ignores is lost unless a rule says it comes back.** The walk does
not stop at a working copy: what the rules find in it is found as before, and
the copy's check is told which of it comes back as it was (`Waiting::within`:
the findings in it, whole, whose way back is safe). A `node_modules` beside
its lockfile and a Cargo `target` explain themselves; a `dist`, which may hold
what no build made, does not, and neither does a `.env`.

**When nothing is only here, the copy is safe, and its command is the clone**
- `git clone <address> <folder>`, run beside the folder, written to the
deletion journal as any command is. The sentence says where the remote is and
when the copy last heard from it (`FETCH_HEAD`, the logs of the remote's
branches): nothing is fetched, so "on the remote" means as of then. **When
anything is only here, the copy is a finding to look at first, with what is
only here listed.**

**Once the check makes the copy a finding, what was found in it is part of
it** (`Survey::swallowed`): the list shows the copy, not its `node_modules`
beside it, so nothing is counted twice and nothing is deleted twice. What
fills the disk and the picture still see what is inside as what it is. A
folder the check cannot open as a working copy - a `.git` git would not open,
or a `.git` that is a file, another repository's worktree or submodule - is
none of the rule's, and keeps what was found in it. A copy something in which
is kept is never named.

**Nothing is written.** `git status` refreshes the index's timestamps; `gix`
reads without writing, so reading a copy does not make it "written today".
What is read first - the index, `HEAD`, the references, the packs, the root's
`.gitignore` - is read under `platform::keeping_access_time`.

## Consequences

Positive:

- An abandoned clone is one decision with one command back, freeing its
  history and sources with its build output.
- "Clean and pushed" is not mistaken for "nothing to lose": a `.env`, a local
  database, an unpushed tag or a stash keeps the copy careful, and says why.
- What is only here is named, so the reader knows what to push or save.

Negative:

- A copy with anything only here is one careful finding: the safe
  `node_modules` in it is part of it, not a line of its own. Saving what is
  listed makes it a safe one on the next look. Listing both would count the
  same bytes twice.
- Whether the remote still has what it had is not asked: a repository
  deleted from its host since the last fetch is still "on the remote". The
  sentence says when that was.
- Loose objects and `.gitignore` files further down are opened without their
  access time held; on a volume that keeps access times, the strata's
  "opened" of a `.git` can move. The age the rules go by is the written time,
  which a read never moves.
- A tag fetched from the remote that no branch of it reaches counts as only
  here: local and fetched tags look the same in a copy. The error is in the
  careful direction.
- Git LFS objects are taken to have been pushed with the commits that name
  them, as LFS's own hook pushes them.

Rejected alternatives:

- **Running `git`.** It is not on every reader's machine, its `status` writes
  the index, and parsing its words is a second format. furca reads the same
  repositories with `gix` 0.90.
- **Fetching to check the remote.** Network, credentials and a write to the
  copy, from a disk analyzer reading in the background.
- **Ignored files as "made again by the project".** True of `node_modules`,
  false of a `.env`; only a rule can tell the two apart, so the rules are
  asked.
- **A setting for the age.** A reader who wants a year writes their own rule
  with `older = "1 year"` and the same check.
