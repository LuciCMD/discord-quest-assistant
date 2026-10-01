//! Steam's local app cache, `appcache/appinfo.vdf`: binary key-values for every app Steam has seen.
//! Once a download has started, it holds that game's install folder and launch options, so no
//! network lookup or Steam login is needed. Versions 28 and 29 of the format are read.

use std::fmt;

use super::kv::{Flat, MAX_DEPTH};

const V28: u32 = 0x0756_4428;
const V29: u32 = 0x0756_4429;
/// Bytes between an entry's size field and its key-values: info state, last updated, PICS token,
/// text SHA-1, change number and binary SHA-1.
const ENTRY_HEADER: usize = 4 + 4 + 8 + 20 + 4 + 20;
/// Most apps scanned, far above the few thousand a real cache holds.
const MAX_APPS: usize = 2_000_000;
/// Most keys in the v29 string table.
const MAX_STRINGS: usize = 4_000_000;

#[derive(Debug, PartialEq, Eq)]
pub enum AppInfoError {
    UnknownVersion(u32),
    Damaged(&'static str),
}

impl fmt::Display for AppInfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVersion(magic) => {
                write!(
                    f,
                    "Steam's app cache is in a format this version can't read ({magic:#x})"
                )
            }
            Self::Damaged(what) => write!(f, "Steam's app cache looks damaged ({what})"),
        }
    }
}

/// A bounds-checked reader over the file.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], AppInfoError> {
        let end = self
            .at
            .checked_add(n)
            .ok_or(AppInfoError::Damaged("size"))?;
        let bytes = self
            .data
            .get(self.at..end)
            .ok_or(AppInfoError::Damaged("ends early"))?;
        self.at = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], AppInfoError> {
        self.take(N)?
            .try_into()
            .map_err(|_| AppInfoError::Damaged("size"))
    }

    fn u32(&mut self) -> Result<u32, AppInfoError> {
        self.array().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, AppInfoError> {
        self.array().map(u64::from_le_bytes)
    }

    fn cstr(&mut self) -> Result<String, AppInfoError> {
        let rest = self.data.get(self.at..).unwrap_or_default();
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(AppInfoError::Damaged("unterminated string"))?;
        let text = String::from_utf8_lossy(self.take(len)?).into_owned();
        self.take(1)?;
        Ok(text)
    }

    fn wide_str(&mut self) -> Result<String, AppInfoError> {
        let mut units = Vec::new();
        loop {
            let unit = u16::from_le_bytes(self.array()?);
            if unit == 0 {
                return Ok(String::from_utf16_lossy(&units));
            }
            units.push(unit);
        }
    }
}

/// Finds one app in the cache. `None` if Steam hasn't cached it.
pub fn find_app(data: &[u8], app_id: u32) -> Result<Option<Flat>, AppInfoError> {
    let mut reader = Reader { data, at: 0 };
    let magic = reader.u32()?;
    let _universe = reader.u32()?;
    let strings = match magic {
        V28 => None,
        V29 => Some(string_table(data, reader.u64()?)?),
        other => return Err(AppInfoError::UnknownVersion(other)),
    };
    for _ in 0..MAX_APPS {
        let id = reader.u32()?;
        if id == 0 {
            return Ok(None);
        }
        let size = usize::try_from(reader.u32()?).map_err(|_| AppInfoError::Damaged("size"))?;
        let body = reader.take(size)?;
        if id == app_id {
            let kv = body
                .get(ENTRY_HEADER..)
                .ok_or(AppInfoError::Damaged("short entry"))?;
            return read_kv(kv, strings.as_deref()).map(Some);
        }
    }
    Err(AppInfoError::Damaged("too many apps"))
}

/// The v29 key table: a count, then that many null-terminated strings.
fn string_table(data: &[u8], offset: u64) -> Result<Vec<String>, AppInfoError> {
    let at = usize::try_from(offset).map_err(|_| AppInfoError::Damaged("string table"))?;
    let mut reader = Reader { data, at };
    let count = usize::try_from(reader.u32()?).map_err(|_| AppInfoError::Damaged("size"))?;
    if count > MAX_STRINGS {
        return Err(AppInfoError::Damaged("string table"));
    }
    (0..count).map(|_| reader.cstr()).collect()
}

/// One app's binary key-values, read iteratively with a depth limit.
fn read_kv(data: &[u8], strings: Option<&[String]>) -> Result<Flat, AppInfoError> {
    let mut reader = Reader { data, at: 0 };
    let mut flat = Flat::default();
    let mut path: Vec<String> = Vec::new();
    // Every pass consumes at least one byte, so the data's length bounds the loop.
    for _ in 0..=data.len() {
        let kind = reader.array::<1>()?[0];
        if kind == 0x08 || kind == 0x0B {
            if path.pop().is_none() {
                return Ok(flat);
            }
            continue;
        }
        let key = match strings {
            None => reader.cstr()?,
            Some(table) => {
                let index = usize::try_from(reader.u32()?).unwrap_or(usize::MAX);
                table
                    .get(index)
                    .cloned()
                    .ok_or(AppInfoError::Damaged("key index"))?
            }
        };
        let value = match kind {
            0x00 => {
                if path.len() >= MAX_DEPTH {
                    return Err(AppInfoError::Damaged("nested too deeply"));
                }
                path.push(key.to_ascii_lowercase());
                continue;
            }
            0x01 => reader.cstr()?,
            0x02 => i32::from_le_bytes(reader.array()?).to_string(),
            0x03 => f32::from_le_bytes(reader.array()?).to_string(),
            0x04 | 0x06 => reader.u32()?.to_string(),
            0x05 => reader.wide_str()?,
            0x07 => reader.u64()?.to_string(),
            0x0A => i64::from_le_bytes(reader.array()?).to_string(),
            _ => return Err(AppInfoError::Damaged("unknown value type")),
        };
        if !flat.push(&path, &key, value) {
            return Err(AppInfoError::Damaged("too many values"));
        }
    }
    Err(AppInfoError::Damaged("never ends"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a v29 cache with one app, keyed through a string table.
    fn sample_v29(app_id: u32) -> Vec<u8> {
        let strings = [
            "appinfo",
            "config",
            "installdir",
            "launch",
            "0",
            "executable",
        ];
        let mut kv = Vec::new();
        let key = |kv: &mut Vec<u8>, kind: u8, name: &str| {
            kv.push(kind);
            let index = strings.iter().position(|s| *s == name).unwrap();
            kv.extend_from_slice(&u32::try_from(index).unwrap().to_le_bytes());
        };
        key(&mut kv, 0x00, "appinfo");
        key(&mut kv, 0x00, "config");
        key(&mut kv, 0x01, "installdir");
        kv.extend_from_slice(b"Some Game\0");
        key(&mut kv, 0x00, "launch");
        key(&mut kv, 0x00, "0");
        key(&mut kv, 0x01, "executable");
        kv.extend_from_slice(b"bin\\game.exe\0");
        kv.extend_from_slice(&[0x08, 0x08, 0x08, 0x08, 0x08]);

        let mut body = vec![0u8; ENTRY_HEADER];
        body.extend_from_slice(&kv);
        let mut out = Vec::new();
        out.extend_from_slice(&V29.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        let table_at_field = out.len();
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&app_id.to_le_bytes());
        out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&0u32.to_le_bytes());
        let table_at = u64::try_from(out.len()).unwrap();
        out[table_at_field..table_at_field + 8].copy_from_slice(&table_at.to_le_bytes());
        out.extend_from_slice(&u32::try_from(strings.len()).unwrap().to_le_bytes());
        for s in strings {
            out.extend_from_slice(s.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn finds_an_apps_launch_options() {
        let data = sample_v29(4242);
        let app = find_app(&data, 4242).unwrap().unwrap();
        assert_eq!(app.get("appinfo/config/installdir"), Some("Some Game"));
        assert_eq!(
            app.get("appinfo/config/launch/0/executable"),
            Some("bin\\game.exe")
        );
        assert!(find_app(&data, 7).unwrap().is_none());
    }

    #[test]
    fn damaged_caches_are_errors_not_panics() {
        let data = sample_v29(4242);
        for cut in [0, 3, 8, 20, data.len() / 2, data.len() - 3] {
            let _ = find_app(data.get(..cut).unwrap(), 4242);
        }
        let mut wrong = data.clone();
        wrong[0] = 0;
        assert!(matches!(
            find_app(&wrong, 4242),
            Err(AppInfoError::UnknownVersion(_))
        ));
    }
}
