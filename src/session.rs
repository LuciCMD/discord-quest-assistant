//! Starting, watching and stopping a stand-in. A session copies this exe to a path Discord watches
//! for, runs the copy in play mode for the chosen time, and undoes everything once it exits.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use crate::journal::{self, Changes, Entry, Journal};
use crate::paths::Paths;
use crate::safe_path::ExePath;
use crate::steam::GameFolder;

/// The argument that starts a copy in play mode.
pub const PLAY_ARG: &str = "--dqa-play";
/// Longest session accepted, in minutes.
pub const MAX_MINUTES: u64 = 240;
/// How long past its time a copy may run before it is stopped.
const GRACE: Duration = Duration::from_secs(60);
/// Suffix for a file moved aside in a Steam folder while the stand-in sits in its place.
const BACKUP_SUFFIX: &str = ".dqa-backup";

/// Where the stand-in goes.
#[derive(Debug, Clone)]
pub enum Target {
    /// Under the app's own folder, at the path Discord's list names.
    Sandbox(ExePath),
    /// Inside a game folder Steam created, for games Discord finds through Steam.
    Steam { folder: GameFolder, exe: ExePath },
}

#[derive(Debug)]
pub enum StartError {
    Installed(PathBuf),
    MissingFolder(PathBuf),
    LeftoverBackup(PathBuf),
    NotAFile(PathBuf),
    Link(PathBuf),
    Io(String),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Installed(path) => write!(
                f,
                "This game is installed, so {} is the real game. Play it from Steam instead",
                path.display()
            ),
            Self::MissingFolder(path) => write!(
                f,
                "{} doesn't exist. Start the game's download in Steam and pause it at 1-2% first",
                path.display()
            ),
            Self::LeftoverBackup(path) => write!(
                f,
                "{} is left over from an earlier run. Check the folder and remove it first",
                path.display()
            ),
            Self::NotAFile(path) => write!(f, "{} is a folder, not a file", path.display()),
            Self::Link(path) => write!(
                f,
                "{} is a link to somewhere else, so nothing is written through it",
                path.display()
            ),
            Self::Io(text) => f.write_str(text),
        }
    }
}

/// A stand-in that is running.
pub struct Running {
    pub id: String,
    pub game: String,
    pub path: PathBuf,
    pub length: Duration,
    started: Instant,
    child: Child,
}

impl Running {
    pub fn remaining(&self) -> Duration {
        self.length.saturating_sub(self.started.elapsed())
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed().min(self.length)
    }

    /// True once the copy has exited. A copy that overstays its time is stopped.
    pub fn finished(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_)) | Err(_) => true,
            Ok(None) => {
                if self.started.elapsed() > self.length + GRACE {
                    self.stop();
                }
                false
            }
        }
    }

    pub fn stop(&mut self) {
        // Either it was stopped or it had already exited; both end the same way.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Everything a start needs to know about this exe.
pub struct Me {
    pub exe: PathBuf,
    pub sha256: String,
}

/// Places the stand-in and starts it. The journal entry is written first, so a crash at any point
/// after this is undone at the next start.
pub fn start(
    game: &str,
    minutes: u64,
    target: &Target,
    me: &Me,
    paths: &Paths,
    journal: &mut Journal,
) -> Result<Running, StartError> {
    let minutes = minutes.clamp(1, MAX_MINUTES);
    let id = format!("s{}", unique_number());
    debug_assert!(journal::is_session_id(&id));
    let (path, changes, backup) = plan(&id, target, me, paths)?;
    journal
        .add(Entry {
            id: id.clone(),
            game: game.to_owned(),
            changes: changes.clone(),
        })
        .map_err(|e| StartError::Io(format!("Couldn't write the session journal: {e}")))?;

    let steam_folder = match target {
        Target::Steam { folder, .. } => Some(folder.folder.as_path()),
        Target::Sandbox(_) => None,
    };
    let placed = place_and_spawn(
        &path,
        steam_folder,
        backup.as_deref(),
        &changes,
        game,
        minutes,
        &me.exe,
    );
    match placed {
        Ok(child) => Ok(Running {
            id,
            game: game.to_owned(),
            path,
            length: Duration::from_secs(minutes * 60),
            started: Instant::now(),
            child,
        }),
        Err(e) => {
            if journal::undo(&changes, &paths.runs) == journal::Undo::Done {
                // Nothing is left on disk, so the entry can go.
                let _ = journal.remove(&id);
            }
            Err(StartError::Io(format!("Couldn't start the stand-in: {e}")))
        }
    }
}

/// Works out the copy's path and what will change, without touching anything.
fn plan(
    id: &str,
    target: &Target,
    me: &Me,
    paths: &Paths,
) -> Result<(PathBuf, Changes, Option<PathBuf>), StartError> {
    match target {
        Target::Sandbox(exe) => {
            let dir = paths.runs.join(id);
            let path = exe.under(&dir);
            Ok((path, Changes::Sandbox { dir }, None))
        }
        Target::Steam { folder, exe } => {
            if !folder.folder.is_dir() {
                return Err(StartError::MissingFolder(folder.folder.clone()));
            }
            let path = exe.under(&folder.folder);
            if let Some(link) = first_link(&folder.folder, &path) {
                return Err(StartError::Link(link));
            }
            let backup = existing_file_backup(&path, folder.installed)?;
            let created_dirs = missing_dirs(&folder.folder, &path);
            let changes = Changes::Steam {
                exe: path.clone(),
                sha256: me.sha256.clone(),
                backup: backup.clone(),
                created_dirs,
            };
            Ok((path, changes, backup))
        }
    }
}

/// A file already at the path is Steam's partial download; it is moved aside and put back after.
/// A real install is never touched.
fn existing_file_backup(path: &Path, installed: bool) -> Result<Option<PathBuf>, StartError> {
    if path.is_dir() {
        return Err(StartError::NotAFile(path.to_path_buf()));
    }
    if !path.exists() {
        return Ok(None);
    }
    if installed {
        return Err(StartError::Installed(path.to_path_buf()));
    }
    let mut name = path.as_os_str().to_owned();
    name.push(BACKUP_SUFFIX);
    let backup = PathBuf::from(name);
    if backup.exists() {
        return Err(StartError::LeftoverBackup(backup));
    }
    Ok(Some(backup))
}

/// The first symbolic link or junction between `base` (which may itself be one: games are often
/// moved to another drive that way) and `path`, including `path`. Writing through one could land
/// anywhere, so a Steam start refuses it.
fn first_link(base: &Path, path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .take_while(|p| *p != base && p.starts_with(base))
        .find(|p| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()))
        .map(Path::to_path_buf)
}

/// The folders between `base` and the file that don't exist yet, outermost first.
fn missing_dirs(base: &Path, file: &Path) -> Vec<PathBuf> {
    let mut missing: Vec<PathBuf> = file
        .ancestors()
        .skip(1)
        .take_while(|dir| *dir != base && dir.starts_with(base))
        .filter(|dir| !dir.exists())
        .map(Path::to_path_buf)
        .collect();
    missing.reverse();
    missing
}

fn place_and_spawn(
    path: &Path,
    steam_folder: Option<&Path>,
    backup: Option<&Path>,
    changes: &Changes,
    game: &str,
    minutes: u64,
    me: &Path,
) -> io::Result<Child> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("the path has no folder"))?;
    if let Some(backup) = backup {
        fs::rename(path, backup)?;
    }
    fs::create_dir_all(parent)?;
    // Checked again now the folders exist, in case one became a link since the plan.
    if let Some(base) = steam_folder
        && first_link(base, path).is_some()
    {
        return Err(io::Error::other("a folder on the way is a link"));
    }
    fs::copy(me, path)?;
    if let Changes::Steam { sha256, .. } = changes {
        // The copy must hash the way the journal says, or it could never be cleaned up.
        if &journal::file_sha256(path)? != sha256 {
            return Err(io::Error::other("the copy doesn't match this exe"));
        }
    }
    // No shell: the path and arguments go to Windows as they are.
    Command::new(path)
        .arg(PLAY_ARG)
        .arg((minutes * 60).to_string())
        .arg(game)
        .current_dir(parent)
        .spawn()
}

/// Milliseconds since 1970, bumped so two sessions started together still differ.
fn unique_number() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    millis * 1000 + u128::from(COUNTER.fetch_add(1, Ordering::Relaxed) % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_dirs_stop_at_the_game_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("Game");
        fs::create_dir_all(base.join("bin")).unwrap();
        let file = base.join("bin/win64/sub/game.exe");
        assert_eq!(
            missing_dirs(&base, &file),
            [base.join("bin/win64"), base.join("bin/win64/sub")]
        );
        assert_eq!(
            missing_dirs(&base, &base.join("game.exe")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn links_inside_the_game_folder_are_found() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("Game");
        let elsewhere = tmp.path().join("Elsewhere");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        let file = base.join("bin").join("game.exe");
        assert_eq!(first_link(&base, &file), None);
        // A junction, which any user can make (a symlink needs Developer Mode or admin).
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(base.join("bin"))
            .arg(&elsewhere)
            .output()
            .unwrap();
        assert!(made.status.success());
        assert_eq!(first_link(&base, &file), Some(base.join("bin")));
    }

    #[test]
    fn a_real_install_is_never_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("game.exe");
        fs::write(&exe, b"real").unwrap();
        assert!(matches!(
            existing_file_backup(&exe, true),
            Err(StartError::Installed(_))
        ));
        let backup = existing_file_backup(&exe, false).unwrap().unwrap();
        assert!(backup.to_string_lossy().ends_with("game.exe.dqa-backup"));
        fs::write(&backup, b"old").unwrap();
        assert!(matches!(
            existing_file_backup(&exe, false),
            Err(StartError::LeftoverBackup(_))
        ));
    }
}

/// Runs real stand-ins end to end. They open windows, so: `cargo build`, then
/// `cargo test -- --ignored end_to_end`.
#[cfg(test)]
mod end_to_end {
    use super::*;
    use crate::journal::{Undo, undo};

    fn setup(tmp: &Path) -> (Paths, Me, Journal) {
        // The app itself, built beside this test binary's `deps` folder by `cargo build`.
        let exe = std::env::current_exe()
            .unwrap()
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("discord-quest-assistant.exe");
        assert!(exe.is_file(), "run cargo build first");
        let me = Me {
            sha256: journal::file_sha256(&exe).unwrap(),
            exe,
        };
        let paths = Paths {
            root: tmp.to_path_buf(),
            runs: tmp.join("runs"),
        };
        fs::create_dir_all(&paths.runs).unwrap();
        let (journal, _) = Journal::load(&tmp.join("sessions.json"));
        (paths, me, journal)
    }

    fn finish(mut run: Running, journal: &mut Journal, paths: &Paths) {
        run.stop();
        let entry = journal
            .entries()
            .iter()
            .find(|e| e.id == run.id)
            .cloned()
            .unwrap();
        let mut result = Undo::Busy;
        for _ in 0..50 {
            result = undo(&entry.changes, &paths.runs);
            if result != Undo::Busy {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(result, Undo::Done);
        journal.remove(&run.id).unwrap();
    }

    #[test]
    #[ignore = "starts real processes"]
    fn end_to_end_sandbox() {
        let tmp = tempfile::tempdir().unwrap();
        let (paths, me, mut journal) = setup(tmp.path());
        let target = Target::Sandbox(ExePath::parse("win64/testgame.exe").unwrap());
        let mut run = start("Test Game", 1, &target, &me, &paths, &mut journal).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        assert!(!run.finished(), "the copy should still be running");
        assert!(run.path.ends_with(r"win64\testgame.exe") && run.path.is_file());
        assert_eq!(journal.entries().len(), 1);
        finish(run, &mut journal, &paths);
        assert!(fs::read_dir(&paths.runs).unwrap().next().is_none());
    }

    #[test]
    #[ignore = "starts real processes"]
    fn end_to_end_steam_partial_download() {
        let tmp = tempfile::tempdir().unwrap();
        let (paths, me, mut journal) = setup(tmp.path());
        let game = tmp.path().join("common").join("Some Game");
        fs::create_dir_all(game.join("bin")).unwrap();
        let partial = game.join("bin").join("game.exe");
        fs::write(&partial, b"steam's partial file").unwrap();
        let folder = GameFolder {
            app_id: 1,
            folder: game.clone(),
            installed: false,
        };
        let exe = ExePath::parse("bin/game.exe").unwrap();
        let target = Target::Steam {
            folder: folder.clone(),
            exe,
        };
        let run = start("Some Game", 1, &target, &me, &paths, &mut journal).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(journal::file_sha256(&partial).unwrap(), me.sha256);
        finish(run, &mut journal, &paths);
        assert_eq!(fs::read(&partial).unwrap(), b"steam's partial file");
        assert_eq!(fs::read_dir(game.join("bin")).unwrap().count(), 1);

        // A new folder inside the game is made for the copy and removed after.
        let deeper = ExePath::parse("new/dir/game.exe").unwrap();
        let target = Target::Steam {
            folder,
            exe: deeper,
        };
        let run = start("Some Game", 1, &target, &me, &paths, &mut journal).unwrap();
        finish(run, &mut journal, &paths);
        assert!(!game.join("new").exists());
    }
}
