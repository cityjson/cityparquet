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
//! one to exist. Both have to be handled, and the order matters: the
//! caller's path is first normalised lexically — `CurDir` is dropped,
//! `ParentDir` pops, and a pop that would rise above the path's own root is
//! refused outright — and only then joined to the canonical root, resolved
//! past any symlinks in the prefix that exists, and compared against the
//! root component-wise.
//!
//! Normalising first is what makes the comparison trustworthy. Doing it the
//! other way round — canonicalising and then folding `..` onto the result —
//! lets a pop land back on a component the walk has already passed, so a
//! symlink out of the root that is refused when named directly
//! (`escape/pkg`) is accepted when named through a pop
//! (`nonexistent/../escape/pkg`). Canonicalisation has to have the last
//! word, which means no `..` may survive into what it resolves.
//!
//! A consequence worth naming: `escape/../x` resolves to `<root>/x`, not to
//! the link target's parent as a shell would read it. That is the safe
//! reading, and the honest one — this function's answer *is* the path the
//! caller writes, so a component popped lexically is never traversed.
//!
//! This is a check-then-use control, not an atomic one. It approves a path,
//! not the state of the tree at the moment of the write: a symlink planted
//! or swapped in inside the root after this function returns — including at
//! a component that did not exist when the path was checked — is not
//! caught. That gap is accepted residual risk, not fixed here.

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
    /// resolving the requested path could not be canonicalised — it is a
    /// broken symlink, or it was removed or became unreadable between being
    /// found and being resolved.
    #[error(
        "the configured output root, or an ancestor of the requested path, could not be resolved"
    )]
    ParentMissing,
}

/// Resolve `requested` against `root`, refusing anything that would write
/// outside it.
///
/// `root` is `None` when `CITYLAKE_OUTPUT_ROOT` is unset — a legitimate
/// state, refused rather than defaulted, because a control that is on only
/// when configured is not a control. `requested` must be a relative path;
/// it is normalised lexically, joined onto the canonicalised root, resolved
/// past any symlinks in the prefix that already exists, and checked to still
/// sit under the root.
pub fn resolve_output_path(
    root: Option<&str>,
    requested: &str,
) -> Result<PathBuf, OutputPathError> {
    let root = root.ok_or(OutputPathError::RootNotConfigured)?;

    let requested_path = Path::new(requested);
    if requested_path.is_absolute() {
        return Err(OutputPathError::Absolute);
    }

    // Before the filesystem is touched at all, so that nothing the
    // canonicalisation below resolves can be undone by a `..` afterwards.
    let normalised = normalise(requested_path)?;

    let canonical_root = std::fs::canonicalize(root).map_err(|_| OutputPathError::ParentMissing)?;

    let full = canonical_root.join(&normalised);

    // The deepest ancestor of `full` that actually exists. `full`'s
    // components may include parts that do not exist yet — that is the
    // normal case for a package about to be created — so this walk finds
    // where resolution has to stop; at worst it stops at `canonical_root`
    // itself, which is guaranteed to exist.
    //
    // `symlink_metadata`, not `exists`: `exists` follows the final symlink
    // and reports a *broken* one as not existing at all, so a dangling link
    // would fall into the not-yet-existing remainder below instead of being
    // canonicalised — and the remainder is re-attached by name, never
    // resolved. `symlink_metadata` sees the link entry itself regardless of
    // what it points to (or whether that target exists), so the walk stops
    // on the link and the canonicalisation below — which does follow it —
    // is what refuses it.
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

    // A plain join, not a lexical fold: `normalise` left `full` free of
    // `.` and `..`, so `remainder` carries nothing but `Normal` components
    // and there is nothing left to resolve.
    let resolved = canonical_ancestor.join(remainder);

    // Component-wise, never a string prefix: `<tmp>/root-backup` starts with
    // the string `<tmp>/root` while sharing no component boundary with it.
    if resolved.starts_with(&canonical_root) {
        Ok(resolved)
    } else {
        Err(OutputPathError::Escapes)
    }
}

/// Resolve `requested`'s own `.` and `..` components, refusing anything that
/// climbs above it.
///
/// `std::path` has no normalising method and [`std::fs::canonicalize`]
/// cannot stand in for one, because the parts of an output path that do not
/// exist yet — the normal case for a package about to be written — are
/// exactly where a `..` hides from it. So the resolution is lexical, and it
/// happens before the filesystem is consulted: what comes back is relative
/// and contains only `Normal` components, which is the precondition the
/// canonicalisation in [`resolve_output_path`] relies on to have the final
/// say.
///
/// A `..` that would rise above the requested path's own root is
/// [`OutputPathError::Escapes`] rather than a silently clamped no-op: the
/// caller asked for somewhere outside the root and should be told so.
fn normalise(requested: &Path) -> Result<PathBuf, OutputPathError> {
    let mut components: Vec<Component> = Vec::new();

    for component in requested.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if components.pop().is_none() {
                    return Err(OutputPathError::Escapes);
                }
            }
            Component::Normal(_) => components.push(component),
            // `requested` is not absolute — checked by the caller — but a
            // Windows path may still carry a bare drive prefix (`C:pkg`)
            // that is relative and names another volume's cwd. Neither
            // belongs under the root.
            Component::RootDir | Component::Prefix(_) => return Err(OutputPathError::Escapes),
        }
    }

    Ok(components.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A root with symlinks pointing out of it — the case a textual check
    /// passes. `escape` leads to a directory whose name shares nothing with
    /// the root's; `backup` leads to a sibling named `root-backup`, whose
    /// path string-prefix-matches the root's while differing from it
    /// component-wise.
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(dir.path().join("outside")).unwrap();
        fs::create_dir_all(dir.path().join("root-backup")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("outside"), root.join("escape")).unwrap();
            std::os::unix::fs::symlink(dir.path().join("root-backup"), root.join("backup"))
                .unwrap();
        }
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
        // `newdir` does not exist, so canonicalisation never sees the `..`
        // that follows it — the filesystem cannot resolve a path through a
        // component that is not there. Only the lexical normalisation
        // catches this.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "newdir/../../outside/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_pop_onto_a_symlink_is_refused() {
        // The composition of the two tests above, and the reason
        // normalisation runs *before* the ancestor walk rather than after
        // it: `nonexistent` does not exist, so a walk over the literal path
        // stops at the root and never sees `escape` at all. Normalising
        // first reduces this to `escape/pkg`, which the walk then
        // canonicalises and refuses.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "nonexistent/../escape/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_pop_past_a_symlink_never_traverses_it() {
        // `escape/../x` is accepted, and resolves to `<root>/x` rather than
        // to the symlink's target. Not a weakening: `resolve_output_path`'s
        // answer is the path the caller writes, so popping `escape` off
        // lexically means the link is never followed — nothing outside the
        // root is reached. The alternative reading, where `..` traverses the
        // link the way a shell would, is the one that escapes.
        let (_dir, root) = fixture();
        let got = resolve_output_path(Some(root.to_str().unwrap()), "escape/../x").unwrap();
        assert_eq!(got, fs::canonicalize(&root).unwrap().join("x"));
    }

    #[test]
    fn a_traversal_to_a_sibling_sharing_a_name_prefix_is_refused() {
        // "root-backup" is not "root" as a path component, even though it
        // shares a string prefix with it. Refused during normalisation,
        // before the filesystem is consulted at all: the leading `..` pops
        // above the requested path's own root.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "../root-backup/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_symlink_to_a_sibling_sharing_a_name_prefix_is_refused() {
        // The same sibling, reached the way normalisation cannot see: a
        // symlink inside the root. This is what pins the final comparison to
        // the component-wise `Path::starts_with` — the resolved
        // `<tmp>/root-backup/pkg` *does* string-prefix-match `<tmp>/root`,
        // so a naive string compare would accept it, and every other test in
        // this module would still pass.
        let (_dir, root) = fixture();
        assert!(matches!(
            resolve_output_path(Some(root.to_str().unwrap()), "backup/pkg"),
            Err(OutputPathError::Escapes)
        ));
    }

    #[test]
    fn a_dangling_symlink_inside_the_root_is_refused() {
        // `Path::exists()` follows symlinks and reports a *broken* one as
        // not existing at all — no race required, this is wrong on its own:
        // the walk would step straight past the link and treat it as part
        // of the not-yet-created remainder, never asking where it points.
        // `symlink_metadata` sees the link entry itself regardless of
        // whether its target exists, so the walk stops there and
        // canonicalising it (which does follow the link) fails, refusing
        // the request instead of silently approving it.
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
