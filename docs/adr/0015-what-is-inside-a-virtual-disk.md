# 0015 — What is inside a virtual disk, read from the file

Date: 2026-10-27
Status: accepted

## Context

Since v0.11 midda has known a WSL distribution's disk, Docker Desktop's disk
and a Hyper-V machine's disk as *virtual disks* (ADR 0014): one file each,
often the largest file on a developer's volume, and said to be opaque. That
was the honest answer and not a useful one. The question a reader has about a
54 GB `docker_data.vhdx` is how much of it is still something, and the known
trouble is that a disk file grows as the machine inside writes and does not
shrink when it deletes: `docker system prune` frees room inside the disk and
gives none of it back to Windows until the file is compacted.

Four things were open: how midda learns what is inside without starting the
machine, how it knows whose disk a file is, what it says compacting gives
back, and what it says about Docker's own contents.

## Decision

**The file is read, not the machine asked.** A VHDX file says, in its block
table, which blocks of the disk it holds; the room in the file no block holds
is what compacting gives back, exactly. midda reads the container itself
(`vdisk::vhdx`, MS-VHDX 1.0: headers and region tables checked by CRC-32C,
metadata, the table), then the filesystems on the disk from its own bytes
(`vdisk::guest`): an ext4 superblock for WSL and Docker, an NTFS `$Bitmap` for
a Windows machine, behind a GUID or MBR partition table when there is one.
Nothing is mounted, no machine is started, no rights are needed, and a
running machine is not kept from its disk: the file is opened to read and
shared every way. A checkpoint (a differencing disk) is said as one — what is
inside it is in the disk it was taken from — and VHD, VMDK, VDI and qcow2
are named by their signatures and not read inside.

**What is in use is what `df` says inside.** An ext4 filesystem keeps blocks
for itself from the day it is made — bitmaps and an inode table per group, the
copies of the superblock, the journal — 17 GB on the terabyte WSL makes. They
are counted as the kernel counts them and left out, so a 150 MB disk file is
not said to hold 17 GB. A layout this does not count (`meta_bg`, `bigalloc`)
leaves the figure unsaid rather than wrong.

**What compacting gives back is the room no block holds**, measured: the
file's size on disk less the blocks it holds and its own records. Room the
machine freed and still holds — blocks a filesystem no longer uses that it
has not trimmed — is said separately, and only while the disk is at rest: a
mounted filesystem writes its free count now and then, and a count from
minutes ago set against the block table of now would be two moments in one
figure. A disk that is open as midda reads it says so.

**Whose disk it is, the places say.** A `[[place]]` of the `virtual-disks`
consumer that names a file makes it a disk; its `owner` — `docker`, `wsl`,
`hyper-v` — or that of the nearest place above it says what has to stop
before it is compacted. Docker's two disks are two places, so the small one
it runs on is not called the one its images are in. WSL's own list in the
registry names a distribution's disk.

**Compacting is said, not done.** It needs the machine stopped and an
administrator, so midda gives the commands to paste: `wsl --shutdown` then
`diskpart` for WSL and Docker — `diskpart` is on every edition of Windows,
where `Optimize-VHD` needs Hyper-V — and `Optimize-VHD` for a Hyper-V
machine's disk, which implies Hyper-V. One way per owner.

**Docker's contents come from Docker's engine.** `GET /system/df` over the
pipe or socket the `docker` command talks to (`DOCKER_HOST`, or the
platform's own) says each image's size and whether a container uses it, each
volume's and whether anything mounts it, the build cache and what is in use.
What can go is reckoned as `docker system df` reckons it, so the two agree,
and each kind comes with its command and how careful to be: the build cache
and images without a name are safe, unused images and stopped containers are
caution, and volumes are data — midda lists the command that shows them, not
one that removes them. A prune is followed by a compaction; the page says so.

## Consequences

Positive:

- The largest opaque file on a developer's disk becomes a sentence: Docker's
  disk is 54 GB, the file holds 50 GB of blocks, compacting would give back
  next to nothing — and inside it 9 GB of build cache and 25 GB of images no
  container uses, with the command for each.
- Measured on the owner's disks while Docker ran: a 1 TB disk's block table
  and superblock read in 55 ms; Docker's engine answered in half a second.
- The story of a full disk says what compacting gives back instead of "one
  file", in the window, and later the CLI and the MCP door.

Negative:

- Only VHDX is read inside. VMware's and VirtualBox's disks are named and
  told how to compact, not measured.
- An ext4 free count is as fresh as the filesystem last wrote it; while the
  disk is in use the page says so, and says nothing it cannot hold to.
- Docker contexts other than the default are reached through `DOCKER_HOST`
  only; TLS and SSH endpoints are refused by name.

Rejected alternatives:

- **Asking the machine** (`wsl -d … df`, `Get-VHD`). Starts what the reader
  may have stopped, needs Hyper-V or rights, and says nothing for a disk
  whose machine is gone.
- **Mounting the disk.** Needs an administrator, and a mounted disk is a disk
  midda has changed.
- **A rule with a new group for disks.** A rule's ways back are about
  deleting a folder and making it again; a disk is neither deleted nor made
  again, and its figure is not what deleting would free.
- **`docker system df` as text.** Its figures are rounded for a person, and
  it needs the command on the search path; the engine gives bytes.
- **Running the prunes and the compaction from the window.** Deleting is
  v0.15's, with its confirmation and its journal; compaction needs rights the
  window does not have.
