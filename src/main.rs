//! midda — a disk space analyzer for Windows.
//!
//! Two scanners sit behind one trait: a filesystem walk that works without
//! elevation, and an MFT reader that needs administrator rights but finishes in
//! seconds. See `docs/adr/0001-hybrid-scanning.md`.

fn main() {
    println!("midda {}", env!("CARGO_PKG_VERSION"));
    println!("Scaffold only — the scanner lands in v0.1.0.");
}
