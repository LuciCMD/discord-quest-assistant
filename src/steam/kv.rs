//! Steam's key-value data, text or binary, read into one flat list: each value under its key path,
//! such as `appstate/installdir`. Keys are lowercased because Steam's own casing varies.

/// Deepest nesting accepted. Steam's files go about six levels deep.
pub const MAX_DEPTH: usize = 32;
/// Most values kept from one file or one app.
pub const MAX_ENTRIES: usize = 200_000;

#[derive(Debug, Default)]
pub struct Flat {
    entries: Vec<(String, String)>,
}

impl Flat {
    /// Adds a value. Returns false once the list is full.
    pub fn push(&mut self, path: &[String], key: &str, value: String) -> bool {
        if self.entries.len() >= MAX_ENTRIES {
            return false;
        }
        let mut full = path.join("/");
        if !full.is_empty() {
            full.push('/');
        }
        full.push_str(&key.to_ascii_lowercase());
        self.entries.push((full, value));
        true
    }

    /// The value at an exact key path.
    pub fn get(&self, path: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(key, _)| key == path)
            .map(|(_, value)| value.as_str())
    }

    /// Every key path and value under `prefix/`, in file order.
    pub fn under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.entries.iter().filter_map(move |(key, value)| {
            key.strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
                .map(|rest| (rest, value.as_str()))
        })
    }
}
