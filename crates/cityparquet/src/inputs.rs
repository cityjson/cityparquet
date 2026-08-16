//! Resolve CLI input patterns (files, directories, glob patterns) into a
//! concrete, de-duplicated, sorted list of source files.
//!
//! A pattern is one of:
//! - a **file** — used directly;
//! - a **directory** — its immediate children whose extension is one of
//!   `json`/`jsonl`/`gml` are collected (non-recursive);
//! - a **glob** (contains `*`, `?`, or `[`) — expanded with [`glob::glob`];
//!   matches that are files are kept, matches that are not (directories, …)
//!   are skipped and reported back on [`ResolvedInputs::skipped_non_files`]
//!   for the caller to surface.
//!
//! The result is canonicalised for de-duplication and sorted for deterministic
//! ordering; an empty resolution is an error.

use std::path::{Path, PathBuf};

use cityparquet_schema::{CityParquetError, Result};

const RECOGNISED_EXTS: [&str; 3] = ["json", "jsonl", "gml"];

fn is_recognised(p: &Path) -> bool {
    p.is_file()
        && p.extension()
            .and_then(|e| e.to_str())
            .map(|e| RECOGNISED_EXTS.contains(&e))
            .unwrap_or(false)
}

fn looks_like_glob(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// The outcome of resolving CLI input patterns: the concrete files, plus
/// every glob match skipped because it is not a plain file (directories,
/// sockets, …) — carried on the value instead of `eprintln!`'d from library
/// code, so the caller decides how to surface it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInputs {
    pub files: Vec<PathBuf>,
    pub skipped_non_files: Vec<PathBuf>,
}

/// Expand `patterns` (files, directories, globs) into the concrete list of
/// source files to convert — canonicalised for de-duplication and sorted.
pub fn resolve_inputs(patterns: &[PathBuf]) -> Result<ResolvedInputs> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut skipped_non_files: Vec<PathBuf> = Vec::new();
    for pat in patterns {
        let s = pat.to_string_lossy();
        if looks_like_glob(&s) {
            let entries =
                glob::glob(&s).map_err(|e| CityParquetError::Io(format!("bad glob {s}: {e}")))?;
            for entry in entries {
                let p = entry.map_err(|e| CityParquetError::Io(format!("glob error: {e}")))?;
                if p.is_file() {
                    out.push(p);
                } else {
                    skipped_non_files.push(p);
                }
            }
        } else if pat.is_dir() {
            let mut children: Vec<PathBuf> = Vec::new();
            for entry in std::fs::read_dir(pat).map_err(|e| {
                CityParquetError::Io(format!("cannot read dir {}: {e}", pat.display()))
            })? {
                // Propagate a per-entry read error rather than silently
                // omitting a file the user asked to convert.
                let entry = entry.map_err(|e| {
                    CityParquetError::Io(format!("cannot read entry in {}: {e}", pat.display()))
                })?;
                let p = entry.path();
                if is_recognised(&p) {
                    children.push(p);
                }
            }
            children.sort();
            out.append(&mut children);
        } else if pat.is_file() {
            out.push(pat.clone());
        } else {
            return Err(CityParquetError::Io(format!(
                "input not found: {}",
                pat.display()
            )));
        }
    }

    // Canonicalise for de-dup; fall back to the raw path if canonicalisation
    // fails (e.g. a path that no longer exists — already an error above, but
    // defensive here so de-dup never silently drops a distinct path).
    let mut seen = std::collections::BTreeSet::new();
    let mut deduped = Vec::new();
    for p in out {
        let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.insert(key) {
            deduped.push(p);
        }
    }
    deduped.sort();
    if deduped.is_empty() {
        return Err(CityParquetError::Io("no input files resolved".to_string()));
    }
    Ok(ResolvedInputs {
        files: deduped,
        skipped_non_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, "{}").unwrap();
        p
    }

    #[test]
    fn resolves_directory_to_recognised_children_sorted() {
        let d = tempfile::tempdir().unwrap();
        touch(d.path(), "b.city.jsonl");
        touch(d.path(), "a.city.json");
        touch(d.path(), "c.gml");
        touch(d.path(), "ignore.txt");
        let sub = d.path().join("nested");
        fs::create_dir(&sub).unwrap();
        touch(&sub, "deep.city.json"); // non-recursive: excluded

        let got = resolve_inputs(&[d.path().to_path_buf()]).unwrap();
        let names: Vec<_> = got
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["a.city.json", "b.city.jsonl", "c.gml"]);
    }

    #[test]
    fn glob_and_explicit_file_are_deduped() {
        let d = tempfile::tempdir().unwrap();
        let a = touch(d.path(), "a.city.json");
        touch(d.path(), "b.city.json");
        let pat = d.path().join("*.city.json");
        let got = resolve_inputs(&[pat, a.clone()]).unwrap();
        assert_eq!(got.files.len(), 2, "duplicate a.city.json must collapse");
        assert!(got.skipped_non_files.is_empty());
    }

    #[test]
    fn glob_matching_a_directory_is_skipped_and_reported() {
        let d = tempfile::tempdir().unwrap();
        touch(d.path(), "a.json");
        let sub = d.path().join("b.json"); // a DIRECTORY whose name matches the glob
        fs::create_dir(&sub).unwrap();
        let got = resolve_inputs(&[d.path().join("*.json")]).unwrap();
        assert_eq!(got.files.len(), 1);
        assert_eq!(got.skipped_non_files, vec![sub]);
    }

    #[test]
    fn empty_resolution_is_error() {
        let d = tempfile::tempdir().unwrap();
        assert!(resolve_inputs(&[d.path().to_path_buf()]).is_err());
    }
}
