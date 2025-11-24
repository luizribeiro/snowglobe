/// System type for a host configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemType {
    /// NixOS configuration
    NixOS,
    /// nix-darwin (macOS) configuration
    Darwin,
}

impl SystemType {
    /// Detect the current system type
    pub fn detect_current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Darwin
        } else {
            Self::NixOS
        }
    }

    /// Get the flake attribute name for this system type (nixosConfigurations or darwinConfigurations)
    pub fn flake_attr(&self) -> &'static str {
        match self {
            Self::NixOS => "nixosConfigurations",
            Self::Darwin => "darwinConfigurations",
        }
    }

    /// Get the system build path suffix for this system type
    pub fn build_path_suffix(&self) -> &'static str {
        match self {
            Self::NixOS => "config.system.build.toplevel",
            Self::Darwin => "config.system.build.toplevel",
        }
    }
}

/// Nix store path for a built configuration
#[derive(Debug, Clone)]
pub struct StorePath(String);

impl StorePath {
    pub fn new(path: String) -> Self {
        Self(path)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Activation mode for NixOS configuration
///
/// Maps directly to switch-to-configuration subcommands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivationMode {
    /// Activate and make default (switch)
    Switch,
    /// Only set for next boot
    Boot,
    /// Test activation without making permanent
    Test,
    /// Dry run - show what would change
    DryActivate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn test_detect_current_on_darwin() {
        assert_eq!(SystemType::detect_current(), SystemType::Darwin);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_detect_current_on_nixos() {
        assert_eq!(SystemType::detect_current(), SystemType::NixOS);
    }

    #[test]
    fn test_flake_attr_nixos() {
        assert_eq!(SystemType::NixOS.flake_attr(), "nixosConfigurations");
    }

    #[test]
    fn test_flake_attr_darwin() {
        assert_eq!(SystemType::Darwin.flake_attr(), "darwinConfigurations");
    }

    #[test]
    fn test_build_path_suffix_nixos() {
        assert_eq!(
            SystemType::NixOS.build_path_suffix(),
            "config.system.build.toplevel"
        );
    }

    #[test]
    fn test_build_path_suffix_darwin() {
        assert_eq!(
            SystemType::Darwin.build_path_suffix(),
            "config.system.build.toplevel"
        );
    }

    #[test]
    fn test_system_type_equality() {
        assert_eq!(SystemType::NixOS, SystemType::NixOS);
        assert_eq!(SystemType::Darwin, SystemType::Darwin);
        assert_ne!(SystemType::NixOS, SystemType::Darwin);
    }
}
