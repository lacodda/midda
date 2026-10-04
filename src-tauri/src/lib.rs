//! midda-desktop: the Tauri door onto midda-core.
//!
//! The window owns no scanning logic. It opens a folder, polls how the read
//! and the upkeep are going, and reads rows out of the tree — everything
//! about what a byte means lives in the core, so the CLI and the MCP door of
//! v0.19 get the same answers without a second implementation to keep in
//! step.

mod elevate;
mod session;
mod volumes;

use tauri::Manager as _;

/// Build and run the desktop application.
pub fn run() {
    let launch = elevate::parse(std::env::args());
    // A window started by "accelerate" opens only once the one that started it
    // has closed: two WebViews at different elevation cannot share one data
    // directory — and the closing one saves the index the new one opens.
    if let Some(pid) = launch.after {
        elevate::wait_for(pid);
    }

    let app = tauri::Builder::default()
        .setup(|app| {
            // Indexes are kept beside the WebView's own data, in the user's
            // local application data: they are a cache of the disk, not a
            // document, and do not roam.
            let store = app
                .path()
                .app_local_data_dir()
                .map(|dir| dir.join("index"))
                .unwrap_or_else(|_| std::env::temp_dir().join("midda-index"));
            app.manage(session::Session::new(store));
            show_if_the_page_never_does(app.handle());
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .manage(elevate::Pending::new(launch.scan))
        .invoke_handler(tauri::generate_handler![
            volumes::list_volumes,
            session::start_scan,
            session::scan_progress,
            session::cancel_scan,
            session::rescan,
            session::list_children,
            session::trail_to,
            session::trail_to_path,
            session::treemap,
            session::changes_since,
            elevate::acceleration,
            elevate::accelerate,
            elevate::launch_request,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the midda application");

    app.run(|app, event| {
        // The index is saved on the way out, however the way out was taken:
        // the close button, "accelerate", the end of the session.
        if let tauri::RunEvent::Exit = event
            && let Some(session) = app.try_state::<session::Session>()
        {
            session.close();
        }
    });
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
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_DEADLINE);
        // Nothing to report to: if the window cannot be shown, it is gone.
        let _ = window.show();
    });
}
