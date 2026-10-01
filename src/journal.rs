//! The journal: every change a session makes on disk is written here *before* it is made, and
//! removed only once it has been undone. If the app or the PC stops mid-session, the next start
//! reads the journal and puts everything back.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::write_atomic;

/// What a session changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Changes {
    /// A folder of the app's own under `runs`, removed whole.
    Sandbox { dir: PathBuf },
    /// A copy placed in a Steam game folder. It is deleted only if its contents are still the
    /// app's own (by SHA-256); a file Steam moved aside is put back; folders the session made are
    /// removed only when empty.
    Steam {
        exe: PathBuf,
        sha256: String,
        backup: Option<PathBuf>,
        created_dirs: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub game: String,
    pub changes: Changes,
}

/// The result of trying to undo a session's changes.
#[derive(Debug, PartialEq, Eq)]
pub enum Undo {
    Done,
    /// The copy is still running (Windows won't delete a running exe). Try again later.
    Busy,
    /// Something isn't as the session left it, so it was left alone. The text says what.
    Kept(String),
}

#[derive(Debug, Default)]
pub struct Journal {
    path: PathBuf,
    entries: Vec<Entry>,
}

impl Journal {
    /// Reads the journal. A damaged one is set aside, not deleted, and reported.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        let mut journal = Self {
            path: path.to_path_buf(),
            entries: Vec::new(),
        };
        let Ok(bytes) = fs::read(path) else {
            return (journal, None);
        };
        if let Ok(entries) = serde_json::from_slice(&bytes) {
            journal.entries = entries;
            (journal, None)
        } else {
            {
                let aside = path.with_extension("damaged.json");
                let note = match fs::rename(path, &aside) {
                    Ok(()) => format!(
                        "The session journal was damaged and was set aside as {}. Check your Steam game folders by hand.",
                        aside.display()
                    ),
                    Err(e) => {
                        format!("The session journal is damaged and couldn't be set aside: {e}.")
                    }
                };
                (journal, Some(note))
            }
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn add(&mut self, entry: Entry) -> io::Result<()> {
        self.entries.push(entry);
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> io::Result<()> {
        self.entries.retain(|e| e.id != id);
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.entries).map_err(io::Error::other)?;
        write_atomic(&self.path, &bytes)
    }
}

/// Undoes one session's changes as far as it safely can.
pub fn undo(changes: &Changes, runs: &Path) -> Undo {
    match changes {
        Changes::Sandbox { dir } => undo_sandbox(dir, runs),
        Changes::Steam {
            exe,
            sha256,
            backup,
            created_dirs,
        } => undo_steam(exe, sha256, backup.as_deref(), created_dirs),
    }
}

fn undo_sandbox(dir: &Path, runs: &Path) -> Undo {
    // Only ever a direct child of `runs` with a session name: the journal can't aim this elsewhere.
    let ours = dir.parent() == Some(runs)
        && dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_session_id);
    if !ours {
        return Undo::Kept(format!("{} isn't one of the app's folders", dir.display()));
    }
    match fs::remove_dir_all(dir) {
        Ok(()) => Undo::Done,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Undo::Done,
        Err(_) => Undo::Busy,
    }
}

fn undo_steam(exe: &Path, sha256: &str, backup: Option<&Path>, created_dirs: &[PathBuf]) -> Undo {
    if exe.exists() {
        match file_sha256(exe) {
            Ok(hash) if hash == sha256 => {
                if fs::remove_file(exe).is_err() {
                    return Undo::Busy;
                }
            }
            Ok(_) => {
                return Undo::Kept(format!(
                    "{} has changed since the session started, so it was left alone",
                    exe.display()
                ));
            }
            Err(_) => return Undo::Busy,
        }
    }
    if let Some(backup) = backup.filter(|b| b.exists())
        && let Err(e) = fs::rename(backup, exe)
    {
        return Undo::Kept(format!(
            "Couldn't put {} back as {}: {e}",
            backup.display(),
            exe.display()
        ));
    }
    // Deepest first; a folder that isn't empty belongs to someone else now and stays.
    for dir in created_dirs.iter().rev() {
        let _ = fs::remove_dir(dir);
    }
    Undo::Done
}

/// Session IDs are `s` and digits, so nothing else under `runs` is ever removed.
pub fn is_session_id(name: &str) -> bool {
    name.len() > 1
        && name.len() <= 32
        && name.starts_with('s')
        && name.chars().skip(1).all(|c| c.is_ascii_digit())
}

pub fn file_sha256(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        // Writing to a String can't fail.
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_undo_only_touches_session_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let runs = tmp.path().join("runs");
        let session = runs.join("s123");
        fs::create_dir_all(session.join("win64")).unwrap();
        fs::write(session.join("win64/game.exe"), b"x").unwrap();
        let outside = tmp.path().join("keep");
        fs::create_dir_all(&outside).unwrap();

        let undo_dir = |dir: &Path| {
            undo(
                &Changes::Sandbox {
                    dir: dir.to_path_buf(),
                },
                &runs,
            )
        };
        assert!(matches!(undo_dir(&outside), Undo::Kept(_)));
        assert!(matches!(undo_dir(&runs.join("notasession")), Undo::Kept(_)));
        assert!(matches!(undo_dir(&runs.join("s1/../..")), Undo::Kept(_)));
        assert!(outside.exists());
        assert_eq!(undo_dir(&session), Undo::Done);
        assert!(!session.exists());
    }

    #[test]
    fn steam_undo_restores_the_original_and_keeps_foreign_files() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("common/Game");
        let made = game.join("bin");
        fs::create_dir_all(&made).unwrap();
        let exe = made.join("game.exe");
        let backup = made.join("game.exe.dqa-backup");
        fs::write(&backup, b"steam's partial file").unwrap();
        fs::write(&exe, b"our copy").unwrap();
        let ours = file_sha256(&exe).unwrap();
        let changes = |hash: &str| Changes::Steam {
            exe: exe.clone(),
            sha256: hash.into(),
            backup: Some(backup.clone()),
            created_dirs: vec![made.clone()],
        };

        // A file that isn't ours any more is left where it is.
        assert!(matches!(undo(&changes("0000"), tmp.path()), Undo::Kept(_)));
        assert!(exe.exists() && backup.exists());

        assert_eq!(undo(&changes(&ours), tmp.path()), Undo::Done);
        assert_eq!(fs::read(&exe).unwrap(), b"steam's partial file");
        assert!(!backup.exists());
        // The folder now holds Steam's file again, so it stays.
        assert!(made.exists());
    }

    #[test]
    fn steam_undo_removes_empty_folders_it_made() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Game");
        let made = [game.join("a"), game.join("a/b")];
        fs::create_dir_all(&made[1]).unwrap();
        let exe = made[1].join("x.exe");
        fs::write(&exe, b"ours").unwrap();
        let changes = Changes::Steam {
            exe: exe.clone(),
            sha256: file_sha256(&exe).unwrap(),
            backup: None,
            created_dirs: made.to_vec(),
        };
        assert_eq!(undo(&changes, tmp.path()), Undo::Done);
        assert!(!made[0].exists());
        assert!(game.exists());
    }

    #[test]
    fn journal_survives_a_reload_and_sets_aside_damage() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sessions.json");
        let (mut journal, note) = Journal::load(&path);
        assert!(note.is_none());
        journal
            .add(Entry {
                id: "s1".into(),
                game: "Game".into(),
                changes: Changes::Sandbox {
                    dir: tmp.path().join("runs/s1"),
                },
            })
            .unwrap();
        let (reloaded, _) = Journal::load(&path);
        assert_eq!(reloaded.entries().len(), 1);

        fs::write(&path, b"{ not json").unwrap();
        let (empty, note) = Journal::load(&path);
        assert_eq!(empty.entries().len(), 0);
        assert!(note.is_some());
        assert!(tmp.path().join("sessions.damaged.json").exists());
    }
}
