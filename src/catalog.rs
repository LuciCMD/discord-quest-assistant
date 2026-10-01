//! Discord's list of games it detects, and which executable each one is detected by. This is the
//! app's only network request: one public download, no account or token, cached on disk.

use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::paths::{now_unix, write_atomic};
use crate::safe_path::ExePath;

const URL: &str = "https://discord.com/api/v9/applications/detectable";
/// The list is about 13 MB; a much bigger answer isn't the list.
const MAX_DOWNLOAD: u64 = 96 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(90);
/// The cached list is refreshed at start once it is this old.
pub const STALE_AFTER_SECS: u64 = 24 * 60 * 60;
const MAX_NAME: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Checked relative paths, Discord's way round: lower case, `/` between parts.
    pub exes: Vec<String>,
    /// Steam app IDs Discord links to the game.
    #[serde(default)]
    pub steam: Vec<u32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub fetched_unix: u64,
    pub games: Vec<Game>,
}

#[derive(Debug)]
pub struct CatalogError(String);

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Deserialize)]
struct RawApp {
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    executables: Vec<RawExe>,
    #[serde(default)]
    third_party_skus: Vec<RawSku>,
}

#[derive(Deserialize)]
struct RawExe {
    name: String,
    os: String,
    #[serde(default)]
    is_launcher: bool,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct RawSku {
    distributor: String,
    #[serde(default)]
    id: Option<String>,
}

impl Catalog {
    /// Downloads the list from Discord. Blocks; run it off the interface thread.
    pub fn download() -> Result<Self, CatalogError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .https_only(true)
            .user_agent(concat!(
                "discord-quest-assistant/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .into();
        let mut response = agent
            .get(URL)
            .call()
            .map_err(|e| CatalogError(format!("Couldn't reach Discord: {e}")))?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD)
            .read_to_vec()
            .map_err(|e| CatalogError(format!("Discord's game list didn't download: {e}")))?;
        Self::from_discord(&bytes, now_unix())
    }

    /// Keeps what detection needs from Discord's answer, and only paths that pass the checks.
    pub fn from_discord(bytes: &[u8], fetched_unix: u64) -> Result<Self, CatalogError> {
        let raw: Vec<RawApp> = serde_json::from_slice(bytes)
            .map_err(|e| CatalogError(format!("Discord's game list wasn't readable: {e}")))?;
        let games: Vec<Game> = raw.into_iter().filter_map(game_from_raw).collect();
        if games.is_empty() {
            return Err(CatalogError("Discord's game list was empty".into()));
        }
        Ok(Self {
            fetched_unix,
            games,
        })
    }

    pub fn load(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice::<Self>(&bytes)
            .ok()
            .filter(|c| !c.games.is_empty())
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        write_atomic(path, &bytes)
    }

    pub fn is_stale(&self) -> bool {
        now_unix().saturating_sub(self.fetched_unix) > STALE_AFTER_SECS
    }

    /// Indexes of the games whose name or alias contains `query`, best matches first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(u8, usize, usize)> = Vec::new();
        for (index, game) in self.games.iter().enumerate() {
            let names = std::iter::once(&game.name).chain(game.aliases.iter());
            let best = names
                .filter_map(|name| rank(&name.to_lowercase(), &needle))
                .min();
            if let Some(score) = best {
                hits.push((score, game.name.len(), index));
            }
        }
        hits.sort_unstable();
        hits.into_iter().take(limit).map(|(_, _, i)| i).collect()
    }
}

/// 0 for an exact name, 1 for a name that starts with the query, 2 for one that contains it.
fn rank(name: &str, needle: &str) -> Option<u8> {
    if name == needle {
        Some(0)
    } else if name.starts_with(needle) {
        Some(1)
    } else if name.contains(needle) {
        Some(2)
    } else {
        None
    }
}

fn game_from_raw(raw: RawApp) -> Option<Game> {
    let name = raw.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return None;
    }
    let mut exes: Vec<String> = Vec::new();
    for exe in raw.executables {
        // Discord marks exact-name matches with `>` and some need arguments; neither can be
        // reproduced by a renamed copy, so they're left out.
        let usable = exe.os == "win32" && !exe.is_launcher && exe.arguments.is_none();
        if !usable || exe.name.starts_with('>') {
            continue;
        }
        if let Ok(path) = ExePath::parse(&exe.name) {
            let text = path.to_string().to_lowercase();
            if !exes.contains(&text) {
                exes.push(text);
            }
        }
    }
    let steam: Vec<u32> = raw
        .third_party_skus
        .iter()
        .filter(|sku| sku.distributor == "steam")
        .filter_map(|sku| sku.id.as_deref()?.parse().ok())
        .collect();
    if exes.is_empty() && steam.is_empty() {
        return None;
    }
    let aliases = raw
        .aliases
        .into_iter()
        .filter(|a| !a.trim().is_empty() && a.chars().count() <= MAX_NAME)
        .take(16)
        .collect();
    Some(Game {
        name: name.to_owned(),
        aliases,
        exes,
        steam,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {"id":"1","name":"Where Winds Meet","aliases":[],"executables":[
        {"is_launcher":false,"name":"wwm.exe","os":"win32"},
        {"is_launcher":false,"name":"where winds meet.exe","os":"win32"}],
       "third_party_skus":[{"distributor":"steam","id":"3564740"}],"hook":false},
      {"id":"2","name":"Marathon","aliases":[],"executables":[],
       "third_party_skus":[{"distributor":"steam","id":"3065800"},{"distributor":"xbox","id":null}]},
      {"id":"3","name":"Nothing Usable","executables":[
        {"is_launcher":false,"name":"../../evil.exe","os":"win32"},
        {"is_launcher":false,"name":">hl2.exe","os":"win32","arguments":"-game x"},
        {"is_launcher":false,"name":"game","os":"darwin"}],"third_party_skus":[]},
      {"id":"4","name":"Garry's Mod","executables":[
        {"is_launcher":false,"name":"garrysmod/hl2.exe","os":"win32","arguments":"-game garrysmod"},
        {"is_launcher":true,"name":"launcher/launch.exe","os":"win32"},
        {"is_launcher":false,"name":"gmod.exe","os":"win32"}]}
    ]"#;

    #[test]
    fn keeps_only_usable_games_and_paths() {
        let catalog = Catalog::from_discord(SAMPLE.as_bytes(), 1).unwrap();
        let names: Vec<&str> = catalog.games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Where Winds Meet", "Marathon", "Garry's Mod"]);
        assert_eq!(catalog.games[0].exes, ["wwm.exe", "where winds meet.exe"]);
        assert_eq!(catalog.games[1].steam, [3_065_800]);
        assert!(catalog.games[1].exes.is_empty());
        assert_eq!(catalog.games[2].exes, ["gmod.exe"]);
    }

    #[test]
    fn search_puts_closer_names_first() {
        let catalog = Catalog::from_discord(SAMPLE.as_bytes(), 1).unwrap();
        assert_eq!(catalog.search("marathon", 10), [1]);
        assert_eq!(catalog.search("  WINDS ", 10), [0]);
        assert!(catalog.search("", 10).is_empty());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(Catalog::from_discord(b"{\"message\":\"nope\"}", 1).is_err());
        assert!(Catalog::from_discord(b"[]", 1).is_err());
    }

    /// Reads a real download: `DQA_DETECTABLE=<file> cargo test -- --ignored`.
    #[test]
    #[ignore = "needs a downloaded copy of Discord's list"]
    fn reads_the_real_list() {
        let path = std::env::var("DQA_DETECTABLE").unwrap();
        let catalog = Catalog::from_discord(&fs::read(path).unwrap(), 1).unwrap();
        let find = |name: &str| catalog.games.iter().find(|g| g.name == name).unwrap();
        assert!(catalog.games.len() > 10_000);
        assert_eq!(
            find("Where Winds Meet").exes,
            ["wwm.exe", "where winds meet.exe"]
        );
        assert!(find("Marathon").exes.is_empty() && !find("Marathon").steam.is_empty());
        println!("{} usable games", catalog.games.len());
    }
}
