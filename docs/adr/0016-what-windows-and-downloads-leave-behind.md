# 0016 — What Windows and downloads leave behind, said by the same rules

Date: 2026-10-28
Status: accepted

## Context

Up to v0.12 the rules (ADR 0012) knew a developer's disk: a `node_modules`
comes back with an install, a `target` with a build. What a person expects
of a disk cleaner first is something else: the Recycle Bin, the temporary
folder, the crash reports Windows keeps, the installers piling up in
Downloads, old downloads, empty folders and shortcuts to programs long gone.

None of that is named by what is beside it, which is all a rule could say.
A memory dump and an installer are files, and rules named folders only. A
temporary folder must not be deleted at all — a program that finds `%TEMP%`
gone does not make it again — only emptied of what nobody uses. A download is
old or not. An empty folder is empty, and whether it is a leftover depends on
whose it is. Whether a shortcut points to anything, or whether what an
installer installs is installed, is in the file and on the computer, not in
the tree.

## Decision

**The same rules, in the same format, with what they could not yet say.**
The shipped set gains `rules/system.toml`, read by the code that reads
`dev.toml`, and the format gains:

- `entries = "folders" | "files" | "all"`: what the paths name. Folders by
  default, so every rule written before means what it meant.
- `older = "7 days"`: only an entry nothing in which was written for that
  long — the age of the layer, `subtree_modified`. A time the volume did not
  give is not old: an invented age is how something recent gets deleted.
- `goes = "whole" | "contents"`: what deleting the finding takes. With
  `contents` the folder stays and what is in it goes; with an age as well,
  only what in it is that old. A finding is still one entry — the folder —
  with what goes counted on it (`Goes::Contents { taken, of }`), and
  `RuleSet::going` names the entries for the deletion of v0.15.
- `hollow = true`: a folder with no file anywhere in it and nothing in it the
  scan could not read. A folder the scan was refused looks empty and is not.
- `within = [consumers]`: only strictly inside what those consumers hold, by
  the places of `places.toml` — never a place itself. An empty folder among
  the reader's documents is a leftover; one a program keeps in `AppData` may
  be one it expects to find, and an empty `Downloads` is not a leftover.
- `except = [paths]`: what the rule never names, and what never goes with a
  folder it empties — the shell's `desktop.ini`, which says how a folder is
  shown, and the folders at the top of OneDrive.
- `check = "shortcut" | "installer"`, and on a way back `if = "installed" |
  "older-installed"`: what only the file can say. The survey names the entry
  and leaves it *waiting*; the check is asked afterwards, with no lock on the
  tree held, and says whether the entry is the rule's at all (a shortcut that
  works is not) and which way back applies. What a check saw is kept by path,
  size and time, and asked again only of a file that changed.
- `**` in a path: any number of folders, so `Start Menu/**/*.lnk` reaches a
  program's own folder in the menu.

**Whose an entry is reaches above the scan.** A scan of a folder inside
`AppData` is a program's though `AppData` is not in the tree: the folders
above the root are matched by their path, from the top of the volume down,
and a place there is taken on its path alone — what is beside it cannot be
asked, and taking it is the careful side for a leftover.

**The reader's own files are found by whose they are, not where Windows put
them first.** OneDrive moves the Desktop and Documents into its own folder,
under names in the reader's language (on a Russian Windows the Desktop is
`OneDrive\` and the Russian word for it); a path
`/Users/*/Desktop` misses them. A broken shortcut is any `.lnk` among the
documents, media and what nothing names, plus the Start menus by path; an
empty folder is never one of the folders at the top of OneDrive, where a
moved known folder may stand empty and still be where programs save.

**Groups say whose bytes they claim, or that they claim none.** The Recycle
Bin, temporary files and crash reports are Windows' (`system`), as the places
already said. Downloads and leftovers claim nothing: an old video in
Downloads is media whether or not it is old, so "what fills the disk" is told
as before, and their findings only list. What a tool makes again — the
picture's colour, one line of growth — stays the development groups'.

**A shortcut is read by its format, not by the shell.** A `.lnk` is a Shell
Link (MS-SHLLINK): midda reads the local path, the target written with
variables and the relative path, and nothing else. It is broken when every
path it gives is on a local volume that is here and nothing is at any of
them. A target on a share, on a drive not connected, behind a folder this
account may not read, named only by the shell's item list, or an advertised
shortcut of Windows Installer is not broken: unknown is never the worse case.
Variables of a profile are that profile's, not the reader's.

**An installer is matched exactly where it can be, by whole names where it
cannot.** A Windows Installer package says its product and upgrade codes;
Windows Installer says whether either is installed, and in what version. An
`.exe` says its product, maker and version in its version resource; the
programs Settings lists are the `Uninstall` keys of the machine (both views)
and of the account. A name matches only when the makers agree, if both say
one, and the names are the same words once the maker's own name, versions
and `Setup`-like words are left out: `Microsoft Visual Studio` is not `Visual
Studio Code`, though every word of it is in it. A version is a number with
dots or a bare number; a year and a generation (`2013`, `v14`) are parts of
the name — `Visual C++ 2013` and `Visual C++ v14` are installed side by side.
A processor said by both must be the same (the x86 and x64 runtimes are two
programs); said by one, it says nothing against. Of several entries that
match, the one in the very version is spoken of first. The same or a newer
version installed is *safe*; an older one installed is *caution* — perhaps an
update not yet run; nothing found, or an installer that does not say what it
installs, is *caution*.

**What ships, and how careful each is.** The Recycle Bin, emptied, *caution*:
it is deleted for good. Temporary files untouched for a week, *safe* — the
age Disk Cleanup waits. Error reports, crash dumps and `MEMORY.DMP`, *safe*.
Unfinished downloads still for a day, *safe*. Disc images and downloads
untouched for three months, *caution*: they are the reader's. Broken
shortcuts, *safe*; empty folders, *caution*, freeing nothing but tidiness.
`C:\inetpub` becomes a place of Windows: an update of April 2025 makes it
empty on purpose and Microsoft asks that it stays.

"Safe" is widened to say so: nothing anyone needs is lost — it comes back as
it was, or nothing uses it any more. The story of a volume says what of
Windows can go, and keeps "a tool makes it again" for development.

Nothing is deleted in this version; the window lists, explains and adds up.

## Consequences

Positive:

- One list of findings for the window now, and for the CLI, the MCP door and
  deletion later; a user's rules (v0.16) can say "installers in `D:\Setups`
  older than a year" with the shipped words.
- A finding that comes from a file is as honest as one from the tree: a
  check that cannot tell says nothing rather than the worse case.
- The survey stays a walk of the tree; files are read only for the few
  entries that wait on a check, once per change of each.

Negative:

- Ages are times of writing: a file copied out of an archive keeps the
  archive's time and can read as old the day it arrived. Downloads are
  *caution* for that reason among others.
- An installer whose name differs from the program it installs — a web
  installer called `Google Update Setup` — is not recognised, and stays
  *caution*.
- Known folders moved somewhere other than OneDrive (Downloads on `D:`) are
  not found by the paths; reading where the shell keeps them is to come.
- A Recycle Bin is one line per account, named by the account's SID.

Rejected alternatives:

- **A module of its own for system cleanups, beside the rules.** Two lists
  of findings for one page, and a user's rules could not say any of it.
- **A finding as a set of entries.** One more thing to name, count, order
  and delete; an emptied folder is one finding with what goes counted on it.
- **One line per file in a temporary folder.** Thousands of `tmp*.tmp`; the
  reader wants to know that the old part of the folder can go.
- **`IShellLink` and `Resolve`.** COM, and a resolve that searches the disk
  and may rewrite the shortcut it was asked about.
- **Matching an installer's name as words anywhere in a program's name.** It
  called the Visual Studio installer "installed" because Visual Studio Code
  is.
- **Empty folders anywhere.** A program's empty folder in `AppData` or
  `Program Files` may be one it fails without.
- **Shortcuts by the paths of the Desktop and Documents.** On the first
  computer it ran on, both were in OneDrive under Russian names.

## Measured

On the owner's computer, read only: the Desktop's 23 shortcuts all work;
the Start menu holds four broken ones (a Python installed into a temporary
folder that is gone); the Recycle Bin 16 MB of one account, another's
unreadable without rights. Seven installers of installed products in
`Package Cache` each name their own product and version. The survey of a
folder of projects (1.19 million entries) takes 80 ms, 60 before files were
asked of rules — each file only of the rules its name lets name it.
