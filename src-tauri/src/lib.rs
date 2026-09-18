//! midda-desktop: the Tauri door onto midda-core.
//!
//! The window owns no scanning logic. It asks for a scan, polls how far it has
//! got, and reads rows out of the finished tree — everything about what a byte
//! means lives in the core, so the CLI and the MCP door of v0.19 get the same
//! answers without a second implementation to keep in step.

mod scan;
mod volumes;

/// Build and run the desktop application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(scan::Session::default())
        .invoke_handler(tauri::generate_handler![
            volumes::list_volumes,
            scan::start_scan,
            scan::scan_progress,
            scan::cancel_scan,
            scan::list_children,
            scan::trail_to,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the midda application");
}
