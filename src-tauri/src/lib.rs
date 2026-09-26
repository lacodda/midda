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
        .setup(|app| {
            show_if_the_page_never_does(app.handle());
            Ok(())
        })
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

/// How long the page has to show the window before this side does.
///
/// The page shows it the moment the splash has painted, which on any machine
/// is well under a second; three is a margin, not an estimate.
const SHOW_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);

/// Shows the window after [`SHOW_DEADLINE`] if the page has not.
///
/// The window is created hidden (`tauri.conf.json`) so that nothing white is
/// ever seen before the splash, and the page shows it. A page that failed
/// before its first line - a bundle that did not load, a module that threw on
/// import - would then leave a running process with no window at all, and a
/// second launch would look like the first one failing too. A window that
/// appears late and says what went wrong in its console is the better failure.
/// `show` on a window already shown does nothing.
fn show_if_the_page_never_does(app: &tauri::AppHandle) {
    use tauri::Manager as _;

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_DEADLINE);
        // Nothing to report to: if the window cannot be shown, it is gone.
        let _ = window.show();
    });
}
