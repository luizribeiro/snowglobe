//! Flake context and discovery utilities

use crate::error::{FlakeError, IoError, Result};
use std::env;
use std::path::{Path, PathBuf};

/// Immutable context representing a Nix flake
///
/// Holds the flake root directory and provides methods for flake discovery
/// and validation.
#[derive(Debug, Clone)]
pub struct Flake {
    flake_root: PathBuf,
}

impl Flake {
    /// Create a new context from an optional flake path
    ///
    /// If flake_path is provided, validates and uses it. Otherwise searches
    /// up from the current directory to find flake.nix.
    pub fn new(flake_path: Option<&str>) -> Result<Self> {
        let flake_root = if let Some(path) = flake_path {
            validate_flake_path(path)?
        } else {
            find_flake_root()?
        };

        Ok(Self { flake_root })
    }

    /// Get the flake root directory
    pub fn flake_root(&self) -> &Path {
        &self.flake_root
    }

    /// Create a test context using the fixture directory
    #[cfg(test)]
    pub fn test_context() -> Self {
        let fixture_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        Self::new(Some(fixture_path.to_str().unwrap())).unwrap()
    }
}

/// Validate and resolve a flake path provided by the user
///
/// The path can be either:
/// - A directory containing flake.nix
/// - A direct path to flake.nix file
///
/// Supports shell expansions like ~ and environment variables.
fn validate_flake_path(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(path).map_err(|_| FlakeError::InvalidPath {
        path: PathBuf::from(path),
    })?;
    let path_buf = PathBuf::from(expanded.as_ref());

    if !path_buf.exists() {
        return Err(FlakeError::PathNotFound { path: path_buf }.into());
    }

    // If path points to flake.nix file, use its parent directory
    let flake_dir =
        if path_buf.is_file() && path_buf.file_name() == Some(std::ffi::OsStr::new("flake.nix")) {
            path_buf
                .parent()
                .ok_or_else(|| FlakeError::InvalidPath {
                    path: path_buf.clone(),
                })?
                .to_path_buf()
        } else if path_buf.is_dir() {
            // If it's a directory, verify flake.nix exists in it
            let flake_file = path_buf.join("flake.nix");
            if !flake_file.exists() {
                return Err(FlakeError::MissingInDir { path: path_buf }.into());
            }
            path_buf
        } else {
            return Err(FlakeError::InvalidPath { path: path_buf }.into());
        };

    Ok(flake_dir)
}

/// Find the flake root by searching up from the current directory
/// Returns the path to the directory containing flake.nix
fn find_flake_root() -> Result<PathBuf> {
    let current_dir = env::current_dir().map_err(|e| IoError::CurrentDir { source: e })?;

    find_flake_root_from(&current_dir)
}

/// Find the flake root by searching up from a given directory
fn find_flake_root_from(start: &Path) -> Result<PathBuf> {
    let mut current = start;

    loop {
        let flake_path = current.join("flake.nix");
        if flake_path.exists() {
            return Ok(current.to_path_buf());
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => {
                return Err(FlakeError::NotFound {
                    start_dir: start.to_path_buf(),
                }
                .into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_find_flake_root_in_current_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let result = find_flake_root_from(temp_dir.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_find_flake_root_in_parent_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let subdir = temp_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        let result = find_flake_root_from(&subdir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_find_flake_root_not_found() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = find_flake_root_from(temp_dir.path());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Could not find flake.nix"));
    }

    #[test]
    fn test_find_flake_root_multiple_levels() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let deep_dir = temp_dir.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep_dir).unwrap();

        let result = find_flake_root_from(&deep_dir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_flake_with_valid_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_file = temp_dir.path().join("flake.nix");
        fs::write(&flake_file, "# test flake").unwrap();

        let path_str = temp_dir.path().to_str().unwrap();
        let result = Flake::new(Some(path_str));

        assert!(result.is_ok());
        assert_eq!(result.unwrap().flake_root(), temp_dir.path());
    }

    #[test]
    fn test_flake_with_nonexistent_path() {
        let result = Flake::new(Some("/nonexistent/path"));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("does not exist"));
    }

    #[test]
    fn test_flake_with_path_missing_flake() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path_str = temp_dir.path().to_str().unwrap();

        let result = Flake::new(Some(path_str));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("No flake.nix found"));
    }

    #[test]
    fn test_flake_with_flake_nix_file_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_file = temp_dir.path().join("flake.nix");
        fs::write(&flake_file, "# test flake").unwrap();

        let file_path_str = flake_file.to_str().unwrap();
        let result = Flake::new(Some(file_path_str));

        assert!(result.is_ok());
        assert_eq!(result.unwrap().flake_root(), temp_dir.path());
    }
}
