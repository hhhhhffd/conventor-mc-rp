use std::collections::BTreeMap;

/// In-memory resource pack: normalized relative paths (`/`-separated) to file contents.
#[derive(Debug, Default, Clone)]
pub struct Pack {
    files: BTreeMap<String, Vec<u8>>,
}

/// Normalizes an archive entry path. Returns `None` for paths that must not be kept
/// (absolute traversal, `..` components, empty names).
pub fn normalize_path(raw: &str) -> Option<String> {
    let unified = raw.replace('\\', "/");
    let mut parts = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

impl Pack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn insert(&mut self, path: impl Into<String>, data: Vec<u8>) {
        self.files.insert(path.into(), data);
    }

    pub fn remove(&mut self, path: &str) -> Option<Vec<u8>> {
        self.files.remove(path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// All paths starting with `prefix`, collected so the pack can be mutated while iterating.
    pub fn paths_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.files
            .range(prefix.to_string()..)
            .take_while(|(k, _)| k.starts_with(prefix))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Moves a file. Returns `true` if the source existed. An existing destination is overwritten.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        if from == to {
            return self.files.contains_key(from);
        }
        match self.files.remove(from) {
            Some(data) => {
                self.files.insert(to.to_string(), data);
                true
            }
            None => false,
        }
    }

    /// Moves every file under directory `from` to directory `to` (both without trailing `/`).
    /// Returns the number of moved files.
    pub fn rename_dir(&mut self, from: &str, to: &str) -> usize {
        let prefix = format!("{from}/");
        let paths = self.paths_with_prefix(&prefix);
        for path in &paths {
            let dest = format!("{to}/{}", &path[prefix.len()..]);
            self.rename(path, &dest);
        }
        paths.len()
    }

    /// Strips a single wrapping directory when the pack root (`pack.mcmeta`) is nested inside it,
    /// e.g. archives created by zipping the pack folder instead of its contents.
    pub fn strip_wrapping_dir(&mut self) {
        if self.files.contains_key("pack.mcmeta") {
            return;
        }
        let Some(root) = self
            .files
            .keys()
            .filter(|p| p.ends_with("/pack.mcmeta"))
            .min_by_key(|p| p.matches('/').count())
            .map(|p| p[..p.len() - "pack.mcmeta".len()].to_string())
        else {
            return;
        };
        let files = std::mem::take(&mut self.files);
        self.files = files
            .into_iter()
            .filter_map(|(k, v)| k.strip_prefix(&root).map(|s| (s.to_string(), v)))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize_path("./a\\b//c.png").as_deref(), Some("a/b/c.png"));
        assert_eq!(normalize_path("/a/b"), Some("a/b".into()));
        assert_eq!(normalize_path("a/../b"), None);
        assert_eq!(normalize_path("/"), None);
    }

    #[test]
    fn strips_wrapping_dir() {
        let mut pack = Pack::new();
        pack.insert("MyPack/pack.mcmeta", b"{}".to_vec());
        pack.insert("MyPack/assets/minecraft/a.png", vec![1]);
        pack.insert("__MACOSX/junk", vec![2]);
        pack.strip_wrapping_dir();
        assert!(pack.contains("pack.mcmeta"));
        assert!(pack.contains("assets/minecraft/a.png"));
        assert_eq!(pack.len(), 2);
    }

    #[test]
    fn renames_dirs() {
        let mut pack = Pack::new();
        pack.insert("a/x/1", vec![]);
        pack.insert("a/x/2", vec![]);
        pack.insert("a/xy", vec![]);
        assert_eq!(pack.rename_dir("a/x", "b"), 2);
        assert!(pack.contains("b/1") && pack.contains("b/2") && pack.contains("a/xy"));
    }
}
