//! Include resolution following GCC/Clang search order.

use crate::compdb::SearchPaths;
use crate::fs::{normalize, SourceProvider};
use std::path::{Path, PathBuf};

/// Resolves one `#include` spelling.
///
/// Quoted includes search the including file's directory, then `-iquote`,
/// `-I`, `-isystem` and `-idirafter` directories. Angled includes skip the
/// first two. Built-in compiler system directories are not searched: they are
/// unknown without running the compiler, so such headers come back unresolved.
pub fn resolve(
    fs: &dyn SourceProvider,
    includer: &Path,
    spelling: &str,
    angled: bool,
    search: &SearchPaths,
) -> Option<PathBuf> {
    let rel = Path::new(spelling);
    if rel.is_absolute() {
        let p = normalize(rel);
        return fs.is_file(&p).then_some(p);
    }
    candidates(includer, angled, search)
        .map(|dir| normalize(&dir.join(rel)))
        .find(|p| fs.is_file(p))
}

fn candidates<'a>(
    includer: &'a Path,
    angled: bool,
    search: &'a SearchPaths,
) -> impl Iterator<Item = &'a Path> + 'a {
    let local = (!angled).then(|| includer.parent()).flatten();
    let quote_dirs: &[PathBuf] = if angled { &[] } else { &search.iquote };
    local
        .into_iter()
        .chain(quote_dirs.iter().map(PathBuf::as_path))
        .chain(search.include.iter().map(PathBuf::as_path))
        .chain(search.system.iter().map(PathBuf::as_path))
        .chain(search.after.iter().map(PathBuf::as_path))
}
