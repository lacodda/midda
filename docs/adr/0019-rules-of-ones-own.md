# 0019 — Rules of one's own, what is kept, and folders that move

Date: 2026-11-12
Status: accepted

## Context

The rules have been data since v0.10 (ADR 0012) so that a reader's own could
be written in the same format; until v0.16 nothing read a reader's. Four more
things were open in the format before it is frozen for 1.0:

- **"This one is mine."** A `target` that holds the build someone hands out
  is Cargo's build output by every rule, and was listed as safe to delete on
  every visit. Nothing could say once that it stays.
- **Folders the shell moves.** Downloads is in the profile until a policy or
  the reader moves it - to `D:\Downloads`, or into OneDrive. The rules named
  `/Users/*/Downloads`, so a moved Downloads had no old downloads and no
  installers, and was "unrecognised" in what fills the disk.
- **A folder that says what it is.** Tools that keep a cache mark it with a
  `CACHEDIR.TAG` (bford.info/cachedir): pytest, mypy, ruff, ccache, borg,
  Cargo. A rule could name a folder by where it is or what is beside it, not
  by what a file in it says.
- **More of what Windows leaves.** The previous Windows, the updates it
  downloaded, Delivery Optimization's cache, Explorer's thumbnails, browser
  caches; and the Recycle Bin said only "what was deleted on this volume",
  not how much or since when, under a SID nobody reads.

## Decision

**A reader's rules are files in `%APPDATA%\com.lacodda.midda\rules`,** every
`.toml` there, in the format of the shipped ones and tried before them, in
the order of their names (`RuleSet::with_own`). On their own disk the reader's
word comes first. A file that does not read - bad TOML, a rule that cannot
explain itself, an id another file has - is refused with why, and costs only
itself: one typo does not take every other rule with it. To change a shipped
rule, a reader writes one with another id; overriding by id would be a second
way to say the same thing. The folder roams with the account: rules are what
a person wants of any disk, unlike the index, a picture of this machine's.
Adding a file reads it with the rest first and refuses it with the same why;
removing one sends it to the Recycle Bin.

**What is kept is in the format too: `[[keep]]`, with paths and an optional
why.** Nothing a keep names is a finding, nor anything in it; a folder that
holds something kept is never taken whole - the rule that names it leaves it
as one thing nobody deletes (closed), so the `target` around a kept artifact
still colours as a tool's and is not looked into. The window's **Keep** writes
the entry's path, with its drive, into `kept.toml`, the window's own file in
the folder; **Stop keeping** takes it out. The window rewrites that file only
while it holds keeps alone: a rule the reader put there by hand is theirs.

**A path can start with a drive or a known folder.** `D:/Archive` is that
volume's and no other's - what a reader writes of their own disk, and what a
keep is. `{downloads}/*` is a path for every place the folder is: every
profile's own `Downloads`, and where this account's shell keeps it now, asked
of `SHGetKnownFolderPath` when the set is read (`Known::of_this_account`).
The shipped rules and places name Desktop, Documents, Downloads, Music,
Pictures and Videos that way, so both the rules (old downloads, installers,
`within`) and what fills the disk see a moved folder. A name in braces that is
not a lowercase word - a GUID - is a folder's own name, as before.

**A marker is a file the folder holds and how it starts** (`[rule.marker]`).
The survey sees that the file is there; what it starts with is read after the
walk, as a check is, at most 512 bytes. The shipped `tagged-cache` is the
standard's signature, last of the rules that name a folder by what it is: a
Cargo `target` holds a tag too, and is Cargo's build first.

**`about` tells more of a finding without a say in whether it is one.** The
Recycle Bin's finding is listed at once; its `$I` records are read after the
walk, once per change of the folder, and the line says "12 things deleted by
ada, from 3 days to 5 months ago" - the account named by `LookupAccountSidW`.
A check would have held the finding back until the files were read, and what
fills the disk counts findings, not entries waiting.

**The shipped rules are the files in `rules/`, in the order `SHIPPED` lists
them,** and a test fails when a file there is not listed. A rule sent by pull
request is a file or an entry in one, in the reader's format, with a test
beside it (CONTRIBUTING). The order is listed rather than taken from the
names: which rule applies first is a decision, and file names are a poor
place to make it.

New groups: **browser caches** (`Cache` and `Code Cache` of every Chromium
profile, emptied - a running browser keeps the folder open) and **what
Windows keeps** (the previous Windows - caution, it is the way back; update
downloads and Delivery Optimization's cache - safe, administrator rights; the
thumbnail cache). Installers are looked for on the Desktop as well as in
Downloads, and Yarn's and Bun's caches beside npm's and pnpm's.

## Consequences

Positive:

- A reader says once what is theirs, and is not asked again; what they keep
  is a line in a file they can read, share and put under version control.
- A moved Downloads is Downloads for every rule and every place, and the same
  rule is right for every account on the volume.
- Any tool that follows the cache standard is found without a rule of its own.
- The format of the reader's rules and of midda's is one, checked by one
  reader; a contributed rule is reviewed as data with a test.

Negative:

- Another account's moved folders are not known: only this account's shell
  can be asked. Their profile folders still are.
- A folder with a `CACHEDIR.TAG` that does not carry the signature is not
  looked into by other rules: it was named, and its marker said no.
- A keep's path names a volume by its letter; a drive that changes letter
  keeps nothing until the keep is written again.

Rejected alternatives:

- **The registry's `User Shell Folders` read directly.** It is the store
  behind the answer, holds unexpanded variables, and misses a redirection the
  shell applies; `SHGetKnownFolderPath` is what Explorer asks.
- **Protected paths as a list in the window's settings.** A second format for
  what is a rule's business, which a shared rules file could not carry.
- **Overriding a shipped rule by its id.** Two answers to "which rule is
  this", and an upgrade of midda would silently drop the reader's change.
- **Reading markers in the survey.** The survey runs over trees that are not
  the disk now - a snapshot, a comparison - and reads no file.
