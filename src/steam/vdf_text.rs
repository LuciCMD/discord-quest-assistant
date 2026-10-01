//! Steam's text key-value files: `libraryfolders.vdf` and `appmanifest_<id>.acf`.

use std::fmt;

use super::kv::{Flat, MAX_DEPTH};

#[derive(Debug, PartialEq, Eq)]
pub enum VdfError {
    Unbalanced,
    TooDeep,
    TooBig,
    Unterminated,
}

impl fmt::Display for VdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Unbalanced => "its braces don't match",
            Self::TooDeep => "it's nested too deeply",
            Self::TooBig => "it has too many values",
            Self::Unterminated => "a quoted string never ends",
        };
        f.write_str(text)
    }
}

enum Token {
    Text(String),
    Open,
    Close,
}

/// Reads a whole file into a flat list of key paths and values.
pub fn parse(text: &str) -> Result<Flat, VdfError> {
    let mut flat = Flat::default();
    let mut path: Vec<String> = Vec::new();
    let mut key: Option<String> = None;
    for token in tokenize(text)? {
        match token {
            Token::Text(word) => match key.take() {
                None => key = Some(word),
                Some(name) => {
                    if !flat.push(&path, &name, word) {
                        return Err(VdfError::TooBig);
                    }
                }
            },
            Token::Open => {
                let name = key.take().ok_or(VdfError::Unbalanced)?;
                if path.len() >= MAX_DEPTH {
                    return Err(VdfError::TooDeep);
                }
                path.push(name.to_ascii_lowercase());
            }
            Token::Close => {
                if key.is_some() || path.pop().is_none() {
                    return Err(VdfError::Unbalanced);
                }
            }
        }
    }
    if path.is_empty() && key.is_none() {
        Ok(flat)
    } else {
        Err(VdfError::Unbalanced)
    }
}

/// Splits the text into quoted or bare words and braces. `//` comments and `[$WIN32]`-style
/// conditions are skipped.
fn tokenize(text: &str) -> Result<Vec<Token>, VdfError> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '"' => tokens.push(Token::Text(quoted(&mut chars)?)),
            '/' if chars.peek() == Some(&'/') => {
                chars.by_ref().find(|&n| n == '\n');
            }
            c if c.is_whitespace() => {}
            c => {
                let mut word = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    word.push(n);
                    chars.next();
                }
                if !word.starts_with('[') {
                    tokens.push(Token::Text(word));
                }
            }
        }
    }
    Ok(tokens)
}

fn quoted(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<String, VdfError> {
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Ok(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => break,
            },
            c => out.push(c),
        }
    }
    Err(VdfError::Unterminated)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARIES: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps"
		{
			"230410"		"43150037815"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary" // a comment
	}
}
"#;

    #[test]
    fn reads_library_folders() {
        let flat = parse(LIBRARIES).unwrap();
        assert_eq!(
            flat.get("libraryfolders/0/path"),
            Some(r"C:\Program Files (x86)\Steam")
        );
        assert_eq!(flat.get("libraryfolders/1/path"), Some(r"D:\SteamLibrary"));
        assert_eq!(
            flat.get("libraryfolders/0/apps/230410"),
            Some("43150037815")
        );
    }

    #[test]
    fn keys_are_case_insensitive() {
        let flat = parse(r#""AppState" { "installdir" "Warframe" "StateFlags" "4" }"#).unwrap();
        assert_eq!(flat.get("appstate/installdir"), Some("Warframe"));
        assert_eq!(flat.get("appstate/stateflags"), Some("4"));
    }

    #[test]
    fn rejects_damaged_files() {
        assert!(parse(r#""a" { "b" "c" "#).is_err());
        assert!(parse(r#""a" } "#).is_err());
        assert!(parse(r#""a" "unterminated"#).is_err());
        let deep = "\"k\" {".repeat(MAX_DEPTH + 1);
        assert_eq!(parse(&deep).err(), Some(VdfError::TooDeep));
    }
}
