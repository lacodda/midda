//! Virtual disks: a WSL distribution, Docker, a Hyper-V machine - each one
//! file on the volume with a whole disk inside, and what that disk holds.
//!
//! A disk file grows as the machine inside writes and does not shrink when it
//! deletes: Docker Desktop's disk keeps the room of every image pulled since it
//! was made, after `docker system prune` as before. From outside the file is
//! one entry of fifty gigabytes, and the question is how much of that is
//! still something. midda answers it from the file itself, without starting
//! the machine and without rights:
//!
//! - the VHDX block table says how much of the disk the file holds
//!   ([`vhdx`]); the rest of the file is room no block holds, and compacting
//!   gives it back;
//! - the filesystems on the disk say how much of it they use ([`guest`]): an
//!   ext4 superblock, an NTFS `$Bitmap`;
//! - WSL's own list says which distribution a disk is ([`wsl`]), and Docker's
//!   engine what it keeps in its disk, and what of it nothing uses
//!   ([`docker`]).
//!
//! Which files are virtual disks, and whose, the places say
//! (`rules/places.toml`): a place of the `virtual-disks` consumer that names a
//! file is a disk, and its `owner` - or that of the nearest place above it -
//! says what has to stop before it is compacted. See
//! `docs/adr/0015-what-is-inside-a-virtual-disk.md`.

pub mod docker;
pub mod guest;
pub mod vhdx;
pub mod wsl;

use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::consumers::{Classified, Consumer};
use crate::rules::{Risk, RuleSet};
use crate::size::Size;
use crate::tree::{NodeId, ROOT, Tree};
use crate::words;

/// Whose machine a disk is: what has to stop before it is compacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Owner {
    /// Docker Desktop, which runs its engine in WSL.
    Docker,
    /// A WSL distribution.
    Wsl,
    /// A Hyper-V machine.
    HyperV,
}

/// A virtual disk in a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub id: NodeId,
    pub path: PathBuf,
    /// What the file takes on the volume.
    pub size: Size,
    /// What the place that names it calls it, in the middle of a sentence.
    pub name: String,
    pub owner: Option<Owner>,
}

/// The virtual disks of a tree, the largest first: every file a place of
/// virtual disks names.
#[must_use]
pub fn find(rules: &RuleSet, tree: &Tree, classified: &Classified) -> Vec<Disk> {
    let mut disks: Vec<Disk> = classified
        .places()
        .filter(|&(id, place)| rules.places()[place].consumer == Consumer::VirtualDisks && !tree.node(id).is_directory())
        .map(|(id, place)| Disk {
            id,
            path: tree.path_of(id),
            size: tree.node(id).size,
            name: rules.places()[place].name.clone(),
            owner: owner_of(rules, tree, classified, id),
        })
        .collect();
    disks.sort_by(|a, b| b.size.allocated.cmp(&a.size.allocated).then_with(|| tree.scan_order(a.id, b.id)));
    disks
}

/// The owner the place of `id` says, or that of the nearest place above it.
fn owner_of(rules: &RuleSet, tree: &Tree, classified: &Classified, id: NodeId) -> Option<Owner> {
    let mut at = id;
    loop {
        if let Some(owner) = classified.place(at).and_then(|place| rules.places()[place].owner) {
            return Some(owner);
        }
        if at == ROOT {
            return None;
        }
        at = tree.node(at).parent;
    }
}

/// The kinds of disk file midda tells apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    Vhdx,
    Vhd,
    Vmdk,
    Vdi,
    Qcow2,
}

impl Format {
    /// What it is called.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Vhdx => "VHDX",
            Self::Vhd => "VHD",
            Self::Vmdk => "VMDK",
            Self::Vdi => "VDI",
            Self::Qcow2 => "qcow2",
        }
    }

    /// The kind of disk a file is, from its first bytes and its last 512.
    #[must_use]
    pub fn of(start: &[u8], end: &[u8]) -> Option<Self> {
        if vhdx::is_vhdx(start) {
            Some(Self::Vhdx)
        } else if start.starts_with(b"QFI\xfb") {
            Some(Self::Qcow2)
        } else if start.starts_with(b"KDMV") || start.starts_with(b"# Disk DescriptorFile") {
            Some(Self::Vmdk)
        } else if start.get(0x40..0x44) == Some(&[0x7F, 0x10, 0xDA, 0xBE]) {
            Some(Self::Vdi)
        } else if start.starts_with(b"conectix") || end.starts_with(b"conectix") {
            Some(Self::Vhd)
        } else {
            None
        }
    }
}

/// What a disk file was found to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inside {
    /// The disk was read.
    Read(Reading),
    /// A checkpoint: the changes made to another disk since it was taken.
    Checkpoint { capacity: u64, written: u64 },
    /// Not read, and why, to the reader.
    Unread(String),
}

/// A VHDX disk, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// How big the disk is to the machine inside.
    pub capacity: u64,
    /// What of the disk the file holds: blocks the machine wrote and has not
    /// let go of.
    pub written: u64,
    /// What the file is besides the blocks: headers, log, metadata, table.
    pub overhead: u64,
    /// Something has it open and is writing to it.
    pub pending: bool,
    pub contents: guest::Contents,
}

impl Reading {
    /// What compacting `file` gives back now: the room in it no block holds.
    #[must_use]
    pub const fn gives_back(&self, file: Size) -> u64 {
        file.allocated.saturating_sub(self.overhead + self.written)
    }

    /// What the file holds that the filesystems inside do not use: blocks
    /// freed in there that the machine has not let go of. Compacting gives
    /// those back only once it has - after a trim.
    ///
    /// Unsaid while the disk is in use: a mounted filesystem writes its own
    /// count of free blocks now and then, and a count from minutes ago set
    /// against the block table of now would be two moments in one figure.
    #[must_use]
    pub fn untrimmed(&self) -> Option<u64> {
        if self.pending || !self.contents.measured() {
            return None;
        }
        self.contents.used().map(|used| self.written.saturating_sub(used))
    }
}

/// A disk, looked at: what it is and what is inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub format: Option<Format>,
    pub inside: Inside,
    /// What WSL calls it, when it is a distribution's disk.
    pub distribution: Option<String>,
}

/// Looks inside the disk file at `path`. `distributions` are WSL's, to name
/// one of theirs ([`wsl::distributions`]).
#[must_use]
pub fn look(path: &Path, distributions: &[wsl::Distribution]) -> Look {
    let distribution = wsl::name_of(path, distributions).map(str::to_owned);
    let (format, inside) = match read_inside(path) {
        Ok(read) => read,
        Err(why) => (None, Inside::Unread(why)),
    };
    Look { format, inside, distribution }
}

fn read_inside(path: &Path) -> Result<(Option<Format>, Inside), String> {
    // Opened to read and shared every way: a running machine has its disk
    // open, and is not to be kept from it.
    let mut file = File::open(path).map_err(|error| format!("the file could not be opened: {error}"))?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let mut start = vec![0_u8; 512];
    let read = file.read(&mut start).map_err(|error| error.to_string())?;
    start.truncate(read);
    let mut end = vec![0_u8; 512];
    if length >= 512 {
        file.seek(SeekFrom::Start(length - 512)).map_err(|error| error.to_string())?;
        file.read_exact(&mut end).map_err(|error| error.to_string())?;
    }
    let format = Format::of(&start, &end);
    match format {
        Some(Format::Vhdx) => {}
        Some(other) => {
            return Ok((
                format,
                Inside::Unread(format!("midda reads inside VHDX disks, and this is a {} disk", other.name())),
            ));
        }
        None => return Ok((None, Inside::Unread("it is not a disk file midda knows".to_owned()))),
    }
    let mut disk = vhdx::Vhdx::read(file)?;
    if disk.differencing {
        return Ok((
            format,
            Inside::Checkpoint {
                capacity: disk.capacity,
                written: disk.written(),
            },
        ));
    }
    let contents = guest::contents(&mut disk)?;
    Ok((
        format,
        Inside::Read(Reading {
            capacity: disk.capacity,
            written: disk.written(),
            overhead: disk.overhead,
            pending: disk.pending,
            contents,
        }),
    ))
}

/// A disk, what was found in it, and that said to a reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Looked {
    pub disk: Disk,
    pub look: Look,
    pub told: Told,
}

/// Looks inside each of `disks` and tells what is there: reads every file,
/// and WSL's list once.
#[must_use]
pub fn look_all(disks: Vec<Disk>) -> Vec<Looked> {
    let distributions = if disks.is_empty() { Vec::new() } else { wsl::distributions() };
    disks
        .into_iter()
        .map(|disk| {
            let look = look(&disk.path, &distributions);
            let told = tell(&disk, &look);
            Looked { disk, look, told }
        })
        .collect()
}

/// What compacting `disks` gives back together, when it is worth saying of
/// the `whole` they take: the disks worth compacting, as measured inside -
/// the rest, and the disks that could not be read, left out.
#[must_use]
pub fn gives_back(disks: &[Looked], whole: u64) -> Option<u64> {
    let back = worth_compacting(disks);
    notable(back, whole).then_some(back)
}

/// What compacting the disks among `disks` that are worth it gives back.
#[must_use]
pub fn worth_compacting(disks: &[Looked]) -> u64 {
    disks.iter().filter(|disk| disk.told.worth).filter_map(|disk| disk.told.gives_back).sum()
}

/// A disk and what was found in it, said to a reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Told {
    /// What it is, at the start of a sentence: `Docker's disk`.
    pub name: String,
    /// What is inside, as measured.
    pub inside: String,
    /// Why the file is larger than what is inside, and what compacting does.
    pub explain: String,
    /// What compacting gives back now, when that was measured.
    pub gives_back: Option<u64>,
    /// Whether that is worth compacting for: a notable part of the file. A
    /// figure said beside "next to nothing" is not added to anything.
    pub worth: bool,
    /// What has to stop first.
    pub first: Option<String>,
    /// What compacts it, for an administrator PowerShell.
    pub script: Option<String>,
}

/// What compacting gives back is said when it is at least one part in this
/// many of the file - and at least [`LEAST`].
const NOTABLE: u64 = 50;

/// The least that is worth compacting a disk for.
const LEAST: u64 = 256 * 1024 * 1024;

/// Whether `part` of `whole` is worth saying.
pub(crate) const fn notable(part: u64, whole: u64) -> bool {
    part >= LEAST && part.saturating_mul(NOTABLE) >= whole
}

/// The machine inside, as a sentence calls it.
const fn machine(owner: Option<Owner>) -> &'static str {
    match owner {
        Some(Owner::Docker | Owner::Wsl) => "Linux",
        Some(Owner::HyperV) | None => "the machine inside",
    }
}

/// Tells what `disk` holds, from `look`.
#[must_use]
pub fn tell(disk: &Disk, look: &Look) -> Told {
    let file = disk.size.allocated;
    let mut told = Told {
        name: capitalised(&disk.name),
        inside: String::new(),
        explain: String::new(),
        gives_back: None,
        worth: false,
        first: None,
        script: None,
    };
    match &look.inside {
        Inside::Read(reading) => {
            told.inside = inside_of(reading);
            let back = reading.gives_back(disk.size);
            told.gives_back = Some(back);
            told.worth = notable(back, file);
            let inside = machine(disk.owner);
            told.explain = format!(
                "A disk file grows as {inside} writes, and does not shrink when it deletes. The file holds {} of the disk",
                words::bytes(reading.written)
            );
            if told.worth {
                let _ = write!(told.explain, "; compacting gives back the room no block holds - about {}.", words::bytes(back));
            } else {
                told.explain.push_str(" and little else: compacting would give back next to nothing.");
            }
            if let Some(untrimmed) = reading.untrimmed().filter(|&untrimmed| notable(untrimmed, file)) {
                let _ = write!(
                    told.explain,
                    " Another {} is room {inside} freed and still holds: it comes back once {inside} lets go of it with a trim.",
                    words::bytes(untrimmed)
                );
            }
            if reading.pending {
                told.explain
                    .push_str(" It is in use as midda reads it: the figures are of its last settled write.");
            }
            told.first = first_step(disk.owner);
            told.script = Some(compaction(&disk.path, disk.owner));
        }
        Inside::Checkpoint { capacity, written } => {
            told.inside = format!(
                "A checkpoint of a {} disk: what changed since it was taken, {}.",
                words::bytes(*capacity),
                words::bytes(*written)
            );
            told.explain =
                "What is inside is in the disk it was taken from. Merging or deleting the checkpoint where it was made gives its room back.".to_owned();
        }
        Inside::Unread(why) => {
            told.inside = format!("midda cannot see inside it: {why}.");
            match look.format {
                Some(Format::Vhd) => {
                    told.explain =
                        "A disk file grows as the machine inside writes, and does not shrink when it deletes: compacting gives back what it has let go of."
                            .to_owned();
                    told.first = first_step(disk.owner);
                    told.script = Some(compaction(&disk.path, disk.owner));
                }
                Some(Format::Vdi) => {
                    told.explain = "VirtualBox compacts its own disks once the machine is off.".to_owned();
                    told.script = Some(format!("VBoxManage modifymedium disk \"{}\" --compact", disk.path.display()));
                }
                Some(Format::Vmdk | Format::Qcow2) => {
                    told.explain = "The program that made it compacts it, once the machine is off.".to_owned();
                }
                Some(Format::Vhdx) | None => {}
            }
        }
    }
    if let Some(name) = &look.distribution {
        let _ = write!(told.inside, " WSL calls it {name}.");
    }
    told
}

/// The filesystems of a disk, in a sentence.
fn inside_of(reading: &Reading) -> String {
    let volumes = &reading.contents.volumes;
    if volumes.is_empty() {
        return format!("A disk of {} with no filesystem midda can read.", words::bytes(reading.capacity));
    }
    let said: Vec<String> = volumes
        .iter()
        .map(|volume| match (volume.filesystem, volume.used) {
            (Some(filesystem), Some(used)) => format!(
                "{} {filesystem} filesystem of {}, {} of it in use",
                article(filesystem),
                words::bytes(volume.capacity),
                words::bytes(used)
            ),
            (Some(filesystem), None) => format!("{} {filesystem} filesystem of {}", article(filesystem), words::bytes(volume.capacity)),
            (None, _) => format!("{} midda cannot read", words::bytes(volume.capacity)),
        })
        .collect();
    format!("Inside: {}.", said.join("; "))
}

/// `a` or `an`, as the name is said: ext4 and NTFS start with a vowel's sound.
fn article(filesystem: &str) -> &'static str {
    if filesystem.starts_with(['e', 'N']) { "an" } else { "a" }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// What has to stop before a disk of `owner` is compacted, beyond what the
/// script does itself.
fn first_step(owner: Option<Owner>) -> Option<String> {
    match owner {
        Some(Owner::Docker) => Some("Quit Docker Desktop first: it starts WSL again while it runs.".to_owned()),
        Some(Owner::Wsl) => None,
        Some(Owner::HyperV) => Some("Turn the machine off in Hyper-V Manager first.".to_owned()),
        None => Some("Stop the machine that uses it first.".to_owned()),
    }
}

/// The commands that compact the disk at `path`, for an administrator
/// PowerShell.
///
/// One way for every edition of Windows: `diskpart` is on all of them, where
/// `Optimize-VHD` needs Hyper-V - which a Hyper-V machine's disk implies, so
/// that one is compacted with it. WSL is shut down first: it holds every
/// distribution's disk while any runs.
#[must_use]
pub fn compaction(path: &Path, owner: Option<Owner>) -> String {
    let path = path.display();
    match owner {
        Some(Owner::HyperV) => format!("Optimize-VHD -Path \"{path}\" -Mode Full"),
        Some(Owner::Docker | Owner::Wsl) => {
            format!("wsl --shutdown\n@'\nselect vdisk file=\"{path}\"\nattach vdisk readonly\ncompact vdisk\ndetach vdisk\n'@ | diskpart")
        }
        None => format!("@'\nselect vdisk file=\"{path}\"\nattach vdisk readonly\ncompact vdisk\ndetach vdisk\n'@ | diskpart"),
    }
}

/// One kind of thing Docker keeps that nothing uses, and how it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prune {
    /// What it is: `Build cache`.
    pub what: &'static str,
    /// How many of them nothing uses, and what they take.
    pub count: u64,
    pub bytes: u64,
    pub risk: Risk,
    /// What removes them - or, for what nothing makes again, what lists them
    /// to look at first.
    pub command: &'static str,
    pub explain: &'static str,
}

/// What Docker keeps that nothing uses, the safest first, each with how it
/// goes and what is lost.
#[must_use]
pub fn prunes(usage: &docker::Usage) -> Vec<Prune> {
    let tagged_count = usage.images.idle.saturating_sub(usage.untagged.idle);
    let tagged_bytes = usage.images.reclaimable.saturating_sub(usage.untagged.reclaimable);
    [
        Prune {
            what: "Build cache",
            count: usage.build_cache.idle,
            bytes: usage.build_cache.reclaimable,
            risk: Risk::Safe,
            command: "docker builder prune --all",
            explain: "What builds kept to be faster next time. The next build makes it again, once more slowly.",
        },
        Prune {
            what: "Images without a name",
            count: usage.untagged.idle,
            bytes: usage.untagged.reclaimable,
            risk: Risk::Safe,
            command: "docker image prune",
            explain: "Left behind when a build or a pull gave their name to a newer image. Nothing can ask for them by name.",
        },
        Prune {
            what: "Images no container uses",
            count: tagged_count,
            bytes: tagged_bytes,
            risk: Risk::Caution,
            command: "docker image prune --all",
            explain: "Pulled again when something asks for them. An image built here and never pushed has to be built again.",
        },
        Prune {
            what: "Stopped containers",
            count: usage.containers.idle,
            bytes: usage.containers.reclaimable,
            risk: Risk::Caution,
            command: "docker container prune",
            explain: "What a stopped container changed in its own files goes with it; its image stays.",
        },
        Prune {
            what: "Volumes no container uses",
            count: usage.volumes.idle,
            bytes: usage.volumes.reclaimable,
            risk: Risk::Caution,
            command: "docker volume ls --filter dangling=true",
            explain: "A volume is data a container kept - a database, uploads - and nothing makes it again. List them, and remove each you know with docker volume rm.",
        },
    ]
    .into_iter()
    .filter(|prune| prune.count > 0)
    .collect()
}

/// What follows a prune: the disk does not give the room back by itself.
pub const AFTER_PRUNE: &str = "Pruning frees room inside Docker's disk; Windows gets it back when the disk is compacted.";

#[cfg(test)]
mod tests;
