use std::path::PathBuf;

use super::guest::tests::ext4;
use super::vhdx::tests::Image;
use super::*;
use crate::consumers::classify;
use crate::tree::{Kind, Node};

/// The top of a volume as the platform writes it.
const VOLUME: &str = if cfg!(windows) { "C:/" } else { "/" };

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// A tree at `root` from paths: a trailing `/` is a folder, anything else a
/// file of `bytes`.
fn tree_of(root: &str, entries: &[(&str, u64)]) -> Tree {
    let mut tree = Tree::new(PathBuf::from(root), Some(4096));
    let mut sorted: Vec<(&str, u64)> = entries.to_vec();
    sorted.sort_by_key(|(path, _)| path.trim_end_matches('/').matches('/').count());
    for (path, bytes) in sorted {
        let folder = path.ends_with('/');
        let path = path.trim_end_matches('/');
        let (above, name) = path.rsplit_once('/').unwrap_or(("", path));
        let parent = above
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(ROOT, |at, part| tree.child(at, part).unwrap_or_else(|| panic!("{part} comes before {path}")));
        let node = if folder {
            Node::new(name.into(), parent, Kind::Directory)
        } else {
            Node {
                size: Size::flat(bytes),
                ..Node::new(name.into(), parent, Kind::File)
            }
        };
        tree.insert(parent, node);
    }
    tree.roll_up();
    tree
}

fn disks_of(tree: &Tree) -> Vec<Disk> {
    let rules = RuleSet::shipped();
    find(rules, tree, &classify(rules, tree, &rules.survey(tree)))
}

/// What a disk is called and whose it is, by its path below the root.
fn named(tree: &Tree, disks: &[Disk]) -> Vec<(String, String, Option<Owner>)> {
    let root = tree.root_path().to_string_lossy().replace('\\', "/");
    disks
        .iter()
        .map(|disk| {
            let path = disk.path.to_string_lossy().replace('\\', "/");
            let below = path.strip_prefix(&root).unwrap_or(&path).trim_start_matches('/').to_owned();
            (below, disk.name.clone(), disk.owner)
        })
        .collect()
}

#[test]
fn every_file_a_place_of_virtual_disks_names_is_a_disk_with_its_owner() {
    let tree = tree_of(
        VOLUME,
        &[
            ("Users/", 0),
            ("Users/me/", 0),
            ("Users/me/AppData/", 0),
            ("Users/me/AppData/Local/", 0),
            ("Users/me/AppData/Local/Docker/", 0),
            ("Users/me/AppData/Local/Docker/wsl/", 0),
            ("Users/me/AppData/Local/Docker/wsl/main/", 0),
            ("Users/me/AppData/Local/Docker/wsl/main/ext4.vhdx", 150 * MIB),
            ("Users/me/AppData/Local/Docker/wsl/disk/", 0),
            ("Users/me/AppData/Local/Docker/wsl/disk/docker_data.vhdx", 50 * GIB),
            ("Users/me/AppData/Local/wsl/", 0),
            ("Users/me/AppData/Local/wsl/{1234}/", 0),
            ("Users/me/AppData/Local/wsl/{1234}/ext4.vhdx", 12 * GIB),
            ("Users/me/AppData/Local/wsl/{1234}/settings.json", 1),
            ("Users/me/Downloads/", 0),
            ("Users/me/Downloads/appliance.vhdx", 3 * GIB),
            ("Users/me/Downloads/notes.txt", 10),
            ("ProgramData/", 0),
            ("ProgramData/Microsoft/", 0),
            ("ProgramData/Microsoft/Windows/", 0),
            ("ProgramData/Microsoft/Windows/Virtual Hard Disks/", 0),
            ("ProgramData/Microsoft/Windows/Virtual Hard Disks/server.vhdx", 30 * GIB),
            ("ProgramData/Microsoft/Windows/Virtual Hard Disks/server_1A2B.avhdx", GIB),
        ],
    );
    let found = disks_of(&tree);
    let expected = [
        ("Users/me/AppData/Local/Docker/wsl/disk/docker_data.vhdx", "Docker's disk", Some(Owner::Docker)),
        (
            "ProgramData/Microsoft/Windows/Virtual Hard Disks/server.vhdx",
            "a virtual disk",
            Some(Owner::HyperV),
        ),
        ("Users/me/AppData/Local/wsl/{1234}/ext4.vhdx", "a WSL distribution's disk", Some(Owner::Wsl)),
        ("Users/me/Downloads/appliance.vhdx", "a virtual disk", None),
        (
            "ProgramData/Microsoft/Windows/Virtual Hard Disks/server_1A2B.avhdx",
            "a virtual disk",
            Some(Owner::HyperV),
        ),
        ("Users/me/AppData/Local/Docker/wsl/main/ext4.vhdx", "Docker's system disk", Some(Owner::Docker)),
    ]
    .map(|(path, name, owner)| (path.to_owned(), name.to_owned(), owner));
    assert_eq!(
        named(&tree, &found),
        expected,
        "the largest first; the settings file beside a disk is not one, and a disk in Downloads is nobody's"
    );
}

#[test]
fn an_owner_belongs_to_a_place_of_virtual_disks() {
    let place = |consumer: &str| format!("[[place]]\nid = \"x\"\nname = \"x\"\nconsumer = \"{consumer}\"\npaths = [\"*.img\"]\nowner = \"wsl\"\n");
    assert!(RuleSet::parse(&place("virtual-disks")).is_ok());
    assert!(RuleSet::parse(&place("games")).expect_err("refused").contains("virtual-disks"));
    let unknown = place("virtual-disks").replace("\"wsl\"", "\"parallels\"");
    assert!(RuleSet::parse(&unknown).is_err(), "an owner midda does not know");
}

/// A disk at `path` of `file` bytes, owned by `owner`.
fn disk(path: &Path, file: u64, owner: Option<Owner>) -> Disk {
    Disk {
        id: 1,
        path: path.to_owned(),
        size: Size::flat(file),
        name: "a WSL distribution's disk".to_owned(),
        owner,
    }
}

/// A VHDX of 64 MiB with an ext4 filesystem of `blocks` 4 KiB blocks, `free`
/// of them free - its superblock in the first block of the disk.
fn ext4_image(blocks: u64, free: u64) -> Image {
    let mut first = vec![0_u8; 2048];
    ext4(&mut first, 0, blocks, free, true, "root");
    Image::new(64 * MIB).write(0, &first)
}

#[test]
fn a_disk_file_is_read_to_its_filesystem_and_its_block_table() {
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("ext4.vhdx");
    std::fs::write(&path, ext4_image(16_384, 16_000).bytes()).expect("write the disk");
    let distributions = [wsl::Distribution {
        name: "Ubuntu".to_owned(),
        disk: path.clone(),
    }];
    let look = look(&path, &distributions);
    assert_eq!(look.format, Some(Format::Vhdx));
    assert_eq!(look.distribution.as_deref(), Some("Ubuntu"));
    let Inside::Read(reading) = &look.inside else {
        panic!("read: {look:?}");
    };
    assert_eq!(reading.capacity, 64 * MIB);
    assert_eq!(reading.written, MIB, "one block written: the one with the superblock");
    let volume = &reading.contents.volumes[0];
    assert_eq!((volume.filesystem, volume.label.as_deref()), (Some("ext4"), Some("root")));
    assert_eq!(volume.used, Some((384 - 5) * 4096));
}

#[test]
fn what_is_not_a_vhdx_is_named_and_not_read_inside() {
    let folder = tempfile::tempdir().expect("a folder");
    let write = |name: &str, bytes: &[u8]| {
        let path = folder.path().join(name);
        std::fs::write(&path, bytes).expect("write");
        look(&path, &[])
    };
    let vmdk = write("disk.vmdk", b"KDMV and the rest of a sparse extent");
    assert_eq!(vmdk.format, Some(Format::Vmdk));
    assert_eq!(vmdk.inside, Inside::Unread("midda reads inside VHDX disks, and this is a VMDK disk".to_owned()));
    let mut fixed = vec![0_u8; 4096];
    fixed[4096 - 512..4096 - 504].copy_from_slice(b"conectix");
    assert_eq!(write("fixed.vhd", &fixed).format, Some(Format::Vhd), "a fixed VHD says so at its end");
    let mut vdi = vec![0_u8; 512];
    vdi[0x40..0x44].copy_from_slice(&[0x7F, 0x10, 0xDA, 0xBE]);
    assert_eq!(write("box.vdi", &vdi).format, Some(Format::Vdi));
    assert_eq!(write("image.qcow2", b"QFI\xfb\0\0\0\x03").format, Some(Format::Qcow2));
    let named_so = write("not-really.vhdx", b"just text");
    assert_eq!(
        (named_so.format, named_so.inside),
        (None, Inside::Unread("it is not a disk file midda knows".to_owned()))
    );
    let missing = look(&folder.path().join("gone.vhdx"), &[]);
    assert!(matches!(missing.inside, Inside::Unread(why) if why.contains("could not be opened")));
}

#[test]
fn a_checkpoint_is_told_as_one() {
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("server_1A2B.avhdx");
    let mut image = Image::new(64 * MIB).write(0, b"changed");
    image.differencing = true;
    std::fs::write(&path, image.bytes()).expect("write");
    let look = look(&path, &[]);
    assert_eq!(
        look.inside,
        Inside::Checkpoint {
            capacity: 64 * MIB,
            written: MIB
        }
    );
    let told = tell(&disk(&path, 4 * MIB, Some(Owner::HyperV)), &look);
    assert!(
        told.inside
            .starts_with("A checkpoint of a 64 MB disk: what changed since it was taken, 1.0 MB."),
        "{told:?}"
    );
    assert_eq!((told.gives_back, told.script), (None, None), "a checkpoint is merged, not compacted");
}

/// A reading of a disk the file holds `written` of, with filesystems that use
/// `used`.
fn reading(written: u64, used: u64, pending: bool) -> Look {
    Look {
        format: Some(Format::Vhdx),
        inside: Inside::Read(Reading {
            capacity: 1024 * GIB,
            written,
            overhead: 12 * MIB,
            pending,
            contents: guest::Contents {
                volumes: vec![guest::Volume {
                    filesystem: Some("ext4"),
                    label: None,
                    at: 0,
                    capacity: 1024 * GIB,
                    used: Some(used),
                }],
            },
        }),
        distribution: Some("Ubuntu".to_owned()),
    }
}

#[test]
fn compacting_gives_back_the_room_no_block_holds() {
    let path = PathBuf::from(r"C:\Users\me\AppData\Local\wsl\{1234}\ext4.vhdx");
    // A 40 GB file that holds 12 GB of blocks: 28 GB of room, less the file's
    // own 12 MB.
    let told = tell(&disk(&path, 40 * GIB, Some(Owner::Wsl)), &reading(12 * GIB, 12 * GIB - 100 * MIB, false));
    assert_eq!((told.gives_back, told.worth), (Some(28 * GIB - 12 * MIB), true));
    assert_eq!(told.name, "A WSL distribution's disk");
    assert_eq!(told.inside, "Inside: an ext4 filesystem of 1.0 TB, 12 GB of it in use. WSL calls it Ubuntu.");
    assert!(
        told.explain
            .contains("The file holds 12 GB of the disk; compacting gives back the room no block holds - about 28 GB."),
        "{told:?}"
    );
    assert!(
        !told.explain.contains("trim"),
        "a hundred megabytes of a 40 GB file is not worth a sentence: {told:?}"
    );
    let script = told.script.expect("a script");
    assert!(script.starts_with("wsl --shutdown\n"), "{script}");
    assert!(script.contains(&format!("select vdisk file=\"{}\"", path.display())), "{script}");
    assert!(script.contains("attach vdisk readonly\ncompact vdisk\ndetach vdisk"), "{script}");
    assert_eq!(told.first, None, "the script shuts WSL down itself");
}

#[test]
fn a_disk_that_holds_only_its_blocks_is_said_to_give_back_next_to_nothing() {
    let path = PathBuf::from(r"C:\Users\me\AppData\Local\Docker\wsl\disk\docker_data.vhdx");
    let told = tell(&disk(&path, 50 * GIB + 700 * MIB, Some(Owner::Docker)), &reading(50 * GIB, 38 * GIB, true));
    assert_eq!((told.gives_back, told.worth), (Some(700 * MIB - 12 * MIB), false), "measured, and not worth it");
    assert!(told.explain.contains("compacting would give back next to nothing"), "{told:?}");
    // In use: the filesystem's count may be minutes old, and 12 GB "freed and
    // still held" would be a guess from two moments.
    assert!(!told.explain.contains("trim"), "{told:?}");
    assert!(told.explain.contains("figures are of its last settled write"), "{told:?}");
    assert_eq!(told.first.as_deref(), Some("Quit Docker Desktop first: it starts WSL again while it runs."));
}

#[test]
fn room_freed_inside_and_not_let_go_of_is_said_when_the_disk_is_at_rest() {
    let path = PathBuf::from(r"C:\vm\data.vhdx");
    let told = tell(&disk(&path, 50 * GIB, None), &reading(48 * GIB, 30 * GIB, false));
    assert!(
        told.explain.contains("Another 18 GB is room the machine inside freed and still holds"),
        "{told:?}"
    );
    assert_eq!(told.first.as_deref(), Some("Stop the machine that uses it first."));
    let script = told.script.expect("a script");
    assert!(script.starts_with("@'\nselect vdisk"), "nobody's disk: no WSL to shut down: {script}");
}

#[test]
fn a_hyper_v_disk_is_compacted_with_hyper_vs_own_command() {
    let path = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Virtual Hard Disks\server.vhdx");
    let told = tell(&disk(&path, 30 * GIB, Some(Owner::HyperV)), &reading(10 * GIB, 9 * GIB, false));
    assert_eq!(
        told.script.as_deref(),
        Some(format!("Optimize-VHD -Path \"{}\" -Mode Full", path.display()).as_str())
    );
    assert_eq!(told.first.as_deref(), Some("Turn the machine off in Hyper-V Manager first."));
}

#[test]
fn what_docker_keeps_that_nothing_uses_is_listed_safest_first_and_only_what_there_is() {
    let usage = docker::Usage {
        images: docker::Part {
            count: 10,
            size: 20 * GIB,
            idle: 8,
            reclaimable: 15 * GIB,
        },
        untagged: docker::Part {
            count: 2,
            size: GIB,
            idle: 2,
            reclaimable: GIB,
        },
        containers: docker::Part {
            count: 1,
            size: 100,
            idle: 0,
            reclaimable: 0,
        },
        volumes: docker::Part {
            count: 3,
            size: 5 * GIB,
            idle: 1,
            reclaimable: 2 * GIB,
        },
        build_cache: docker::Part {
            count: 40,
            size: 9 * GIB,
            idle: 40,
            reclaimable: 9 * GIB,
        },
    };
    let prunes = prunes(&usage);
    let listed: Vec<(&str, u64, u64, Risk, &str)> = prunes
        .iter()
        .map(|prune| (prune.what, prune.count, prune.bytes, prune.risk, prune.command))
        .collect();
    assert_eq!(
        listed,
        [
            ("Build cache", 40, 9 * GIB, Risk::Safe, "docker builder prune --all"),
            ("Images without a name", 2, GIB, Risk::Safe, "docker image prune"),
            ("Images no container uses", 6, 14 * GIB, Risk::Caution, "docker image prune --all"),
            (
                "Volumes no container uses",
                1,
                2 * GIB,
                Risk::Caution,
                "docker volume ls --filter dangling=true"
            ),
        ],
        "the images without a name are not counted twice; no container is stopped"
    );
    assert!(
        prunes[3].explain.contains("nothing makes it again"),
        "a volume is data: it is listed, not pruned"
    );
}

#[test]
fn what_compacting_gives_back_together_is_the_disks_worth_it() {
    let path = PathBuf::from(r"C:\vm\a.vhdx");
    let looked = |file: u64, written: u64| {
        let disk = disk(&path, file, None);
        let look = reading(written, written, false);
        let told = tell(&disk, &look);
        Looked { disk, look, told }
    };
    // 28 GB from one, and 300 MB of a 50 GB file from the other: a figure the
    // page calls next to nothing is not added in.
    let disks = [looked(40 * GIB, 12 * GIB), looked(50 * GIB + 312 * MIB, 50 * GIB)];
    assert!(!disks[1].told.worth);
    assert_eq!(worth_compacting(&disks), 28 * GIB - 12 * MIB);
    assert_eq!(gives_back(&disks, 90 * GIB), Some(28 * GIB - 12 * MIB));
    assert_eq!(gives_back(&disks[1..], 50 * GIB), None);
}
