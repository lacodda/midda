//! Holds every place that writes midda's version number to the same one.
//!
//! The number lives in five places, because four different tools insist on
//! owning it: the Cargo workspace, the workspace's own dependency entry for
//! `midda-core` (a path dependency still needs a version to be publishable),
//! `package.json`, and `tauri.conf.json`, which is what the installer and the
//! window title show.
//!
//! Each one goes stale on its own schedule and none of them complains. Both
//! failures have already happened here: the `midda-core` entry was written
//! inside the consuming crate and broke the build at v0.2.0, and `package.json`
//! was left at v0.2.0 through that whole release — it happened to be right, and
//! would have shipped v0.3.0 calling itself v0.2.0.
//!
//! A release is one of the few moments where a mistake is expensive and the
//! diff looks fine, so it is a test rather than a step in a checklist.

use std::fs;
use std::path::PathBuf;

/// The repository root: this crate is `src-tauri`, one level down.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

/// The first value of `"version"` in a JSON file.
///
/// A three-line scan rather than a JSON dependency: both files put the field at
/// the top level, and a parser here would mean `serde_json` in the dev tree to
/// read one string.
fn json_version(source: &str, file: &str) -> String {
    source
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("\"version\":")?;
            let value = rest.trim().trim_start_matches('"');
            let end = value.find('"')?;
            Some(value[..end].to_owned())
        })
        .unwrap_or_else(|| panic!("no top-level \"version\" in {file}"))
}

/// The value of `version = "..."` on the line following `[workspace.package]`.
fn workspace_version(source: &str) -> String {
    let after = source
        .split_once("[workspace.package]")
        .expect("Cargo.toml has a [workspace.package] section")
        .1;
    after
        .lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix("version")?.trim_start().strip_prefix('=')?;
            let value = rest.trim().trim_start_matches('"');
            let end = value.find('"')?;
            Some(value[..end].to_owned())
        })
        .expect("[workspace.package] declares a version")
}

/// The version the workspace pins its own `midda-core` at.
fn core_dependency_version(source: &str) -> String {
    let line = source
        .lines()
        .find(|line| line.trim_start().starts_with("midda-core = {"))
        .expect("[workspace.dependencies] declares midda-core");
    let rest = line.split_once("version =").expect("the midda-core entry carries a version").1;
    let value = rest.trim().trim_start_matches('"');
    let end = value.find('"').expect("a closed version string");
    value[..end].to_owned()
}

#[test]
fn every_manifest_names_the_same_version() {
    let cargo = read("Cargo.toml");
    let workspace = workspace_version(&cargo);

    // The crate version this binary was built at — so the test fails when the
    // manifests agree with each other but not with what is being compiled.
    assert_eq!(workspace, env!("CARGO_PKG_VERSION"), "Cargo.toml and the built crate disagree");

    assert_eq!(
        core_dependency_version(&cargo),
        workspace,
        "[workspace.dependencies] pins midda-core at a version the workspace is not at — \
         the build breaks the moment the old one is gone"
    );

    let package = read("package.json");
    assert_eq!(json_version(&package, "package.json"), workspace, "package.json is behind the workspace");

    let tauri = read("src-tauri/tauri.conf.json");
    assert_eq!(
        json_version(&tauri, "tauri.conf.json"),
        workspace,
        "tauri.conf.json is behind the workspace — this is the number the installer \
         and the window show"
    );
}

#[test]
fn the_readme_speaks_about_the_version_being_shipped() {
    // The shopfront's Status block names a version, and a README describing the
    // release before this one is the first thing anyone reads.
    let readme = read("README.md");
    let version = workspace_version(&read("Cargo.toml"));
    assert!(readme.contains(&format!("v{version}")), "README.md does not mention v{version}");
}
