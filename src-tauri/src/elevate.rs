//! "Accelerate": the window asks to be started again as an administrator, so
//! scans can read the MFT.
//!
//! The whole window restarts rather than handing the read to an elevated
//! helper (ADR 0006). The new process is told two things on its command line:
//! which folder to scan as soon as it opens, so the reader does not have to
//! pick it twice, and which process to wait for, so the elevated window does
//! not open its WebView while the old one still holds the same data directory.

use std::path::PathBuf;
use std::sync::Mutex;

use midda_core::Acceleration;
use serde::{Deserialize, Serialize};

/// What this process was started to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// A folder to scan as soon as the window opens.
    pub scan: Option<PathBuf>,
    /// A process to let finish before opening a window.
    pub after: Option<u32>,
}

/// Reads the launch arguments. Anything it does not know is ignored: a stray
/// argument from a shortcut should not stop the window from opening.
pub fn parse(args: impl IntoIterator<Item = String>) -> Launch {
    let mut launch = Launch::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scan" => launch.scan = args.next().map(PathBuf::from),
            "--after" => launch.after = args.next().and_then(|pid| pid.parse().ok()),
            _ => {}
        }
    }
    launch
}

/// One argument quoted the way `CommandLineToArgvW` — and so Rust's own
/// `std::env::args` — will split it back out.
///
/// Backslashes are literal except before a quote, where they escape in pairs.
/// That is what makes `"C:\"` a trap: the backslash escapes the closing quote
/// and the argument runs on into the next one. A volume root is exactly the
/// path someone accelerates on, so this is not a corner case.
pub fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in arg.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// The folder this launch was asked to scan, handed out once.
#[derive(Default)]
pub struct Pending(Mutex<Option<PathBuf>>);

impl Pending {
    pub fn new(scan: Option<PathBuf>) -> Self {
        Self(Mutex::new(scan))
    }
}

/// Where scans stand with the fast scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccelerationState {
    pub state: Acceleration,
}

/// Whether scans of `path` can read the MFT, could after elevation, or never
/// can. With no path, the system drive answers: it is where "accelerate" is
/// offered before anything has been picked.
///
/// Off the window's thread for the reason `list_volumes` is: the answer asks
/// the volume under `path` for its file system, and a network share can take
/// its time.
#[tauri::command(async)]
pub fn acceleration(path: Option<String>) -> AccelerationState {
    let root = path.map_or_else(system_drive, PathBuf::from);
    AccelerationState {
        state: midda_core::acceleration(&root),
    }
}

fn system_drive() -> PathBuf {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    PathBuf::from(format!("{drive}\\"))
}

/// The folder this window was started to scan, if any. Answers once: a window
/// reloaded later should not rescan by surprise.
///
/// # Errors
///
/// When the state lock is poisoned.
#[tauri::command]
pub fn launch_request(pending: tauri::State<'_, Pending>) -> Result<Option<String>, String> {
    let mut scan = pending.0.lock().map_err(|_| "the launch state is poisoned".to_owned())?;
    Ok(scan.take().map(|path| path.to_string_lossy().into_owned()))
}

/// Starts this program again as an administrator, scanning `path`, and closes
/// this window.
///
/// Async so it runs off the main thread: the call returns only once the
/// administrator's prompt has been answered, which can be minutes, and a
/// synchronous Tauri command runs on the thread that owns the window. The
/// window stayed responsive in the live run of 2026-09-23 only because
/// `ShellExecuteExW` pumps messages while it waits — not a property to lean on.
///
/// # Errors
///
/// When the prompt was declined or the process could not be started. The
/// window stays open and keeps the walk.
#[tauri::command]
pub async fn accelerate(path: Option<String>, app: tauri::AppHandle) -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
    let mut args = vec!["--after".to_owned(), std::process::id().to_string()];
    if let Some(path) = path {
        args.push("--scan".to_owned());
        args.push(quote(&path));
    }
    imp::relaunch_elevated(&program, &args.join(" "))?;
    app.exit(0);
    Ok(())
}

/// Waits for the process that asked for this one to close, for a few seconds
/// at most. Called before the window is built.
pub fn wait_for(pid: u32) {
    imp::wait_for(pid);
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    pub(super) fn relaunch_elevated(program: &Path, args: &str) -> Result<(), String> {
        let verb = wide("runas".as_ref());
        let file = wide(program.as_os_str());
        let parameters = wide(args.as_ref());
        // SAFETY: an all-zero SHELLEXECUTEINFOW is its documented empty state;
        // the fields that matter are set below.
        #[allow(unsafe_code, reason = "SHELLEXECUTEINFOW has no safe constructor")]
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = u32::try_from(size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0);
        info.fMask = SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.nShow = SW_SHOWNORMAL;

        // SAFETY: `info` is initialised and its string pointers outlive the call.
        #[allow(unsafe_code, reason = "starting a process elevated is not exposed by std")]
        let ok = unsafe { ShellExecuteExW(&raw mut info) };
        if ok != 0 {
            return Ok(());
        }
        // SAFETY: reads the calling thread's last error, nothing else.
        #[allow(unsafe_code, reason = "the reason the prompt failed is only in the last error")]
        let error = unsafe { GetLastError() };
        if error == ERROR_CANCELLED {
            Err("The administrator prompt was declined; scans keep walking the folders.".to_owned())
        } else {
            Err(format!("Could not start midda as an administrator (error {error})."))
        }
    }

    pub(super) fn wait_for(pid: u32) {
        // SAFETY: asks for a handle that can only be waited on; a pid that is
        // gone or was never ours yields a null handle, which is checked.
        #[allow(unsafe_code, reason = "waiting on another process is not exposed by std")]
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return;
        }
        // SAFETY: `handle` is open; it is waited on once and closed once.
        #[allow(unsafe_code, reason = "waiting on another process is not exposed by std")]
        unsafe {
            WaitForSingleObject(handle, 10_000);
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    // There is no MFT to accelerate onto; the window never offers it.
    pub(super) fn relaunch_elevated(_program: &Path, _args: &str) -> Result<(), String> {
        Err("acceleration exists only on Windows".to_owned())
    }

    pub(super) const fn wait_for(_pid: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &[&str]) -> Vec<String> {
        line.iter().map(|&arg| arg.to_owned()).collect()
    }

    #[test]
    fn a_relaunch_says_what_to_scan_and_whom_to_wait_for() {
        let launch = parse(args(&["midda-desktop.exe", "--after", "4242", "--scan", "C:\\"]));
        assert_eq!(
            launch,
            Launch {
                scan: Some(PathBuf::from("C:\\")),
                after: Some(4242),
            }
        );
    }

    #[test]
    fn an_ordinary_launch_asks_for_nothing() {
        assert_eq!(parse(args(&["midda-desktop.exe"])), Launch::default());
        assert_eq!(parse(args(&["midda-desktop.exe", "--after", "not-a-pid", "--unknown"])), Launch::default());
    }

    /// Splits a command line the way `CommandLineToArgvW` does, for the
    /// arguments after the program name.
    fn split(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut started = false;
        let chars: Vec<char> = line.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            let character = chars[index];
            if character == '\\' {
                let mut count = 0;
                while index < chars.len() && chars[index] == '\\' {
                    count += 1;
                    index += 1;
                }
                if index < chars.len() && chars[index] == '"' {
                    current.extend(std::iter::repeat_n('\\', count / 2));
                    if count % 2 == 1 {
                        current.push('"');
                        index += 1;
                    }
                } else {
                    current.extend(std::iter::repeat_n('\\', count));
                }
                started = true;
                continue;
            }
            match character {
                '"' => {
                    quoted = !quoted;
                    started = true;
                }
                ' ' | '\t' if !quoted => {
                    if started {
                        out.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                other => {
                    current.push(other);
                    started = true;
                }
            }
            index += 1;
        }
        if started {
            out.push(current);
        }
        out
    }

    #[test]
    fn a_quoted_path_comes_back_as_itself() {
        for path in [
            "C:\\",
            "C:\\Program Files\\",
            "D:\\a folder\\with \"quotes\"",
            "C:\\plain",
            "",
            "E:\\ends in two\\\\",
        ] {
            let line = format!("--scan {} --after 7", quote(path));
            assert_eq!(split(&line), ["--scan", path, "--after", "7"], "the line was {line}");
        }
    }

    #[test]
    fn a_volume_root_does_not_swallow_the_next_argument() {
        // The trap itself: naively quoted, `"C:\"` escapes its own closing
        // quote.
        let naive = format!("--scan \"{}\" --after 7", "C:\\");
        assert_ne!(split(&naive).len(), 4, "the naive quoting really is broken");
        assert_eq!(split(&format!("--scan {} --after 7", quote("C:\\"))).len(), 4);
    }
}
