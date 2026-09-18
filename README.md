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

**Freshness without rescanning.** The USN change journal updates the index incrementally. Open the app and the picture is already current. Neither WizTree nor TreeSize does this.

**Domain knowledge.** Not "here are big folders" but "here are 40 GB that a build regenerates": `node_modules`, `target/`, `.next`, `__pycache__`, Docker layers, package manager caches. Every finding explains itself — what recreates it, what is lost.

## Privileges

Reading the MFT means opening the volume as a device, bypassing the filesystem. Windows grants that to administrators only — otherwise any program could sidestep file permissions.

So `midda` runs both ways: without elevation it walks the filesystem normally and works everywhere; an "accelerate" button requests elevation and switches to the MFT path. You choose.

## Status

v0.2.0. Every volume at once, fullest first, then pick one and midda reads it — in parallel, with a live count. The result is a sortable table that draws a screenful of a folder however many rows it holds: name, items, share, what it occupies and what it reads as side by side, and the age of the layer beneath it. The treemap and the rules about what is safe to delete are still ahead — see the [CHANGELOG](https://github.com/lacodda/midda/blob/main/CHANGELOG.md).

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
