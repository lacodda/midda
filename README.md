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

**Growth, not only size.** Every time a folder is opened, midda keeps the picture it had last time. What grew since then is one press away: a treemap sized by how much changed, red where it grew, green where it was freed. Take a snapshot before a cleanup, and afterwards one page says how much came back and from where.

**Domain knowledge.** Not "here are big folders" but "here are 40 GB that a build regenerates": `node_modules`, `target/`, `.next`, `__pycache__`, Docker layers, package manager caches. Every finding explains itself — what recreates it, what is lost.

## Privileges

Reading the MFT means opening the volume as a device, bypassing the filesystem. Windows grants that to administrators only — otherwise any program could sidestep file permissions.

So `midda` runs both ways: without elevation it walks the filesystem normally and works everywhere; an "accelerate" button restarts it as an administrator and switches to the MFT path. You choose, and every result says which way it was read and how long that took.

## Status

v0.8.0. Pick a volume and see it as a sortable table and a treemap of what is on disk. The picture stays current while the window is open and is saved for the next start. "Grown" compares the folder with the last time it was open; snapshots compare any two moments, with a one-page report of what a cleanup gave back. "Accelerate" restarts midda as an administrator to read the Master File Table in seconds and follow the change journal. Rules for *what* is safe to delete are next: see the [CHANGELOG](https://github.com/lacodda/midda/blob/main/CHANGELOG.md).

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
