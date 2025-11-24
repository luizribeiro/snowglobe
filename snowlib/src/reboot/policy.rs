use crate::error::Result;

/// Reboot policy for a host
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RebootPolicy {
    /// Never automatically reboot (use boot mode without reboot)
    Never,
    /// Always reboot after deployment (force reboot even without detection)
    Always,
    /// Automatically detect if reboot is needed (default)
    #[default]
    Auto,
}

impl std::str::FromStr for RebootPolicy {
    type Err = crate::error::SnowError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "never" => Ok(Self::Never),
            "always" => Ok(Self::Always),
            "auto" => Ok(Self::Auto),
            _ => Err(crate::error::NixError::EvalFailed {
                stderr: format!(
                    "Invalid reboot policy '{}'. Valid values are: 'auto', 'always', 'never'",
                    s
                ),
                source: std::io::Error::other("Invalid reboot policy"),
            }
            .into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_reboot_policy_from_str_valid() {
        assert!(matches!(
            RebootPolicy::from_str("auto").unwrap(),
            RebootPolicy::Auto
        ));
        assert!(matches!(
            RebootPolicy::from_str("never").unwrap(),
            RebootPolicy::Never
        ));
        assert!(matches!(
            RebootPolicy::from_str("always").unwrap(),
            RebootPolicy::Always
        ));
    }

    #[test]
    fn test_reboot_policy_from_str_invalid() {
        let result = RebootPolicy::from_str("nver");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Invalid reboot policy 'nver'"),
            "Error should mention invalid policy: {}",
            err_msg
        );
        assert!(
            err_msg.contains("'auto', 'always', 'never'"),
            "Error should list valid values: {}",
            err_msg
        );
    }
}
