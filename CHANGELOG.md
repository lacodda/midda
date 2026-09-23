# Changelog

All notable changes to this project are documented in this file.

## [0.5.0] - 2026-09-23

### Bug Fixes
- Take write times from the entry itself, not its directory listing
- Count a WOF-compressed file by its compressed stream
- Answer accelerate off the window's thread
- Say a scan under a tenth of a second took that, not 0.0 s
- Round a WOF stream kept inside its record up to a cluster

### Documentation
- Record accelerate by relaunch and the streamed MFT in ADR 0006

### Features
- Read the MFT as a second scanner
- Restart elevated on accelerate, and scan where it left off
- Offer accelerate where it helps, and say how each scan was read

### Performance
- Read the MFT in 16 MiB reads straight from the device

### Testing
- Compare every file both scanners saw, not only the totals
- Tell a file written between two scans from a disagreement
- Name what the walk could not open and what the system holds open
## [0.4.1] - 2026-09-20

### Documentation
- Add the changelog for v0.4.1

### Testing
- Find the owner by its mark, not by its name
## [0.4.0] - 2026-09-20

### Documentation
- Record how shared bytes are charged, and to which name
- Add the changelog for v0.4.0

### Features
- Learn why a file's two sizes differ
- Count shared bytes once, and say where
- Carry the traits and the shared count to the window
- Account for every size the reader cannot explain
## [0.3.0] - 2026-09-19

### Documentation
- Record where the treemap is laid out, and why
- Add the changelog for v0.3.0

### Features
- Lay a folder out as a treemap, in fractions
- Serve the treemap layout to the window
- A treemap beside the table, pointing at the same entry

### Testing
- Hold every manifest to one version number
## [0.2.0] - 2026-09-18

### Bug Fixes
- Close the gap between the last month and the first year
- Declare midda-core once, in the workspace

### Documentation
- Add the changelog for v0.2.0

### Features
- Order and page a folder's children in the core
- Serve pages of rows, and the trail to a node
- A sortable virtualized table, and an overview of the volumes
## [0.1.0] - 2026-09-18

### Bug Fixes
- Recolor the mark from terracotta to violet
- Declare the MSRV that actually builds
- Cut every icon from the master its size calls for
- Split the non-Windows measurement by cfg instead of by return

### CI
- Gate the workspace and the frontend

### Documentation
- Make the readme a shopfront
- Drop the duplicate heading, add the badges
- Record the stack and the two-sizes decision
- Add the changelog for v0.1.0

### Features
- Scaffold the project
- Add the lacodda line mark and derived assets
- Scan a tree with both sizes behind a Scanner trait
- A window on Tauri v2 and dowel
