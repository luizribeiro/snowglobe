use crate::error::{IoError, Result};
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
    hostname_from(match SystemType::detect_current() {
        SystemType::NixOS => fs::read_to_string("/etc/hostname").map_err(|e| {
            std::io::Error::new(e.kind(), format!("Failed to read /etc/hostname: {e}"))
        }),
        SystemType::Darwin => hostname_command_output(),
    })
}

fn hostname_from(output: std::io::Result<String>) -> Result<String> {
    let hostname = output.and_then(|output| match output.trim() {
        "" => Err(std::io::Error::other("The hostname is empty")),
        hostname => Ok(hostname.to_string()),
    });
    hostname.map_err(|source| IoError::Hostname { source }.into())
}

fn hostname_command_output() -> std::io::Result<String> {
    let output = Command::new("hostname")
        .arg("-s")
        .output()
        .map_err(|e| std::io::Error::new(e.kind(), format!("Failed to run `hostname -s`: {e}")))?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "`hostname -s` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SnowError;

    fn cause(error: &SnowError) -> String {
        std::error::Error::source(error).unwrap().to_string()
    }

    #[test]
    fn test_hostname_from_trims_the_output() {
        assert_eq!(hostname_from(Ok("raven\n".to_string())).unwrap(), "raven");
    }

    #[test]
    fn test_empty_hostname_is_a_hostname_error() {
        let error = hostname_from(Ok("  \n".to_string())).unwrap_err();

        assert_eq!(error.to_string(), "Could not tell this machine's hostname");
        assert_eq!(cause(&error), "The hostname is empty");
    }

    #[test]
    fn test_unreadable_hostname_keeps_the_cause() {
        let unreadable = std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Failed to read /etc/hostname: No such file or directory",
        );

        let error = hostname_from(Err(unreadable)).unwrap_err();

        assert!(matches!(error, SnowError::Io(IoError::Hostname { .. })));
        assert_eq!(
            cause(&error),
            "Failed to read /etc/hostname: No such file or directory"
        );
    }

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
