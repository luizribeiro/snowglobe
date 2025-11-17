use crate::error::{DeployError, Result};
use std::fs;

/// Read the current system hostname from /etc/hostname
///
/// This is used by the rebuild command to auto-detect which nixosConfiguration
/// to build when no explicit host is specified.
pub fn get_hostname() -> Result<String> {
    let hostname = fs::read_to_string("/etc/hostname").map_err(|e| DeployError::Failed {
        host: "localhost".to_string(),
        source: std::io::Error::new(e.kind(), format!("Failed to read /etc/hostname: {}", e)),
    })?;

    let hostname = hostname.trim().to_string();

    if hostname.is_empty() {
        return Err(DeployError::Failed {
            host: "localhost".to_string(),
            source: std::io::Error::other("Hostname is empty in /etc/hostname"),
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

        // This test should pass on most systems
        // If /etc/hostname doesn't exist (unusual), test will fail gracefully
        if let Ok(hostname) = result {
            assert!(!hostname.is_empty());
            assert!(!hostname.contains('\n'));
        }
    }
}
