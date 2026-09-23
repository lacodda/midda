//! The rule that lets two scanners exist: the walk and the MFT read produce the
//! same tree for the same directory, node for node.
//!
//! The MFT read needs an elevated process on an NTFS volume. Where it cannot
//! run, the test says so on stderr and passes — unless `MIDDA_REQUIRE_MFT` is
//! set to something, which CI does on its Windows runner (elevated by default
//! there), so the agreement is checked on every push rather than whenever
//! someone remembers to open an administrator's terminal.

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use midda_core::{Kind, MftScanner, Progress, Scanner, Traits, Tree, WalkScanner};

/// Whether this process may run the MFT scanner on `root`; if not, whether that
/// is allowed to pass.
fn mft_can_read(root: &Path) -> bool {
    if MftScanner::new().available(root) {
        return true;
    }
    assert!(
        std::env::var_os("MIDDA_REQUIRE_MFT").is_none_or(|value| value.is_empty()),
        "MIDDA_REQUIRE_MFT is set, but the MFT scanner cannot read {} — the process is not elevated or the volume is not NTFS",
        root.display()
    );
    eprintln!(
        "SKIPPED: the MFT scanner cannot read {} here (not elevated, or not NTFS). Run elevated, or in CI, to check the agreement.",
        root.display()
    );
    false
}

fn write(path: &Path, bytes: usize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create the parent");
    }
    fs::write(path, vec![b'a'; bytes]).expect("write a fixture file");
}

fn run(program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .status()
        .unwrap_or_else(|error| panic!("{program} did not start: {error}"));
    assert!(status.success(), "{program} {args:?} failed with {status}");
}

/// A directory holding one of everything the two scanners could read
/// differently. Synthetic, like every fixture in the line.
fn fixture(base: &Path) -> PathBuf {
    let root = base.join("root");

    // Plain files across the sizes NTFS stores differently: empty, resident in
    // the record, one cluster and a bit, and large.
    write(&root.join("zero.bin"), 0);
    write(&root.join("tiny.txt"), 100);
    write(&root.join("spill.bin"), 5000);
    write(&root.join("project/src/main.rs"), 1000);
    write(&root.join("project/target/build.bin"), 1 << 20);
    fs::create_dir_all(root.join("empty")).expect("create an empty directory");

    // Names whose order a case-sensitive or a byte sort would get wrong, and
    // names outside ASCII.
    for name in ["Zeta.txt", "alpha.txt", "Beta.txt", "ünïcødé.txt", "with space.txt", "UPPER", "lower"] {
        write(&root.join("names").join(name), 10);
    }

    // One file under three names, at different depths, plus a fourth name
    // outside the scanned root: the bytes are counted once, under the
    // shallowest name, and the fourth name is not the root's business.
    write(&root.join("store/pkg.bin"), 60_000);
    fs::create_dir_all(root.join("project/deps")).expect("create deps");
    fs::create_dir_all(root.join("deep/a/b")).expect("create deep");
    fs::hard_link(root.join("store/pkg.bin"), root.join("project/deps/pkg.bin")).expect("link");
    fs::hard_link(root.join("store/pkg.bin"), root.join("deep/a/b/pkg.bin")).expect("link");
    write(&root.join("half-outside.bin"), 7000);
    fs::hard_link(root.join("half-outside.bin"), base.join("outside.bin")).expect("link outside");
    // Written through one name after the others exist: the other names'
    // directory entries may still say the old size.
    fs::write(root.join("project/deps/pkg.bin"), vec![b'b'; 90_000]).expect("grow through a second name");

    // A junction: an entry that costs nothing and is never followed.
    let junction = root.join("junction");
    run(
        "cmd",
        &["/c", "mklink", "/J", &junction.to_string_lossy(), &root.join("project").to_string_lossy()],
    );

    // NTFS compression, which makes the two sizes differ the other way.
    write(&root.join("compressed/data.bin"), 400_000);
    run("compact", &["/c", "/q", &root.join("compressed/data.bin").to_string_lossy()]);

    // Windows' own file compression, which keeps the bytes in a named stream
    // and hides that from everyone but a reader of the MFT. A CompactOS system
    // volume is made of these.
    fs::write(root.join("overlaid.bin"), b"abcdefgh".repeat(100_000)).expect("write a compressible file");
    run("compact", &["/c", "/q", "/exe:xpress4k", &root.join("overlaid.bin").to_string_lossy()]);

    // A sparse file: a megabyte long, a few bytes held.
    let sparse = root.join("sparse.bin");
    write(&sparse, 10);
    run("fsutil", &["sparse", "setflag", &sparse.to_string_lossy()]);
    fs::OpenOptions::new()
        .write(true)
        .open(&sparse)
        .and_then(|file| file.set_len(1 << 20))
        .expect("extend the sparse file");

    root
}

/// Asserts two trees are the same, node for node, naming the first place they
/// differ by path rather than by index.
fn assert_same(walked: &Tree, read: &Tree) {
    assert_eq!(walked.root_path(), read.root_path());
    assert_eq!(walked.cluster_bytes(), read.cluster_bytes(), "cluster size");
    assert!(walked.skipped().is_empty(), "the walk skipped {:?}", walked.skipped());
    assert!(read.skipped().is_empty(), "the MFT read skipped {:?}", read.skipped());

    let paths_walked: Vec<_> = walked.nodes().map(|(id, _)| walked.path_of(id)).collect();
    let paths_read: Vec<_> = read.nodes().map(|(id, _)| read.path_of(id)).collect();
    assert_eq!(
        paths_walked, paths_read,
        "the two scanners found different entries, or listed them in a different order"
    );

    for ((id, a), (_, b)) in walked.nodes().zip(read.nodes()) {
        let path = walked.path_of(id);
        let at = path.display();
        assert_eq!(a.kind, b.kind, "kind of {at}");
        assert_eq!(a.parent, b.parent, "parent of {at}");
        assert_eq!(a.children, b.children, "children of {at}");
        assert_eq!(a.size, b.size, "size of {at}");
        assert_eq!(a.traits, b.traits, "traits of {at}");
        assert_eq!(a.links, b.links, "link count of {at}");
        assert_eq!(a.identity, b.identity, "identity of {at}");
        assert_eq!(a.entries, b.entries, "entries under {at}");
        assert_eq!(a.modified, b.modified, "modified time of {at}");
        assert_eq!(a.subtree_modified, b.subtree_modified, "subtree modified time of {at}");
    }
    assert_eq!(walked.shared(), read.shared(), "shared bytes");
}

fn node<'t>(tree: &'t Tree, path: &str) -> &'t midda_core::Node {
    let wanted = tree.root_path().join(path);
    tree.nodes()
        .find(|(id, _)| tree.path_of(*id) == wanted)
        .map(|(_, node)| node)
        .unwrap_or_else(|| panic!("{path} is not in the tree"))
}

#[test]
fn the_walk_and_the_mft_read_the_same_tree() {
    let base = tempfile::tempdir().expect("a temporary directory");
    let root = fixture(base.path());
    if !mft_can_read(&root) {
        return;
    }

    let walked = WalkScanner::new().scan(&root, &Progress::new()).expect("the walk succeeds");
    let read = MftScanner::new().scan(&root, &Progress::new()).expect("the MFT read succeeds");
    assert_eq!(walked.scanned_by(), "walk");
    assert_eq!(read.scanned_by(), "mft");

    // The fixture produced what it claims to — a case that silently did not
    // happen would make the agreement below agree about nothing.
    let owner = node(&read, "store/pkg.bin");
    assert_eq!(owner.links, Some(3));
    assert!(owner.traits.has(Traits::LINKED | Traits::COUNTED_HERE), "the shallowest name keeps the bytes");
    assert_eq!(owner.size.logical, 90_000, "the length written through another name");
    assert!(node(&read, "project/deps/pkg.bin").traits.is_shared_name());
    assert_eq!(read.shared().shared_names, 2);
    assert_eq!(node(&read, "half-outside.bin").links, Some(2));
    assert!(
        !node(&read, "half-outside.bin").traits.is_shared_name(),
        "its other name is outside the scan, so this one keeps the bytes"
    );
    assert_eq!(node(&read, "junction").kind, Kind::File);
    assert!(node(&read, "compressed/data.bin").traits.has(Traits::COMPRESSED));
    assert!(node(&read, "sparse.bin").traits.has(Traits::SPARSE));
    assert_eq!(node(&read, "tiny.txt").size.allocated, 104, "resident: the length rounded to eight");
    let overlaid = node(&read, "overlaid.bin").size;
    assert!(
        overlaid.allocated > 0 && overlaid.allocated < overlaid.logical,
        "an overlaid file occupies its compressed stream: {overlaid:?}"
    );

    assert_same(&walked, &read);
}

#[test]
fn a_scan_is_read_by_the_mft_when_it_can_be() {
    let base = tempfile::tempdir().expect("a temporary directory");
    write(&base.path().join("one.bin"), 10);
    if !mft_can_read(base.path()) {
        return;
    }
    let tree = midda_core::scan(base.path(), &Progress::new()).expect("the scan succeeds");
    assert_eq!(tree.scanned_by(), "mft");
    assert_eq!(tree.fallback(), None);
}

/// The same agreement on a real folder, for a live check before a release:
/// `MIDDA_AGREE_ON=C:\some\folder cargo test --test scanners_agree -- --ignored`
/// from an administrator's terminal.
///
/// Not part of the gate: a real folder changes while two scans read it, and
/// the walk is refused things the MFT read is not.
#[test]
#[ignore = "reads a real folder named by MIDDA_AGREE_ON, elevated"]
fn the_two_scanners_agree_on_a_real_folder() {
    let root = PathBuf::from(std::env::var_os("MIDDA_AGREE_ON").expect("MIDDA_AGREE_ON names the folder to compare"));
    assert!(MftScanner::new().available(&root), "run elevated, on NTFS");

    let started = std::time::Instant::now();
    let walked = WalkScanner::new().scan(&root, &Progress::new()).expect("the walk succeeds");
    let walk_took = started.elapsed();
    let started = std::time::Instant::now();
    let read = MftScanner::new().scan(&root, &Progress::new()).expect("the MFT read succeeds");
    let mft_took = started.elapsed();
    eprintln!(
        "walk: {} entries, {} bytes on disk, {walk_took:?}; mft: {} entries, {} bytes on disk, {mft_took:?}",
        walked.root().entries,
        walked.root().size.allocated,
        read.root().entries,
        read.root().size.allocated
    );

    if walked.skipped().is_empty() {
        assert_same(&walked, &read);
    } else {
        // The walk was refused somewhere, so the trees cannot match node for
        // node. Every *file* both of them saw must still match: a total that
        // differs only because the walk was refused is fine; a file measured
        // differently is not, and totals alone once hid 17 GB of that.
        eprintln!("the walk skipped {} entries; comparing the files both saw", walked.skipped().len());
        assert_files_agree(&walked, &read);
    }
}

/// Every file present in both trees and unchanged between the two scans has
/// the same size, traits and link count.
///
/// A live volume moves while it is read twice: on a CI runner's system volume
/// the registry logs, the event logs and every database's write-ahead log
/// changed in the minutes the walk took. A file whose write time differs
/// between the trees was written in between and is listed, not failed; a file
/// with one write time and two sizes is the scanners disagreeing.
fn assert_files_agree(walked: &Tree, read: &Tree) {
    type Seen = (midda_core::Size, Traits, Option<u32>, Option<std::time::SystemTime>);
    let files = |tree: &Tree| -> std::collections::HashMap<PathBuf, Seen> {
        tree.nodes()
            .filter(|(_, node)| node.kind == Kind::File)
            .map(|(id, node)| (tree.path_of(id), (node.size, node.traits, node.links, node.modified)))
            .collect()
    };
    let from_walk = files(walked);
    let from_mft = files(read);

    let mut differ: Vec<_> = from_walk
        .iter()
        .filter_map(|(path, walk)| {
            let mft = from_mft.get(path)?;
            (walk.0 != mft.0 || walk.1 != mft.1 || walk.2 != mft.2).then_some((path, *walk, *mft))
        })
        .collect();
    differ.sort_by_key(|(_, walk, mft)| std::cmp::Reverse(walk.0.allocated.abs_diff(mft.0.allocated)));

    let (changed, disagree): (Vec<_>, Vec<_>) = differ.into_iter().partition(|(_, walk, mft)| walk.3 != mft.3);
    let common = from_walk.keys().filter(|path| from_mft.contains_key(*path)).count();
    eprintln!(
        "{common} files in both trees; {} written between the scans, {} measured differently while unchanged",
        changed.len(),
        disagree.len()
    );
    for (path, walk, mft) in disagree.iter().chain(changed.iter()).take(25) {
        eprintln!(
            "  {}: walk {:?} traits {:#04x} links {:?} | mft {:?} traits {:#04x} links {:?}",
            path.display(),
            walk.0,
            walk.1.bits(),
            walk.2,
            mft.0,
            mft.1.bits(),
            mft.2
        );
    }
    assert!(
        disagree.is_empty(),
        "{} unchanged files measured differently by the two scanners",
        disagree.len()
    );
}
