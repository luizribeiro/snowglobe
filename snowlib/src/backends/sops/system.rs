//! Real implementation of SOPS backend using the sops command-line tool

use super::SopsBackend;
use crate::error::{BootstrapError, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Real implementation of SopsBackend using the sops CLI
pub struct SystemSopsBackend;

impl SystemSopsBackend {
    pub fn new() -> Self {
        Self
    }

    fn which(cmd: &str) -> bool {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn sops_yaml_path(flake_dir: &Path) -> std::path::PathBuf {
        flake_dir.join(".sops.yaml")
    }
}

impl Default for SystemSopsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SopsBackend for SystemSopsBackend {
    fn check_dependencies(&self) -> Result<()> {
        if !Self::which("sops") {
            return Err(BootstrapError::SopsMissing.into());
        }
        Ok(())
    }

    fn sops_yaml_exists(&self, flake_dir: &Path) -> bool {
        Self::sops_yaml_path(flake_dir).exists()
    }

    fn host_exists(&self, flake_dir: &Path, hostname: &str) -> Result<bool> {
        let sops_yaml = Self::sops_yaml_path(flake_dir);
        let content =
            fs::read_to_string(&sops_yaml).map_err(|e| BootstrapError::SopsYamlReadFailed {
                path: sops_yaml.display().to_string(),
                source: e,
            })?;

        // Look for an anchor definition with this hostname
        // Format: `- &hostname ` (note the space after)
        let anchor_pattern = format!("- &{} ", hostname);
        Ok(content.contains(&anchor_pattern))
    }

    fn add_host_key(&self, flake_dir: &Path, hostname: &str, age_key: &str) -> Result<()> {
        let sops_yaml = Self::sops_yaml_path(flake_dir);
        let content =
            fs::read_to_string(&sops_yaml).map_err(|e| BootstrapError::SopsYamlReadFailed {
                path: sops_yaml.display().to_string(),
                source: e,
            })?;

        let new_content = insert_age_key(&content, hostname, age_key)?;

        fs::write(&sops_yaml, new_content).map_err(|e| BootstrapError::SopsYamlWriteFailed {
            path: sops_yaml.display().to_string(),
            source: e,
        })?;

        Ok(())
    }

    fn update_keys(&self, flake_dir: &Path) -> Result<usize> {
        // Find all .yaml files in secrets/ directories that might need updating
        let secrets_dir = flake_dir.join("secrets");
        if !secrets_dir.exists() {
            // No secrets directory is not an error - many setups don't use
            // a secrets/ directory or store secrets elsewhere
            return Ok(0);
        }

        let mut updated = 0;
        for entry in walkdir(&secrets_dir) {
            if entry
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                // Run sops updatekeys on each file
                // --yes to auto-confirm
                let output = Command::new("sops")
                    .arg("updatekeys")
                    .arg("--yes")
                    .arg(&entry)
                    .current_dir(flake_dir)
                    .output();

                match output {
                    Ok(out) if out.status.success() => {
                        updated += 1;
                    }
                    Ok(out) => {
                        // Log warning for failed files but continue
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        tracing::warn!(
                            "sops updatekeys failed for {}: {}",
                            entry.display(),
                            stderr.trim()
                        );
                    }
                    Err(e) => {
                        // Log warning for execution errors but continue
                        tracing::warn!(
                            "Failed to run sops updatekeys for {}: {}",
                            entry.display(),
                            e
                        );
                    }
                }
            }
        }

        Ok(updated)
    }
}

/// Walk a directory recursively to find files
fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        let path = entry.path();
                        if path.is_dir() {
                            files.extend(walkdir(&path));
                        } else if path.is_file() {
                            files.push(path);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Skipping entry in {}: {}", dir.display(), e);
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Could not read directory {}: {}", dir.display(), e);
        }
    }
    files
}

/// Insert an age key into the keys section of .sops.yaml
///
/// This function uses a line-based approach to preserve YAML anchors and formatting.
/// It finds the last key in the keys section and inserts the new key after it.
fn insert_age_key(content: &str, hostname: &str, age_key: &str) -> Result<String> {
    let lines: Vec<&str> = content.lines().collect();

    // Find the keys: section
    let keys_line_idx = lines
        .iter()
        .position(|line| line.trim() == "keys:")
        .ok_or_else(|| BootstrapError::SopsYamlInvalid {
            reason: "No 'keys:' section found".to_string(),
        })?;

    // Find where to insert the new key (before creation_rules: or at end of keys section)
    let mut insert_idx = None;
    for (idx, line) in lines.iter().enumerate().skip(keys_line_idx + 1) {
        let trimmed = line.trim();

        // If we hit creation_rules: or another top-level key, insert before it
        if !trimmed.is_empty() && !trimmed.starts_with('-') && !trimmed.starts_with('#') {
            insert_idx = Some(idx);
            break;
        }

        // Track the last key line (starts with `  -`)
        if trimmed.starts_with('-') {
            insert_idx = Some(idx + 1);
        }
    }

    let insert_idx = insert_idx.ok_or_else(|| BootstrapError::SopsYamlInvalid {
        reason: "Could not find insertion point for new key".to_string(),
    })?;

    // Determine the indentation from nearby lines
    let indent = if insert_idx > 0 {
        let prev_line = lines[insert_idx - 1];
        let stripped = prev_line.trim_start();
        if stripped.starts_with('-') {
            // Use the same indentation as the previous key
            &prev_line[..prev_line.len() - stripped.len()]
        } else {
            "  " // Default to 2 spaces
        }
    } else {
        "  "
    };

    // Build the new key line
    let new_key_line = format!("{}- &{} {}", indent, hostname, age_key);

    // Reconstruct the content
    let mut result = Vec::with_capacity(lines.len() + 1);
    result.extend_from_slice(&lines[..insert_idx]);
    result.push(&new_key_line);
    result.extend_from_slice(&lines[insert_idx..]);

    // Join with newlines, preserving original line endings
    let mut output = result.join("\n");

    // Preserve trailing newline if original had one
    if content.ends_with('\n') {
        output.push('\n');
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_backend() {
        let backend = SystemSopsBackend::new();
        assert!(std::mem::size_of_val(&backend) == 0);
    }

    #[test]
    fn test_insert_age_key_simple() {
        let content = r#"---
keys:
  - &luiz 97A0AE5E03F3499B7D7A65C676A4143237EF5817
  - &oxygen age1gdwx7nfe6gzzu7gxm95adt3kqvwlzkmgryrvh46sz646ns0g2yaqql56hd
creation_rules:
  - path_regex: secrets/.*
"#;

        let result = insert_age_key(
            content,
            "carbon",
            "age1newkey0000000000000000000000000000000000000000000000000",
        )
        .unwrap();

        assert!(result.contains("- &carbon age1newkey"));
        assert!(result.contains("- &oxygen age1gdwx7nfe"));
        // Ensure carbon comes after oxygen but before creation_rules
        let carbon_pos = result.find("- &carbon").unwrap();
        let oxygen_pos = result.find("- &oxygen").unwrap();
        let creation_pos = result.find("creation_rules:").unwrap();
        assert!(carbon_pos > oxygen_pos);
        assert!(carbon_pos < creation_pos);
    }

    #[test]
    fn test_insert_age_key_preserves_comments() {
        let content = r#"keys:
  # GPG keys
  - &luiz 97A0AE5E03F3499B7D7A65C676A4143237EF5817
  # age keys
  - &oxygen age1gdwx7nfe6gzzu7gxm95adt3kqvwlzkmgryrvh46sz646ns0g2yaqql56hd
creation_rules:
"#;

        let result = insert_age_key(
            content,
            "helium",
            "age1helium000000000000000000000000000000000000000000000000",
        )
        .unwrap();

        assert!(result.contains("# GPG keys"));
        assert!(result.contains("# age keys"));
        assert!(result.contains("- &helium age1helium"));
    }

    #[test]
    fn test_insert_age_key_no_keys_section_fails() {
        let content = r#"---
creation_rules:
  - path_regex: secrets/.*
"#;

        let result = insert_age_key(content, "carbon", "age1key");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No 'keys:' section"));
    }

    #[test]
    fn test_insert_age_key_preserves_trailing_newline() {
        let content = "keys:\n  - &foo age1foo\ncreation_rules:\n";
        let result = insert_age_key(content, "bar", "age1bar").unwrap();
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_insert_age_key_no_trailing_newline() {
        let content = "keys:\n  - &foo age1foo\ncreation_rules:";
        let result = insert_age_key(content, "bar", "age1bar").unwrap();
        assert!(!result.ends_with('\n'));
    }

    #[test]
    fn test_insert_age_key_empty_keys_section() {
        // Keys section exists but has no entries yet - new key is first
        let content = r#"keys:
creation_rules:
  - path_regex: secrets/.*
"#;
        let result = insert_age_key(content, "carbon", "age1carbon").unwrap();
        assert!(result.contains("- &carbon age1carbon"));
        // Key should be inserted after keys: but before creation_rules:
        let keys_pos = result.find("keys:").unwrap();
        let carbon_pos = result.find("- &carbon").unwrap();
        let creation_pos = result.find("creation_rules:").unwrap();
        assert!(carbon_pos > keys_pos);
        assert!(carbon_pos < creation_pos);
    }

    #[test]
    fn test_insert_age_key_at_end_of_file() {
        // Keys section at end of file with no creation_rules following
        let content = r#"---
keys:
  - &oxygen age1oxygen0000000000000000000000000000000000000000000000000
"#;

        let result = insert_age_key(content, "carbon", "age1carbon").unwrap();
        assert!(result.contains("- &carbon age1carbon"));
        assert!(result.contains("- &oxygen age1oxygen"));
        // Carbon should come after oxygen
        let carbon_pos = result.find("- &carbon").unwrap();
        let oxygen_pos = result.find("- &oxygen").unwrap();
        assert!(carbon_pos > oxygen_pos);
    }

    #[test]
    fn test_insert_age_key_multiple_sections() {
        let content = r#"keys:
  - &foo age1foo
creation_rules:
  - path_regex: secrets/.*
other_section:
  key: value
"#;

        let result = insert_age_key(content, "bar", "age1bar").unwrap();
        assert!(result.contains("- &bar age1bar"));
        // New key should be in keys section, not after other_section
        let bar_pos = result.find("- &bar").unwrap();
        let creation_pos = result.find("creation_rules:").unwrap();
        assert!(bar_pos < creation_pos);
    }

    #[test]
    fn test_walkdir_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let files = walkdir(temp_dir.path());
        assert!(files.is_empty());
    }

    #[test]
    fn test_walkdir_with_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        fs::write(temp_dir.path().join("test.yaml"), "key: value").unwrap();
        fs::create_dir(temp_dir.path().join("subdir")).unwrap();
        fs::write(temp_dir.path().join("subdir/nested.yaml"), "other: val").unwrap();

        let files = walkdir(temp_dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_sops_yaml_path() {
        let path = SystemSopsBackend::sops_yaml_path(Path::new("/home/user/ops"));
        assert_eq!(path, std::path::PathBuf::from("/home/user/ops/.sops.yaml"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_sops_yaml_exists_returns_false_when_missing() {
        let temp_dir = TempDir::new().unwrap();
        let backend = SystemSopsBackend::new();
        assert!(!backend.sops_yaml_exists(temp_dir.path()));
    }

    #[test]
    fn test_sops_yaml_exists_returns_true_when_present() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join(".sops.yaml"), "keys:\n").unwrap();

        let backend = SystemSopsBackend::new();
        assert!(backend.sops_yaml_exists(temp_dir.path()));
    }

    #[test]
    fn test_host_exists_returns_false_for_new_host() {
        let temp_dir = TempDir::new().unwrap();
        let content = r#"keys:
  - &oxygen age1gdwx7nfe6gzzu7gxm95adt3kqvwlzkmgryrvh46sz646ns0g2yaqql56hd
"#;
        fs::write(temp_dir.path().join(".sops.yaml"), content).unwrap();

        let backend = SystemSopsBackend::new();
        assert!(!backend.host_exists(temp_dir.path(), "carbon").unwrap());
    }

    #[test]
    fn test_host_exists_returns_true_for_existing_host() {
        let temp_dir = TempDir::new().unwrap();
        let content = r#"keys:
  - &oxygen age1gdwx7nfe6gzzu7gxm95adt3kqvwlzkmgryrvh46sz646ns0g2yaqql56hd
  - &carbon age1carbon00000000000000000000000000000000000000000000000000
"#;
        fs::write(temp_dir.path().join(".sops.yaml"), content).unwrap();

        let backend = SystemSopsBackend::new();
        assert!(backend.host_exists(temp_dir.path(), "carbon").unwrap());
    }

    #[test]
    fn test_add_host_key_creates_entry() {
        let temp_dir = TempDir::new().unwrap();
        let content = r#"keys:
  - &oxygen age1gdwx7nfe6gzzu7gxm95adt3kqvwlzkmgryrvh46sz646ns0g2yaqql56hd
creation_rules:
  - path_regex: secrets/.*
"#;
        fs::write(temp_dir.path().join(".sops.yaml"), content).unwrap();

        let backend = SystemSopsBackend::new();
        backend
            .add_host_key(
                temp_dir.path(),
                "carbon",
                "age1carbon00000000000000000000000000000000000000000000000000",
            )
            .unwrap();

        let result = fs::read_to_string(temp_dir.path().join(".sops.yaml")).unwrap();
        assert!(result.contains("- &carbon age1carbon"));
    }

    #[test]
    fn test_update_keys_with_no_secrets_dir() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join(".sops.yaml"), "keys:\n").unwrap();

        let backend = SystemSopsBackend::new();
        let updated = backend.update_keys(temp_dir.path()).unwrap();
        assert_eq!(updated, 0);
    }

    #[test]
    fn test_check_dependencies_does_not_panic() {
        let backend = SystemSopsBackend::new();
        // Should not panic, result depends on whether sops is installed
        let _ = backend.check_dependencies();
    }
}
