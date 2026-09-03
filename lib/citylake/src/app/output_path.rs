//! Confining an API-supplied output path to a configured root.
//!
//! The HTTP API lets a caller name where a file or a package gets written.
//! Without a check, that name reaches the filesystem verbatim and the caller
//! can write anywhere the server process can reach. [`resolve_output_path`]
//! is that check: it turns a root and a caller-supplied path into either a
//! [`PathBuf`] guaranteed to sit inside the root, or a reason it does not.
//!
//! A textual check on the caller's string is not enough. A symlink inside
//! the root can point anywhere, and no amount of string inspection sees
//! where it leads — only resolving the filesystem does. And a `..` in a
//! part of the path that does not exist yet is invisible to
//! [`std::fs::canonicalize`], which requires every component up to the last
//! one to exist. So the path is resolved in two halves: canonicalise the
//! deepest ancestor that does exist (following any symlinks along the way),
//! then fold the remaining, not-yet-existing components onto it lexically —
//! `ParentDir` pops, `CurDir` is skipped, `Normal` pushes — before comparing
//! the result against the canonicalised root by prefix.
//!
//! This is a check-then-use control, not an atomic one: nothing stops a
//! symlink being planted inside the root between this function returning and
//! the caller opening the path it approved. That gap is accepted residual
//! risk, not fixed here.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// Why a requested output path was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OutputPathError {
    /// No output root is configured, so every write is refused.
    #[error("CITYLAKE_OUTPUT_ROOT is not configured")]
    RootNotConfigured,
    /// The requested path is absolute, so it names a location outside the
    /// root regardless of the root's own value.
    #[error("requested path must be relative")]
    Absolute,
    /// The requested path, once resolved, lies outside the configured root.
    #[error("requested path escapes the configured output root")]
    Escapes,
    /// The configured root does not exist, or an ancestor found while
    /// resolving the requested path could not be canonicalised (it was
    /// removed, or became unreadable, between being found and being
    /// resolved).
    #[error("configured output root does not exist")]
    ParentMissing,
}

/// Resolve `requested` against `root`, refusing anything that would write
/// outside it.
///
/// `root` is `None` when `CITYLAKE_OUTPUT_ROOT` is unset — a legitimate
/// state, refused rather than defaulted, because a control that is on only
/// when configured is not a control. `requested` must be a relative path;
/// it is joined onto the canonicalised root, resolved past any symlinks in
/// its existing prefix, and its remaining components are folded lexically
/// before the result is checked to still sit under the root.
pub fn resolve_output_path(
    root: Option<&str>,
    requested: &str,
) -> Result<PathBuf, OutputPathError> {
    let root = root.ok_or(OutputPathError::RootNotConfigured)?;

    let requested_path = Path::new(requested);
    if requested_path.is_absolute() {
        return Err(OutputPathError::Absolute);
    }

    let canonical_root = std::fs::canonicalize(root).map_err(|_| OutputPathError::ParentMissing)?;

    let full = canonical_root.join(requested_path);

    // The deepest ancestor of `full` that actually exists. `full`'s
    // components may include parts that do not exist yet — that is the
    // normal case for a package about to be created — so this walk finds
    // where resolution has to stop; at worst it stops at `canonical_root`
    // itself, which is guaranteed to exist.
    //
    // `symlink_metadata`, not `exists`: `exists` follows the final symlink
    // and reports a *broken* one as not existing at all, so a dangling link
    // would fall into the lexically-folded remainder below instead of being
    // canonicalised — and the fold never sees where the link points, only
    // its literal name. `symlink_metadata` sees the link entry itself
    // regardless of what it points to (or whether that target exists), so
    // the walk stops on the link and the canonicalisation below — which
    // does follow it — is what refuses it.
    let existing_ancestor = full
        .ancestors()
        .find(|candidate| candidate.symlink_metadata().is_ok())
        .ok_or(OutputPathError::ParentMissing)?;

    // Canonicalising this ancestor is what sees through a symlink planted
    // inside the root — a textual check on `requested` cannot.
    let canonical_ancestor =
        std::fs::canonicalize(existing_ancestor).map_err(|_| OutputPathError::ParentMissing)?;

    let remainder = full
        .strip_prefix(existing_ancestor)
        .expect("existing_ancestor is an ancestor of full by construction");

    let resolved = fold_onto(&canonical_ancestor, remainder);

    if resolved.starts_with(&canonical_root) {
        Ok(resolved)
    } else {
        Err(OutputPathError::Escapes)
    }
}

/// Fold `remainder`'s components onto `base` lexically.
///
/// `base` is already canonical — absolute, and free of `.`/`..` — so this
/// only has to resolve the components `remainder` contributes: `ParentDir`
/// pops the last pushed component, `CurDir` is skipped, `Normal` pushes.
/// `std::path` has no normalising method, and `canonicalize` cannot help
/// here because part of the combined path does not exist on disk yet — so
/// this is the step that catches a `..` hidden in that not-yet-existing
/// part, which resolving only `base` would miss entirely.
fn fold_onto(base: &Path, remainder: &Path) -> PathBuf {
    let mut components: Vec<Component> = base.components().collect();
    // Never pop the filesystem root itself off the stack — popping past it
    // is meaningless, not a further escape.
    let floor = components
        .iter()
        .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        .count();

    for component in remainder.components() {
        match component {
            Component::ParentDir => {
                if components.len() > floor {
                    components.pop();
                }
            }
            Component::CurDir => {}
            Component::Normal(_) => components.push(component),
            // `remainder` comes from stripping a prefix off `base.join(_)`,
            // so it is relative and carries neither of these.
            Component::RootDir | Component::Prefix(_) => {}
        }
    }

    components.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A root with a symlink pointing out of it — the case a textual check passes.
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(dir.path().join("outside")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("outside"), root.join("escape")).unwrap();
        (dir, root)
    }

    #[test]
    fn an_unset_root_refuses_every_path() {
        // A control that is off unless configured is not a control.
        assert!(matches!(
            resolve_output_path(None, "pkg"),
            Err(OutputPathError::RootNotConfigured)
        ));
    }

    #[test]
    fn a_relative_path_resolves_inside_the_root() {
        let (_dir, root) = fixture();
        let got = resolve_output_path(Some(root.to_str().unwrap()), "pkg").unwrap();
        assert!(got.starts_with(fs::canonicalize(&root).unwrap()));
    }

    #[test]
    fn a_deep_path_that_does_not_exist_yet_is_allowed() {
        // `package` write names a directory it is about to create, so a
        // non-existent target is the normal case, not an error.
        let (_dir, root) = fixture();
        let got = resolve_output_path(Some(root.to_str().unwrap()), "a/b/c").unwrap();
        assert!(got.starts_with(fs::canonicalize(&root).unwrap()));
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "/etc/pkg"),
            Err(OutputPathError::Absolute)
        ));
    }

    #[test]
    fn a_parent_traversal_is_refused() {
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "../outside/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_symlink_out_of_the_root_is_refused() {
        // The case that distinguishes a real control from a plausible one: no
        // amount of string inspection sees where a symlink points.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "escape/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_traversal_hidden_in_a_non_existent_remainder_is_refused() {
        // Measured bypass: `newdir` does not exist, so canonicalisation never
        // sees the `..` that follows it. Only normalising the re-attached
        // remainder catches this.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "newdir/../../outside/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_sibling_directory_sharing_a_name_prefix_is_refused() {
        // "root-backup" is not "root" as a path component, even though it
        // shares a string prefix with it. A naive string-prefix comparison
        // (rather than the committed component-wise `Path::starts_with`)
        // would accept this and pass all seven tests above it regardless.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "../root-backup/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_dangling_symlink_inside_the_root_is_refused() {
        // `Path::exists()` follows symlinks and reports a *broken* one as
        // not existing at all — no race required, this is wrong on its own:
        // the link then falls into the lexically-folded remainder instead
        // of being canonicalised, and the fold never sees where the link
        // actually points. `symlink_metadata` sees the link entry itself
        // regardless of whether its target exists, so the ancestor walk
        // stops there and canonicalising it (which does follow the link)
        // fails, refusing the request instead of silently approving it.
        let (_dir, root) = fixture();
        let outside = _dir.path().join("nowhere");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("dangling")).unwrap();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "dangling/pkg"),
            Err(OutputPathError::ParentMissing)
        ));
    }

    #[test]
    fn a_root_that_does_not_exist_is_reported_as_parent_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing_root = dir.path().join("does-not-exist");
        assert!(matches!(
            resolve_output_path(Some(missing_root.to_str().unwrap()), "pkg"),
            Err(OutputPathError::ParentMissing)
        ));
    }
}
