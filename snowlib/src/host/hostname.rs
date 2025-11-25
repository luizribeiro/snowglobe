use crate::error::{DeployError, Result};
use crate::nix::SystemType;
use std::fs;
use std::process::Command;

/// Read the current system hostname
///
/// On NixOS, reads from /etc/hostname.
/// On Darwin, uses the hostname command.
///
/// This is used by the rebuild command to auto-detect which configuration
/// to build when no explicit host is specified.
pub fn get_hostname() -> Result<String> {
    let system_type = SystemType::detect_current();

    let hostname = match system_type {
        SystemType::NixOS => {
            let content = fs::read_to_string("/etc/hostname").map_err(|e| DeployError::Failed {
                host: "localhost".to_string(),
                source: std::io::Error::new(
                    e.kind(),
                    format!("Failed to read /etc/hostname: {}", e),
                ),
            })?;
            content.trim().to_string()
        }
        SystemType::Darwin => {
            let output =
                Command::new("hostname")
                    .arg("-s")
                    .output()
                    .map_err(|e| DeployError::Failed {
                        host: "localhost".to_string(),
                        source: std::io::Error::new(
                            e.kind(),
                            format!("Failed to run hostname command: {}", e),
                        ),
                    })?;

            if !output.status.success() {
                return Err(DeployError::Failed {
                    host: "localhost".to_string(),
                    source: std::io::Error::other(format!(
                        "hostname command failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    )),
                }
                .into());
            }

            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };

    if hostname.is_empty() {
        return Err(DeployError::Failed {
            host: "localhost".to_string(),
            source: std::io::Error::other("Hostname is empty"),
        }
        .into());
    }

    Ok(hostname)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_hostname() {
        let result = get_hostname();

        if let Ok(hostname) = result {
            assert!(!hostname.is_empty());
            assert!(!hostname.contains('\n'));
            assert!(!hostname.contains('\r'));
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_get_hostname_darwin() {
        let result = get_hostname();
        assert!(
            result.is_ok(),
            "hostname command should succeed on Darwin: {:?}",
            result
        );

        let hostname = result.unwrap();
        assert!(!hostname.is_empty(), "hostname should not be empty");
        assert!(
            !hostname.contains('\n'),
            "hostname should not contain newlines"
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_get_hostname_nixos() {
        let result = get_hostname();

        if let Ok(hostname) = result {
            assert!(!hostname.is_empty());
            assert!(!hostname.contains('\n'));
        }
    }
}
