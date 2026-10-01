//! Finding a game in the local Steam install: where Steam is, which library holds the game, its
//! install folder and its launch options. Everything is read from Steam's own files on this PC.

mod appinfo;
mod kv;
mod vdf_text;

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf, Prefix};

use crate::safe_path::{self, ExePath};

/// Steam's text files are small; anything bigger than this isn't one.
const MAX_TEXT_FILE: u64 = 4 * 1024 * 1024;
/// The app cache is a few megabytes; this is far above any real one.
const MAX_APP_CACHE: u64 = 256 * 1024 * 1024;
/// Steam's `StateFlags` bit for a game whose files are all there.
const FULLY_INSTALLED: u32 = 4;

#[derive(Debug)]
pub enum SteamError {
    NotInstalled,
    Unreadable(String),
    NotStarted,
    NoLaunchOptions,
}

impl fmt::Display for SteamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => write!(f, "Steam doesn't seem to be installed"),
            Self::Unreadable(why) => f.write_str(why),
            Self::NotStarted => write!(
                f,
                "Steam doesn't know this game yet. Start its download in Steam, pause it at 1-2%, then come back"
            ),
            Self::NoLaunchOptions => write!(
                f,
                "Steam lists no Windows launch option for this game. Enter the executable path yourself"
            ),
        }
    }
}

/// The game's folder in a Steam library, as Steam's manifest describes it.
#[derive(Debug, Clone)]
pub struct GameFolder {
    pub app_id: u32,
    pub folder: PathBuf,
    /// All files are present: this is a real install, which is never touched.
    pub installed: bool,
}

/// One way Steam can start the game.
#[derive(Debug, Clone)]
pub struct LaunchOption {
    pub exe: ExePath,
    pub label: String,
}

/// Where Steam is, from the registry, and every library folder it uses.
pub fn libraries() -> Result<Vec<PathBuf>, SteamError> {
    let root = windows_registry::CURRENT_USER
        .open(r"Software\Valve\Steam")
        .and_then(|key| key.get_string("SteamPath"))
        .map_err(|_| SteamError::NotInstalled)?;
    let root = PathBuf::from(root.replace('/', "\\"));
    if !on_local_drive(&root) || !root.is_dir() {
        return Err(SteamError::NotInstalled);
    }
    let mut found = vec![root.clone()];
    let listing = root.join("steamapps").join("libraryfolders.vdf");
    // Without the listing, the main library is still worth searching.
    if let Ok(flat) = read_text(&listing) {
        for (key, value) in flat.under("libraryfolders") {
            let path = PathBuf::from(value);
            let is_path = key.ends_with("/path") && key.matches('/').count() == 1;
            let known = found
                .iter()
                .any(|p| p.as_os_str().eq_ignore_ascii_case(path.as_os_str()));
            if is_path && on_local_drive(&path) && !known {
                found.push(path);
            }
        }
    }
    Ok(found)
}

/// A plain `C:\…` path. Network shares (`\\server\…`) are refused: opening one sends this PC's
/// sign-in to that server, and `libraryfolders.vdf` isn't trusted enough to choose one.
fn on_local_drive(path: &Path) -> bool {
    let mut parts = path.components();
    let disk = matches!(
        parts.next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    );
    disk && parts.next() == Some(Component::RootDir)
}

/// Finds the first of `app_ids` that has a manifest in one of `libraries`.
pub fn find_game(libraries: &[PathBuf], app_ids: &[u32]) -> Result<GameFolder, SteamError> {
    for &app_id in app_ids {
        for library in libraries {
            let apps = library.join("steamapps");
            let manifest = apps.join(format!("appmanifest_{app_id}.acf"));
            if !manifest.is_file() {
                continue;
            }
            let flat = read_text(&manifest)?;
            let install_dir = flat
                .get("appstate/installdir")
                .ok_or_else(|| unreadable(&manifest, "it names no install folder"))?;
            safe_path::check_name(install_dir)
                .map_err(|e| unreadable(&manifest, &e.to_string()))?;
            let flags: u32 = flat
                .get("appstate/stateflags")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            return Ok(GameFolder {
                app_id,
                folder: apps.join("common").join(install_dir),
                installed: flags & FULLY_INSTALLED != 0,
            });
        }
    }
    Err(SteamError::NotStarted)
}

/// The game's Windows launch options from Steam's app cache, the usual one first.
pub fn launch_options(libraries: &[PathBuf], app_id: u32) -> Result<Vec<LaunchOption>, SteamError> {
    let root = libraries.first().ok_or(SteamError::NotInstalled)?;
    let cache = root.join("appcache").join("appinfo.vdf");
    let data = read_capped(&cache, MAX_APP_CACHE)?;
    let app = appinfo::find_app(&data, app_id)
        .map_err(|e| SteamError::Unreadable(e.to_string()))?
        .ok_or(SteamError::NotStarted)?;

    let mut options: Vec<(u8, LaunchOption)> = Vec::new();
    let ids: Vec<&str> = app
        .under("appinfo/config/launch")
        .filter_map(|(key, _)| key.split('/').next())
        .collect();
    let mut seen: Vec<&str> = Vec::new();
    for id in ids {
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        if let Some(option) = launch_option(&app, id) {
            options.push(option);
        }
    }
    options.sort_by_key(|(rank, _)| *rank);
    let mut unique: Vec<LaunchOption> = Vec::new();
    for (_, option) in options {
        if !unique.iter().any(|o| o.exe == option.exe) {
            unique.push(option);
        }
    }
    if unique.is_empty() {
        return Err(SteamError::NoLaunchOptions);
    }
    Ok(unique)
}

/// One launch option, ranked: the default type first, beta-only options last. `None` if it isn't
/// for Windows or its path isn't safe.
fn launch_option(app: &kv::Flat, id: &str) -> Option<(u8, LaunchOption)> {
    let field = |name: &str| app.get(&format!("appinfo/config/launch/{id}/{name}"));
    let exe = ExePath::parse(field("executable")?).ok()?;
    if let Some(os) = field("config/oslist")
        && !os
            .split(',')
            .any(|o| o.trim().eq_ignore_ascii_case("windows"))
    {
        return None;
    }
    let beta = field("config/betakey").is_some();
    let rank = match (field("type"), beta) {
        (_, true) => 3,
        (Some("default"), false) => 0,
        (None, false) => 1,
        _ => 2,
    };
    let label = field("description")
        .unwrap_or("")
        .chars()
        .take(80)
        .collect();
    Some((rank, LaunchOption { exe, label }))
}

fn read_text(path: &Path) -> Result<kv::Flat, SteamError> {
    let bytes = read_capped(path, MAX_TEXT_FILE)?;
    vdf_text::parse(&String::from_utf8_lossy(&bytes)).map_err(|e| unreadable(path, &e.to_string()))
}

fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, SteamError> {
    let file = fs::File::open(path).map_err(|e| unreadable(path, &e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| unreadable(path, &e.to_string()))?;
    if bytes.len() as u64 > cap {
        return Err(unreadable(path, "it's far bigger than expected"));
    }
    Ok(bytes)
}

fn unreadable(path: &Path, why: &str) -> SteamError {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    SteamError::Unreadable(format!("Couldn't read Steam's {name}: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_drive_libraries_are_used() {
        assert!(on_local_drive(Path::new(r"D:\SteamLibrary")));
        assert!(on_local_drive(Path::new("c:/program files (x86)/steam")));
        assert!(!on_local_drive(Path::new(r"\\server\share\Steam")));
        assert!(!on_local_drive(Path::new(r"\\?\UNC\server\share")));
        assert!(!on_local_drive(Path::new(r"\\?\C:\Steam")));
        assert!(!on_local_drive(Path::new("D:relative")));
        assert!(!on_local_drive(Path::new(r"relative\path")));
    }

    /// Reads this PC's Steam: `DQA_STEAM_APP=<installed app id> cargo test -- --ignored`.
    #[test]
    #[ignore = "needs Steam with the given app in a library"]
    fn reads_the_real_steam_install() {
        let app_id: u32 = std::env::var("DQA_STEAM_APP").unwrap().parse().unwrap();
        let libraries = libraries().unwrap();
        let game = find_game(&libraries, &[app_id]).unwrap();
        let options = launch_options(&libraries, app_id).unwrap();
        println!(
            "{libraries:?}
{game:?}"
        );
        for option in &options {
            println!("{} ({})", option.exe, option.label);
        }
        assert!(game.folder.is_dir());
        assert!(options.first().unwrap().exe.under(&game.folder).is_file());
    }
}
