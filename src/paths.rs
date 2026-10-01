//! Where the app keeps its files: `%LOCALAPPDATA%\Discord Quest Assistant\data`.
//!
//! - `games.json`: the cached copy of Discord's game list.
//! - `sessions.json`: the journal of every change made on disk, so a crash can be undone.
//! - `runs\<session>\…`: the stand-in copies, one folder per session.
//! - `instance.lock`: held while the main window is open, so only one copy manages the journal.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub runs: PathBuf,
}

impl Paths {
    pub fn resolve() -> io::Result<Self> {
        let dirs = directories::ProjectDirs::from("", "", "Discord Quest Assistant")
            .ok_or_else(|| io::Error::other("Windows didn't say where app data goes"))?;
        let root = dirs.data_local_dir().to_path_buf();
        let runs = root.join("runs");
        fs::create_dir_all(&runs)?;
        Ok(Self { root, runs })
    }

    pub fn catalog(&self) -> PathBuf {
        self.root.join("games.json")
    }

    pub fn journal(&self) -> PathBuf {
        self.root.join("sessions.json")
    }

    pub fn lock(&self) -> PathBuf {
        self.root.join("instance.lock")
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Writes beside the target, then renames over it, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = fs::File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path)
}
