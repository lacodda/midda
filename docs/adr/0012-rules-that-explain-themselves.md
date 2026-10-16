# 0012 — Rules that explain themselves, and what deleting really gives back

Date: 2026-10-16
Status: accepted

## Context

Up to v0.9 midda said what is large and what grew. The question the product
exists for is the other one: which of this a tool makes again, and which is
the only copy (Vision §2). That needs knowledge of a developer's disk —
`node_modules` comes back with an install, `target` with a build, a package
manager's cache with the next download — and it needs to be said honestly
enough to act on.

Four things were open: how a rule is written, what makes a folder a finding,
how much a finding is worth, and how a package manager's cache is cleaned.

## Decision

**Rules are data.** The rules midda ships are a file,
`crates/midda-core/rules/dev.toml`, embedded in the core and read by the code
that will read a user's own rules (v0.16): one format, so whatever the
shipped rules can say, a user's can. TOML, because a rule deserves a comment
saying why it exists.

**A rule names a folder, the conditions that explain it, and its ways back.**

- *Where*: a path read from its end — `node_modules` anywhere,
  `AppData/Local/npm-cache` in any profile — with `*` and `?` inside a name,
  case ignored. Above the root of a scan the names are the root's own path,
  so a cache is recognised whether the scan began at `C:\` or inside the
  profile.
- *Beside* and *inside*: a `target` is a Rust build only with a `Cargo.toml`
  beside it; a `__pycache__` only with bytecode in it.
- *Ways back*, tried in order, each with a risk and a sentence: a
  `node_modules` with `pnpm-lock.yaml` beside it is safe and comes back with
  `pnpm install`; without a lockfile it is *caution*, because an install may
  bring newer versions. The last way back has no condition and is required —
  a rule that cannot explain every folder it names cannot be written, and the
  reader refuses it.
- *Closed*: nothing inside a folder whose name alone says what it holds is
  looked at by any rule, even when its own conditions fail. A global npm
  folder has no `package.json` beside its `node_modules`; without this every
  installed package's `dist` would read as "what the project's build wrote",
  and deleting it would break the package. Measured on a real folder of
  projects: 859 findings, 605 of them such `dist` folders, before; 115 after.

**A finding is the first folder down from the root a rule names**, and is
one thing: nothing under it is looked at. It is also a *unit* for the list of
who is eating the disk (ADR 0011): a build folder is one line, not its
`debug` and `release`. Trees of changes hold only what changed — not the
`Cargo.toml` beside a folder — so units are worked out on the picture that
holds the folder and matched into the tree of changes by name: the later
picture for what grew, the earlier one for what was freed.

**A finding carries two numbers: what it holds, and what deleting it frees.**
A file is freed only when every one of its names is inside the folder. A
`node_modules` installed by pnpm is hard links into the store: it can hold a
gigabyte and free a few megabytes, and the second number is the one on the
line, with the first said beside it. A file whose names were counted but
whose identity was not given cannot be matched, and is not counted as freed.

**A cache is cleaned by its own tool, after the tool says it is the cache
found.** `npm cache clean --force`, `pnpm store prune`, `pip cache purge` —
the tool knows what in its cache is still in use. Before running, midda asks
the tool where it cleans (`npm config get cache`, `pnpm store path`, `pip
cache dir`) and runs the command only when the answer is the folder found: a
scan of `C:\` finds every profile's cache, and a pnpm 10 prunes `store\v10`,
not the `store\v3` an older pnpm left. The tool is looked for on the search
path only, run with no input, no window and no shell, from the folder above
the cache — a package manager started inside a project reads that project
first. Afterwards the folder is read again whole and the window says what
came back, measured. Cargo and Gradle have no stable command; their caches
are findings to delete (v0.15).

Nothing is deleted in this version. The window lists, explains, adds up and
cleans caches through their tools; deleting a folder, with its confirmation
and the journal of what to run to bring it back, is v0.15.

## Consequences

Positive:

- One function in the core (`RuleSet::find`) answers the window now and the
  CLI and the MCP door later; a whole folder of projects (1.3 million
  entries) is searched in about 60 ms, so the findings are worked out again
  whenever the tree changes.
- Every finding says what it is, how it comes back and how careful to be,
  and the figure on it is what deleting would really give back.
- The format of user rules is already the format of the shipped ones.

Negative:

- A rule's knowledge is only as good as its conditions: a `dist` is always
  *caution*, because some projects keep files there they did not build.
- "Up to" is an upper bound for a tool's cleanup: pnpm keeps what a
  registered project still uses even when no link to it is on this volume.
- Units are matched by name: a folder that was renamed between two pictures
  is not recognised in the earlier one.

Rejected alternatives:

- **Rules as Rust literals.** The format of v0.16 would be written from
  nothing then, and the shipped rules would not be an example of it.
- **JSON or a format of our own.** JSON has no comments; a format of our own
  is a parser to keep, where `toml` is already in the dependency tree.
- **The size a folder holds as its worth.** For a pnpm project that is the
  store's bytes, which deleting the project's folder does not give back.
- **Deleting a cache folder rather than calling its tool.** Deleting a pnpm
  store breaks nothing installed, but throws away every package a project
  would need again; the tool's prune keeps those.
- **Running the cleanup without asking the tool where it cleans.** On a
  scan of a system volume it would clean the current user's cache while the
  window showed another's.
