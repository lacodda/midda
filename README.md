<p align="center"><img src="https://github.com/lacodda/midda/raw/main/assets/banner.svg" alt="midda - what is safe to delete" width="720"></p>

> A disk space analyzer for Windows that answers the question you actually have.

<p align="center">
  <a href="https://github.com/lacodda/midda/actions"><img src="https://img.shields.io/github/actions/workflow/status/lacodda/midda/ci.yml?style=flat-square" alt="CI"></a>
  <a href="https://github.com/lacodda/midda/blob/main/LICENSE"><img src="https://img.shields.io/github/license/lacodda/midda?style=flat-square" alt="License"></a>
</p>

Existing tools tell you *what is large*. That is the easy half. Looking at 40 GB spread across project folders, the real question is *which of this comes back with one build command, and which is the only copy*. `midda` answers that one.

The name comes from *midden* — the archaeological refuse layer. A disk is stratified the same way: under this week's files sit fossilized `target/` and `node_modules` directories from projects abandoned years ago.

## What makes it different

**Speed.** Reading the NTFS Master File Table directly scans a full volume in seconds instead of minutes — one sequential read instead of a syscall per directory.

**Freshness without rescanning.** The index is kept current as the disk changes and saved between runs; with elevation, the USN change journal brings it up to date with everything that changed while midda was closed. Open the app and the picture is already current. Neither WizTree nor TreeSize does this.

**Growth, not only size.** Every time a folder is opened, midda keeps the picture it had last time. What grew since then is one press away: the few folders eating the disk, each counted once, the pace in gigabytes a day and how long the free space lasts at it - and a treemap sized by how much changed, red where it grew, green where it was freed. Take a snapshot before a cleanup, and afterwards one page says how much came back and from where.

**Why it is full.** Every byte goes to one consumer - Windows, programs, games, development, browsers, virtual disks, media, documents - by the places midda knows by name, with the few folders that hold the most of each. One paragraph says why the disk is full; copied, it is what an assistant needs to answer the same question. A WSL, Docker or Hyper-V disk is read inside without starting it: what it holds, what compacting gives back, and what Docker keeps that nothing uses.

**Domain knowledge.** Not "here are big folders" but "here are 40 GB that a build regenerates": `node_modules`, `target/`, `.next`, `dist`, `__pycache__`, `.gradle` and the npm, pnpm, cargo, pip and Gradle caches. Every finding explains itself — what brings it back, what is lost, how careful to be — and counts what deleting it really gives back: a `node_modules` linked from pnpm's store frees almost nothing, and says so. A cache is cleaned by its own tool, once the tool confirms it is the cache found.

**The age of the layers.** How old the bytes on a disk are, in bands from this month to over five years, and the fossils: the largest folders nothing has written to in a year, each named whole when nothing in it moved. Where Windows keeps it, each says when a file in it was last opened. A fossil worth keeping is compressed in place - NTFS compression, files where they are, undone in Explorer - and midda measures what came back.

**What Windows leaves behind.** The Recycle Bin, the previous Windows and the updates it downloaded, browser caches, what nothing has written to in a temporary folder for a week, crash reports and memory dumps, unfinished and old downloads, disc images, installers of programs already installed, empty folders and shortcuts to nothing - each with what is lost and how careful to be. An installer is checked against what Windows says is installed; a shortcut is read to see whether its target is still there.

**Rules of your own.** What midda knows is data, in the format your own rules are written in: put a file in midda's rules folder and it is tried first. Keep a finding once - the `target` that holds the build you hand out - and it is never listed again. Downloads moved to `D:` is still Downloads. See [the rules](https://github.com/lacodda/midda/blob/main/docs/rules.md).

**Deleting, written down.** Pick what can go - an entry, or a whole group - and confirm one list that names both ways: what comes back as it was is deleted for good, since the Recycle Bin would only move it; what may not goes to the bin, and is never deleted instead when the bin will not take it. Deleting a careful one for good is asked about twice. Every deletion is measured and logged with how it comes back: `pnpm install`, and the folder to run it in.

## Install

Windows 10 or 11, x64: download the `x86_64-pc-windows-msvc` archive from the [latest release](https://github.com/lacodda/midda/releases/latest), unpack it anywhere and run `midda-desktop.exe`. Nothing is installed: the folder is the program, and what it keeps between runs lives in `%LOCALAPPDATA%\com.lacodda.midda`.

## Privileges

Reading the MFT means opening the volume as a device, bypassing the filesystem. Windows grants that to administrators only — otherwise any program could sidestep file permissions.

So `midda` runs both ways: without elevation it walks the filesystem normally and works everywhere; an "accelerate" button restarts it as an administrator and switches to the MFT path. You choose, and every result says which way it was read and how long that took.

## Status

v0.15.0: a volume read, kept current and explained - what fills it, what grew, how old it is, what can go - and what can go deleted, with a log of how each comes back. Rules of your own are tried before midda's, and what you keep is never listed. The formats of the index and the rules may change before 1.0; see the [CHANGELOG](https://github.com/lacodda/midda/blob/main/CHANGELOG.md).

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
