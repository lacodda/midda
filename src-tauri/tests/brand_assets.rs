//! Guards the brand mark against being cut from the wrong level.
//!
//! The line's identity has three masters — S, M and L — and which one a raster
//! uses is decided by how big it is, never by what it is for. Getting that
//! wrong is invisible in a diff and obvious on screen: the S tile is a hex
//! filled with colour, which is the only thing that reads at 16 px and a
//! coloured blob at 48.
//!
//! It has shipped three times in this line already — kasl's docs header,
//! kilna's desktop icon, lyrid in three places at once — and every time the
//! owner's eye caught it, several releases late. midda's own generator took the
//! S tile for all seven sizes in the `.ico` until this test was written. Hence a
//! test rather than a note.
//!
//! These read files in the repository on purpose. An earlier attempt in the
//! line compared the SVGs against a registry kept in a notes vault, which CI
//! does not have: the check passed by finding nothing to check.

use std::fs;
use std::path::{Path, PathBuf};

/// The repository root: this crate is `src-tauri`, one level down.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent")
        .to_path_buf()
}

fn read(path: impl AsRef<Path>) -> Vec<u8> {
    let path = repo_root().join(path);
    fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The three masters, smallest detail first.
const S: &str = "assets/logo-s.svg";
const M: &str = "assets/logo-m.svg";
const L: &str = "assets/logo.svg";

/// The level a raster of this many pixels should come from.
///
/// The bands are the line's, not this project's: S up to 27 px, M to 63, L
/// above. Kept beside the assertions rather than imported, because the
/// generator that writes the files is JavaScript and this is the independent
/// reading of the same rule.
fn level_for(size: u32) -> &'static str {
    match size {
        0..=27 => S,
        28..=63 => M,
        _ => L,
    }
}

/// One image inside an `.ico`: the width its directory entry declares, and the
/// bytes of the image itself.
///
/// The entries are whole PNG files, so the image can be compared with a
/// reference render rather than only weighed.
fn ico_entries(ico: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    (0..count)
        .map(|i| {
            let at = 6 + i * 16;
            // A width byte of 0 means 256: the field is one byte and 256 does
            // not fit in it.
            let width = if ico[at] == 0 { 256_u32 } else { u32::from(ico[at]) };
            let length = u32::from_le_bytes([ico[at + 8], ico[at + 9], ico[at + 10], ico[at + 11]]) as usize;
            let offset = u32::from_le_bytes([ico[at + 12], ico[at + 13], ico[at + 14], ico[at + 15]]) as usize;
            (width, ico[offset..offset + length].to_vec())
        })
        .collect()
}

/// A PNG's width, read from its header.
fn png_width(png: &[u8]) -> u32 {
    u32::from_be_bytes([png[16], png[17], png[18], png[19]])
}

#[test]
fn the_ico_lists_its_images_largest_first() {
    // Windows picks by nearest size and ignores the order, but `tauri-codegen`
    // takes `entries()[0]` literally as the window icon — a 16 px entry at the
    // front is a title bar stretched from sixteen pixels. midda's generator
    // listed them ascending until v0.1.0.
    let entries = ico_entries(&read("assets/icon.ico"));
    let widths: Vec<u32> = entries.iter().map(|(width, _)| *width).collect();

    assert_eq!(widths, vec![256, 128, 64, 48, 32, 24, 16]);
}

#[test]
fn every_icon_in_the_ico_is_cut_for_its_own_size() {
    // An `.ico` holds sizes from 16 to 256, and there is room in it for all
    // three levels. Taking every entry from S is what put a filled blob on
    // kilna's desktop at 48 px, and is what midda's generator did until v0.1.0.
    //
    // Compared against a reference render per size, written by the same
    // generator from the level the rule names. Weighing the entries instead
    // cannot work and looks like it can: a PNG cut from the plain filled tile
    // still grows with its size, so "the 32 px image is heavier than the 24 px
    // one" holds whichever master both came from. Measured on this repository —
    // with every entry wrongly cut from S the weights were 393, 487, 625, 784,
    // 1040, 2183, 4594, rising just as neatly as the correct ones.
    let entries = ico_entries(&read("assets/icon.ico"));
    assert!(!entries.is_empty(), "the .ico holds no images at all");

    for (width, image) in entries {
        let reference = read(format!("assets/icon-levels/{width}.png"));
        assert_eq!(
            image,
            reference,
            "the {width} px entry is not the {} master rendered at {width} px",
            level_for(width)
        );
    }
}

#[test]
fn every_reference_render_names_the_master_the_rule_names() {
    // The comparison above is only worth anything if the references themselves
    // came from the right masters. A generator that took one master for the
    // `.ico` *and* for the references would leave every byte matching and every
    // icon wrong.
    //
    // So the generator writes down which master each size was cut from, and
    // this reads that back against its own reading of the rule. Named rather
    // than weighed, because weight cannot tell: cut wrongly from S the seven
    // entries still grow 393 → 4594 bytes, and the step across the 48→64 px
    // boundary is 132% where a correct set gives 139% — the two ranges overlap,
    // so no threshold separates them.
    let manifest = String::from_utf8(read("assets/icon-levels/manifest.json")).expect("the manifest is UTF-8");

    let mut checked = 0;
    for size in [16_u32, 24, 32, 48, 64, 128, 256] {
        let expected = level_for(size).rsplit('/').next().expect("a master path has a file name");
        let entry = manifest
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                (key.trim().trim_matches(['"', ' ']) == size.to_string()).then(|| value.trim().trim_matches(['"', ',', ' ']).to_owned())
            })
            .unwrap_or_else(|| panic!("the manifest says nothing about {size} px"));

        assert_eq!(entry, expected, "the {size} px reference was cut from {entry}, and the rule says {expected}");
        checked += 1;
    }

    assert_eq!(checked, 7, "the rule was not checked at every size in the .ico");
}

#[test]
fn the_desktop_icon_is_the_one_the_brand_assets_hold() {
    // Tauri reads `src-tauri/icons`, the rest of the line reads `assets`. Two
    // copies of a file drift, and the one that drifts is always the one nobody
    // opens — so they are asserted identical rather than trusted to a build
    // step someone remembers to run.
    assert_eq!(read("src-tauri/icons/icon.ico"), read("assets/icon.ico"));
}

#[test]
fn every_desktop_png_is_cut_for_its_own_size() {
    for (file, expected) in [
        ("src-tauri/icons/32x32.png", 32_u32),
        ("src-tauri/icons/128x128.png", 128),
        ("src-tauri/icons/icon.png", 256),
    ] {
        let png = read(file);
        assert_eq!(png_width(&png), expected, "{file} is not {expected} px wide");
    }

    // 32 px is the M band and 128 is L. If both came from one master the
    // smaller would scale to the larger's weight per pixel; they do not.
    let small = read("src-tauri/icons/32x32.png");
    let large = read("src-tauri/icons/128x128.png");
    assert_eq!(level_for(png_width(&small)), M);
    assert_eq!(level_for(png_width(&large)), L);
}

#[test]
fn the_favicon_is_the_small_tile() {
    // An SVG favicon is drawn at 16 px in a tab, whatever its viewBox says.
    // This is the one documented exception to `level_for`.
    assert_eq!(read("docs/public/favicon.svg"), read(S));
}

#[test]
fn the_docs_header_is_the_full_mark() {
    // The docs site draws it at bar height, well into the L band. This is the
    // exact placement that shipped wrong in kasl.
    assert_eq!(read("docs/src/assets/logo.svg"), read(L));
}

#[test]
fn the_touch_icon_is_the_full_mark_at_touch_size() {
    // 180 px is a home-screen tile, not an icon in a list. It was cut from S
    // until v0.1.0, which is a coloured square on a phone's home screen.
    let png = read("assets/apple-touch-icon.png");
    assert_eq!(png_width(&png), 180, "the touch icon is not 180 px");
    assert_eq!(level_for(180), L, "180 px should be the L band");

    // And the docs site serves the same file, so they cannot drift.
    assert_eq!(read("docs/public/apple-touch-icon.png"), png);
}

#[test]
fn the_three_masters_are_three_different_files() {
    // The whole rule rests on them differing. A copy-paste that made two of
    // them the same would leave every assertion above passing and every icon
    // wrong.
    assert_ne!(read(S), read(M));
    assert_ne!(read(M), read(L));
}
