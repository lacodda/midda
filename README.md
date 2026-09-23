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

So `midda` runs both ways: without elevation it walks the filesystem normally and works everywhere; an "accelerate" button restarts it as an administrator and switches to the MFT path. You choose, and every result says which way it was read and how long that took.

## Status

v0.4.1. Pick a volume, watch it read, and see the folder two ways at once: a sortable table of any length, and a treemap whose rectangles are shares of what is on disk. Click either and the other highlights the same entry; double-click to go in, Backspace to come back. The picture takes arrow keys, names the full path under the pointer, and copies itself as a PNG. Sizes are the space a file occupies rather than the space it reads as, so a hard-linked package is counted once however many names it has, a cloud placeholder costs what it costs here, and every entry whose two numbers disagree says why. The rules that name *what* is safe to delete are next — see the [CHANGELOG](https://github.com/lacodda/midda/blob/main/CHANGELOG.md).

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
