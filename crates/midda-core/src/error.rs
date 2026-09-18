//! What can go wrong at the level where a scan has to stop.
//!
//! A single unreadable directory is not an error — it is recorded in
//! [`crate::tree::Tree::skipped`] and the scan carries on, because a system
//! volume always has some and a scan that aborted on the first one would never
//! finish. These are the conditions where there is no tree to return at all.

use std::path::PathBuf;

/// What went wrong.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The scan root itself could not be read. There is nothing to show.
    #[error("cannot read {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The scan root is not a directory.
    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),

    /// The scan was asked to stop before it finished.
    ///
    /// A partial tree is not returned as if it were whole: a total that is
    /// missing half the volume looks exactly like a total, and acting on it is
    /// how someone deletes the wrong thing.
    #[error("the scan was cancelled")]
    Cancelled,

    /// This scanner cannot run here — the MFT reader without elevation, for
    /// instance.
    #[error("{0}")]
    Unavailable(String),
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;
