# 0014 — What fills the disk, and why it is full

Date: 2026-10-21
Status: accepted

## Context

Up to v0.10 midda said what is large, what grew and what a tool makes again.
The question a full disk asks first is another one: *why* is it full. The
answer a person can act on is a handful of reasons — games take a third of
it, development another third, Windows a tenth — and the places behind each,
not a million entries. The same answer is what an assistant asked "why is my
C: full" needs, and what the CLI and the MCP door will give (v0.19, v0.20).

Four things were open: how midda knows what a folder is, how every byte gets
exactly one answer, how each answer is located, and who writes the sentence.

## Decision

**What things are is data, in the rules' own format.** `rules/places.toml`
sits beside `rules/dev.toml` and is read by the same parser. A `[[place]]`
names a folder or a file by a path read from its end, with `beside` and
`inside` as a rule has them, and says which consumer it is: system,
programs, games, development, browsers, virtual disks, media, documents.
A `[[file]]` says what a file is by its name — `*.mp4` is media. A path
written with a leading `/` is anchored to the top of a volume: `/Windows` is
the system's folder, not `AppData\Local\Microsoft\Windows`. Ids are unique
across every file; the first rule or place in the set that names an entry is
the one that applies.

**Every byte gets one consumer, the first of these with an answer:**

1. A folder a rule finds or closes is its group's consumer, whole —
   `node_modules`, a Cargo `target` — whatever place it is in. A group
   belongs to one consumer (`dependencies`, `build`, `caches` are
   development), so a rule does not name its own: two fields that must agree
   are two answers to one question.
2. The deepest place naming the file or a folder above it: Steam's library
   inside Program Files is games, Docker's disk inside Docker's folder a
   virtual disk.
3. The file's own name — but only where no place above it decides: a video
   inside a game is the game's.
4. A soft place (`yields = true`) — Documents, Downloads, Pictures — for the
   files whose names say nothing.
5. Nothing: the file is *unrecognised*, said as such rather than guessed.

The consumers add up to the root, byte for byte, and a test holds them to
it.

**Where in a consumer is the rule that names who is eating the disk.** The
lines of a consumer are the folders that hold the most of it, each named as
deep as it is one sizeable thing and counted once (ADR 0011), asked of one
consumer's bytes instead of growth. The walk of the tree is made once
(`growth::Folders`) and asked of all nine consumers over dense arrays per
folder, rather than nine walks of every entry. A line is called by the place it
is when it holds nearly all of one (nine tenths) — a profile folder by
"Chrome", the folder holding Docker's disk by "Docker's disk" — and a place
is said once.

**The core writes the sentence.** `consumers::story` tells why the volume is
full: how full, then up to five consumers that take at least a fiftieth,
each with up to two places that hold a tenth of it, what a tool makes again,
and that a virtual disk is one opaque file. Always on disk: "full" is a fact
about clusters. Beside the paragraph a brief — the same paragraph with every
consumer's places, as Markdown — is what goes into a chat. Numbers are
written by `words`, which mirrors the window's `formatBytes` and
`formatShare` case for case, so a sentence and a row beside it never
disagree.

**The treemap's colours read the same rules.** "Rebuilt by a tool" is a
folder the rules found or closed and what is inside it, no longer a list of
names (`bin`, `build`, `out`) that coloured a program's own folder as
something a build writes; a file's colour is what the rules files say it is
by name.

The window has a third way to show a folder: *What fills it*, beside *Tree*
and *Safe to delete* — the paragraph, then each consumer with its share and
its places, each opening in the tree.

## Consequences

Positive:

- One place knows what a folder is; user places (v0.16) are the same format.
- Measured on a system volume of 3.0 million entries, release build: the
  rules' survey 0.17 s, the classification 0.26 s, the summary 0.13 s (0.7 s
  as nine walks). The window works each out once per version of the tree,
  the first time it is asked.
- The paragraph is the same in the window, the CLI and an assistant's
  answer.

Negative:

- What no place names is unrecognised. The shipped places are a judgment of
  where Windows, launchers and tools keep things, and will grow.
- A soft place yields to names only: a `.dat` in Videos is media, a `.txt`
  there a document.
- A virtual disk is one file: what is inside it is v0.12's (ADR 0015).

Rejected alternatives:

- **A category from the largest kind of file inside a folder.** A guess, and
  a different answer every time the folder changes.
- **A `consumer` field on every rule.** It can disagree with the group.
- **The sentence written by the window.** The CLI and the MCP door would each
  write their own.
- **Nine walks of the tree, one per consumer.** Correct, and five times
  slower on a system volume each time the tree changes.
