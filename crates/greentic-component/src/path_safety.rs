use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Normalize a user-supplied path and ensure it stays within an allowed root.
/// Reject absolute paths and any that escape via `..`.
pub fn normalize_under_root(root: &Path, candidate: &Path) -> Result<PathBuf> {
    if candidate.is_absolute() {
        anyhow::bail!("absolute paths are not allowed: {}", candidate.display());
    }

    let root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize root {}", root.display()))?;

    // Join and canonicalize. If the full path does not exist yet (e.g., we are
    // about to create it), canonicalize the nearest existing ancestor instead.
    let joined = root.join(candidate);
    let canon =
        match joined.canonicalize() {
            Ok(path) => path,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                let mut missing: Vec<PathBuf> = Vec::new();
                let mut ancestor = joined.as_path();
                loop {
                    if ancestor.try_exists().unwrap_or(false) {
                        break;
                    }
                    let parent = ancestor
                        .parent()
                        .with_context(|| format!("{} has no parent", ancestor.display()))?;
                    missing.push(ancestor.file_name().map(PathBuf::from).with_context(|| {
                        format!("{} missing final component", ancestor.display())
                    })?);
                    ancestor = parent;
                }

                let mut rebuilt = ancestor
                    .canonicalize()
                    .with_context(|| format!("failed to canonicalize {}", ancestor.display()))?;
                while let Some(component) = missing.pop() {
                    rebuilt.push(component);
                }
                rebuilt
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to canonicalize {}", joined.display()));
            }
        };

    // Ensure the canonical path is still under root
    if !canon.starts_with(&root) {
        anyhow::bail!(
            "path escapes root ({}): {}",
            root.display(),
            canon.display()
        );
    }

    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rejects_absolute_paths() {
        let root = tempfile::tempdir().expect("root tempdir");
        let err = normalize_under_root(root.path(), Path::new("/tmp/escape.txt"))
            .expect_err("absolute paths must be rejected");
        assert!(err.to_string().contains("absolute paths are not allowed"));
    }

    #[test]
    fn preserves_missing_leaf_paths_under_root() {
        let root = tempfile::tempdir().expect("root tempdir");
        let nested = root.path().join("safe");
        fs::create_dir_all(&nested).expect("create nested dir");

        let resolved = normalize_under_root(root.path(), Path::new("safe/new/file.txt"))
            .expect("missing leaf path inside root should be allowed");

        assert_eq!(
            resolved,
            nested
                .canonicalize()
                .expect("canonical nested directory")
                .join("new/file.txt")
        );
    }

    #[test]
    fn rejects_missing_path_that_escapes_via_symlinked_ancestor() {
        let root = tempfile::tempdir().expect("root tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let link = root.path().join("link-out");

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).expect("create symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), &link).expect("create symlink");

        let err = normalize_under_root(root.path(), Path::new("link-out/new/file.txt"))
            .expect_err("symlinked ancestor escape must be rejected");

        assert!(err.to_string().contains("path escapes root"));
    }

    #[test]
    fn rejects_parent_directory_escape_even_when_leaf_is_missing() {
        let root = tempfile::tempdir().expect("root tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let candidate = outside
            .path()
            .strip_prefix(outside.path().parent().expect("outside parent"))
            .expect("relative outside path");

        let err = normalize_under_root(root.path(), Path::new("../").join(candidate).as_path())
            .expect_err("parent escape must be rejected");

        assert!(err.to_string().contains("path escapes root"));
    }
}
