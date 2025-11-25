//! Common deployment primitives shared between local and SSH backends

use crate::nix::ActivationMode;

pub fn activation_mode_to_string(mode: ActivationMode) -> &'static str {
    match mode {
        ActivationMode::Switch => "switch",
        ActivationMode::Boot => "boot",
        ActivationMode::Test => "test",
        ActivationMode::DryActivate => "dry-activate",
    }
}

/// Profile generations are required for rollbacks and appear in nixos-rebuild list-generations.
/// Ephemeral modes (Test/DryActivate) intentionally skip this to avoid polluting the generation list.
pub fn should_update_profile(mode: ActivationMode) -> bool {
    !matches!(mode, ActivationMode::Test | ActivationMode::DryActivate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activation_mode_to_string() {
        assert_eq!(activation_mode_to_string(ActivationMode::Switch), "switch");
        assert_eq!(activation_mode_to_string(ActivationMode::Boot), "boot");
        assert_eq!(activation_mode_to_string(ActivationMode::Test), "test");
        assert_eq!(
            activation_mode_to_string(ActivationMode::DryActivate),
            "dry-activate"
        );
    }

    #[test]
    fn test_should_update_profile_for_persistent_modes() {
        assert!(should_update_profile(ActivationMode::Switch));
        assert!(should_update_profile(ActivationMode::Boot));
    }

    #[test]
    fn test_should_not_update_profile_for_ephemeral_modes() {
        assert!(!should_update_profile(ActivationMode::Test));
        assert!(!should_update_profile(ActivationMode::DryActivate));
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::super::DeployTarget;
    use crate::nix::StorePath;

    pub(crate) fn create_test_target(hostname: &str) -> DeployTarget {
        DeployTarget {
            user: None,
            hostname: hostname.to_string(),
            port: None,
        }
    }

    pub(crate) fn create_test_target_with_port(hostname: &str, port: Option<u16>) -> DeployTarget {
        DeployTarget {
            user: None,
            hostname: hostname.to_string(),
            port,
        }
    }

    pub(crate) fn create_test_store_path(path: &str) -> StorePath {
        StorePath::new(path.to_string())
    }
}
