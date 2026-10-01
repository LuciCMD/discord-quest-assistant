//! Paths that come from outside the app (Discord's list, Steam's files, the user) are checked here
//! before they touch the disk. A checked path is relative, has no `..`, no drive or root, and no
//! name Windows treats specially, so joining it to a folder can never land outside that folder.

use std::fmt;
use std::path::{Path, PathBuf};

/// Deepest path accepted. Discord's list goes six folders deep at most.
const MAX_DEPTH: usize = 10;
/// Longest single file or folder name accepted.
const MAX_PART: usize = 160;

/// Names Windows reserves for devices, with or without an extension.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    Empty,
    TooDeep,
    BadName(String),
    NotExe,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the path is empty"),
            Self::TooDeep => write!(f, "the path has more than {MAX_DEPTH} folders"),
            Self::BadName(name) => write!(f, "\"{name}\" isn't a name Windows allows here"),
            Self::NotExe => write!(f, "the path doesn't end in an .exe file"),
        }
    }
}

/// A checked relative path to an `.exe`, such as `win64/game.exe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExePath {
    parts: Vec<String>,
}

impl ExePath {
    /// Checks a path written with `/` or `\` between its parts.
    pub fn parse(raw: &str) -> Result<Self, PathError> {
        let trimmed = raw.trim().trim_start_matches(['/', '\\']);
        if trimmed.is_empty() {
            return Err(PathError::Empty);
        }
        let parts: Vec<String> = trimmed.split(['/', '\\']).map(str::to_owned).collect();
        if parts.len() > MAX_DEPTH {
            return Err(PathError::TooDeep);
        }
        for part in &parts {
            check_name(part)?;
        }
        let file = parts.last().ok_or(PathError::Empty)?;
        if !file.to_ascii_lowercase().ends_with(".exe") || file.len() <= ".exe".len() {
            return Err(PathError::NotExe);
        }
        Ok(Self { parts })
    }

    /// The path under `base`. The parts were checked, so the result is always inside `base`.
    pub fn under(&self, base: &Path) -> PathBuf {
        let mut path = base.to_path_buf();
        for part in &self.parts {
            path.push(part);
        }
        debug_assert!(path.starts_with(base) && path != base);
        path
    }
}

impl fmt::Display for ExePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.parts.join("/"))
    }
}

/// Checks one file or folder name, such as a Steam install folder.
pub fn check_name(name: &str) -> Result<(), PathError> {
    let bad = || Err(PathError::BadName(name.chars().take(60).collect()));
    if name.is_empty() || name.len() > MAX_PART || name == "." || name == ".." {
        return bad();
    }
    if name.trim() != name || name.ends_with('.') {
        // Windows silently drops trailing dots and spaces, so the name on disk would differ.
        return bad();
    }
    let forbidden = |c: char| c.is_control() || "<>:\"/\\|?*".contains(c);
    if name.chars().any(forbidden) {
        return bad();
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    if RESERVED.contains(&stem.trim_end()) {
        return bad();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_shapes_in_discords_list() {
        for raw in [
            "overwatch.exe",
            "win64/primalcarnagegame.exe",
            "dark souls prepare to die edition/data/darksouls.exe",
            "rukimins disappointing adventure/rukimin's disappointing adventure!.exe",
            "missedmessageswindows/missed messages..exe",
            "oh...sir! the insult simulator/ohsir.exe",
            r"Tools\Launcher.exe",
        ] {
            assert!(ExePath::parse(raw).is_ok(), "{raw}");
        }
    }

    #[test]
    fn rejects_anything_that_could_leave_its_folder() {
        for raw in [
            "",
            "../evil.exe",
            "a/../../evil.exe",
            "C:/Windows/evil.exe",
            "c:evil.exe",
            "a/./b.exe",
            "a//b.exe",
            "con.exe",
            "com1/b.exe",
            "trailing./b.exe",
            "space /b.exe",
            "a/b.dll",
            ".exe",
            "a/b?.exe",
        ] {
            assert!(ExePath::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn a_leading_slash_is_only_a_separator() {
        let path = ExePath::parse("/win64/game.exe").unwrap();
        assert_eq!(path.to_string(), "win64/game.exe");
    }

    #[test]
    fn joined_paths_stay_under_their_base() {
        let base = Path::new(r"C:\base");
        let path = ExePath::parse("a/b/c.exe").unwrap().under(base);
        assert!(path.starts_with(base));
        assert_eq!(path, Path::new(r"C:\base\a\b\c.exe"));
    }
}
