# 0006 — Accelerate by relaunching elevated; stream the MFT

Date: 2026-09-23
Status: accepted

## Context

ADR 0001 gives midda two scanners behind one trait and an explicit "accelerate"
action that trades a UAC prompt for an MFT read. v0.5.0 builds that action and
the MFT scanner. Three questions were open.

**Where the elevated code runs.** Only an elevated process may open a volume as a
device. Either the whole window restarts elevated, or the window stays as it is
and starts an elevated helper that reads the table and hands the result back.

**How the table is read.** `ntfs-reader` — the crate ADR 0001's survey settled
on — parses the format well, but its `Mft` reads the entire table into one buffer
before handing out a record, and its aligned reader serves one 4 KiB block per
call. The table of a busy system volume is gigabytes.

**How two scanners can produce the same tree.** Node order is load-bearing:
shared bytes go to the first name in it (ADR 0005). The walk's order was
whatever `read_dir` returned; the MFT has no directory listing to return.

## Decision

**"Accelerate" restarts the whole window as an administrator.** `ShellExecuteExW`
with the `runas` verb starts the same program with `--after <pid> --scan
<folder>`: the new window waits for the old one to close — two WebView2
instances at different elevation cannot share one data directory — and scans the
folder the reader was looking at without asking twice. A declined prompt leaves
the old window open, on the walk, saying so.

**The table is streamed.** `ntfs-reader` is used for the format only: the boot
sector, the data runs of `$MFT`, the attributes of a record. The table is read
straight from the device in 16 MiB cluster-aligned reads, each record is reduced
at once to a small summary — its names and their parents, both sizes of the
unnamed stream, its attributes, its write time, its reparse tag — and extension
records are merged into their base by number, so the attribute list never needs
following. Turning summaries into a tree is a pure function, tested on every
platform.

**One listing order for both scanners.** Children are pushed by name with case
folded, then by exact name (`tree::listing_order`). The walk sorts each
directory's entries before returning them; the MFT scanner sorts the name
edges. The two trees are then the same arena, node for node, and
`tests/scanners_agree.rs` compares them field by field on a fixture built to
hold every case the two could read differently. CI's Windows runner is elevated
and must run it.

**The fast scanner falls back, out loud.** `scan()` prefers the MFT when the
process is elevated and the folder is on NTFS. If the read fails for any reason
but the root itself or a cancel, the walk reads the folder and the tree records
why.

## Consequences

Positive:

- One process, one code path for the tree: the elevated window is the same
  program with the same bridge. v0.7's second freshness step — USN with UAC —
  lives in that process too.
- Memory for a scan is the summaries and the tree, not the raw table.
- The owner of shared bytes is statable on screen and identical on NTFS, ext4
  and through the MFT.

Negative:

- An elevated window runs everything elevated — including, from v0.15,
  deletion. That is the reader's choice, made at a prompt, but it is broader
  than the read needs.
- The window closes and reopens, which loses nothing today (a scan is the only
  state) and will need thought once the window holds more.
- A table so fragmented that its own map spills out of record 0 is not streamed;
  the walk reads the volume and the tree says why.

Rejected alternatives:

- **An elevated helper process.** Keeps the window unelevated, but needs a named
  pipe (a `runas` launch cannot redirect standard streams) and either a UAC prompt
  per scan or a helper that lives for the session — the service of v0.21, built
  early and without its install step. When that service exists, the window will
  read its index; the relaunch stays as the path for anyone who did not install
  it.
- **`ntfs-reader`'s `Mft` as is.** The least code, and peak memory equal to the
  table plus the tree, which is not fixable later without doing this anyway.
- **Comparing the trees by path and keeping the volume's order.** The test would
  pass while the two scanners gave shared bytes to different folders.
