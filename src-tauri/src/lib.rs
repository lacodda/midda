//! midda-desktop: the Tauri door onto midda-core.
//!
//! The window owns no scanning logic. It asks for a scan, polls how far it has
//! got, and reads rows out of the finished tree — everything about what a byte
//! means lives in the core, so the CLI and the MCP door of v0.19 get the same
//! answers without a second implementation to keep in step.

mod elevate;
mod scan;
mod volumes;

/// Build and run the desktop application.
pub fn run() {
    let launch = elevate::parse(std::env::args());
    // A window started by "accelerate" opens only once the one that started it
    // has closed: two WebViews at different elevation cannot share one data
    // directory.
    if let Some(pid) = launch.after {
        elevate::wait_for(pid);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(scan::Session::default())
        .manage(elevate::Pending::new(launch.scan))
        .invoke_handler(tauri::generate_handler![
            volumes::list_volumes,
            scan::start_scan,
            scan::scan_progress,
            scan::cancel_scan,
            scan::list_children,
            scan::trail_to,
            scan::treemap,
            elevate::acceleration,
            elevate::accelerate,
            elevate::launch_request,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the midda application");
}
