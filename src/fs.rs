//! Source access. Analysis goes through [`SourceProvider`] so the tests and the
//! benchmark can run on an in-memory tree with exactly the same code path as a
//! real checkout on disk.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

pub trait SourceProvider: Sync {
    fn read(&self, path: &Path) -> Option<Vec<u8>>;
    fn is_file(&self, path: &Path) -> bool;
}

/// Reads from the real filesystem.
pub struct DiskFs;

impl SourceProvider for DiskFs {
    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
}

/// An in-memory file tree keyed by normalized absolute path.
#[derive(Default, Clone, Debug)]
pub struct MemFs {
    files: HashMap<PathBuf, Vec<u8>>,
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: impl AsRef<Path>, contents: impl Into<Vec<u8>>) {
        self.files.insert(normalize(path.as_ref()), contents.into());
    }

    pub fn remove(&mut self, path: impl AsRef<Path>) {
        self.files.remove(&normalize(path.as_ref()));
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl SourceProvider for MemFs {
    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        self.files.get(&normalize(path)).cloned()
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(&normalize(path))
    }
}

/// Lexical normalization: removes `.` and folds `dir/..`.
///
/// Symlinks are deliberately not resolved, so the same header reached through
/// two spellings of one real path is only merged when the spellings normalize
/// to the same string. That keeps analysis independent of the machine layout.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => out.push(comp),
            },
            other => out.push(other),
        }
    }
    let mut buf = PathBuf::new();
    for comp in out {
        buf.push(comp.as_os_str());
    }
    if buf.as_os_str().is_empty() {
        buf.push(".");
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_dots() {
        assert_eq!(
            normalize(Path::new("/a/./b/../c.h")),
            PathBuf::from("/a/c.h")
        );
        assert_eq!(normalize(Path::new("/../x")), PathBuf::from("/x"));
        assert_eq!(normalize(Path::new("../x/./y")), PathBuf::from("../x/y"));
        assert_eq!(normalize(Path::new("a/..")), PathBuf::from("."));
    }

    #[test]
    fn memfs_lookups_are_normalized() {
        let mut fs = MemFs::new();
        fs.insert("/p/include/../src/a.h", "x");
        assert!(fs.is_file(Path::new("/p/src/./a.h")));
        assert_eq!(fs.read(Path::new("/p/src/a.h")), Some(b"x".to_vec()));
        fs.remove("/p/src/a.h");
        assert!(fs.is_empty());
    }
}
