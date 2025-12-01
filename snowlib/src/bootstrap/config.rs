//! Bootstrap configuration from NixOS module evaluation

use serde::Deserialize;

/// Bootstrap configuration for a host
///
/// This is evaluated from the NixOS module `snowglobe.bootstrap`:
/// ```nix
/// {
///   snowglobe.bootstrap = {
///     provisioner = "sd-image";
///     imageAttr = "config.system.build.sdImage";
///     sshKeyPath = "/etc/ssh";
///   };
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct BootstrapConfig {
    /// Provisioner to use for key injection (e.g., "manual", "sd-image")
    pub provisioner: Option<String>,
    /// Nix attribute for building the image (e.g., "config.system.build.sdImage")
    pub image_attr: Option<String>,
    /// Path where SSH keys should be injected (default: /etc/ssh)
    pub ssh_key_path: Option<String>,
    /// Root filesystem device path (e.g., "/dev/disk/by-label/nixos")
    pub root_fs_device: Option<String>,
}

impl BootstrapConfig {
    /// Get the provisioner name, defaulting to "manual" if not specified
    pub fn provisioner_name(&self) -> &str {
        self.provisioner.as_deref().unwrap_or("manual")
    }

    /// Get the SSH key path, defaulting to "/etc/ssh" if not specified
    pub fn ssh_key_path(&self) -> &str {
        self.ssh_key_path.as_deref().unwrap_or("/etc/ssh")
    }

    /// Check if this config has an image attribute for building
    pub fn has_image_attr(&self) -> bool {
        self.image_attr.is_some()
    }

    /// Extract the root filesystem label from the device path
    ///
    /// Parses paths like `/dev/disk/by-label/nixos` to extract `nixos`.
    /// Returns None if the device path doesn't use a label.
    pub fn root_fs_label(&self) -> Option<&str> {
        self.root_fs_device
            .as_deref()
            .and_then(|device| device.strip_prefix("/dev/disk/by-label/"))
    }
}

/// JSON structure for parsing bootstrap config from Nix evaluation
#[derive(Debug, Deserialize)]
pub(crate) struct BootstrapConfigJson {
    pub provisioner: Option<String>,
    #[serde(rename = "imageAttr")]
    pub image_attr: Option<String>,
    #[serde(rename = "sshKeyPath")]
    pub ssh_key_path: Option<String>,
    #[serde(rename = "rootFsDevice")]
    pub root_fs_device: Option<String>,
}

impl From<BootstrapConfigJson> for BootstrapConfig {
    fn from(json: BootstrapConfigJson) -> Self {
        Self {
            provisioner: json.provisioner.filter(|s| !s.is_empty()),
            image_attr: json.image_attr.filter(|s| !s.is_empty()),
            ssh_key_path: json.ssh_key_path.filter(|s| !s.is_empty()),
            root_fs_device: json.root_fs_device.filter(|s| !s.is_empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bootstrap_config_defaults() {
        let config = BootstrapConfig::default();
        assert_eq!(config.provisioner_name(), "manual");
        assert_eq!(config.ssh_key_path(), "/etc/ssh");
        assert!(!config.has_image_attr());
    }

    #[test]
    fn test_bootstrap_config_with_values() {
        let config = BootstrapConfig {
            provisioner: Some("sd-image".to_string()),
            image_attr: Some("config.system.build.sdImage".to_string()),
            ssh_key_path: Some("/custom/ssh".to_string()),
            root_fs_device: Some("/dev/disk/by-label/NIXOS_SD".to_string()),
        };
        assert_eq!(config.provisioner_name(), "sd-image");
        assert_eq!(config.ssh_key_path(), "/custom/ssh");
        assert!(config.has_image_attr());
        assert_eq!(config.root_fs_label(), Some("NIXOS_SD"));
    }

    #[test]
    fn test_bootstrap_config_json_parsing() {
        let json = r#"{
            "provisioner": "sd-image",
            "imageAttr": "config.system.build.sdImage",
            "sshKeyPath": "/etc/ssh"
        }"#;

        let json_config: BootstrapConfigJson = serde_json::from_str(json).unwrap();
        let config: BootstrapConfig = json_config.into();

        assert_eq!(config.provisioner_name(), "sd-image");
        assert_eq!(
            config.image_attr,
            Some("config.system.build.sdImage".to_string())
        );
        assert_eq!(config.ssh_key_path(), "/etc/ssh");
    }

    #[test]
    fn test_bootstrap_config_json_with_nulls() {
        let json = r#"{
            "provisioner": null,
            "imageAttr": null,
            "sshKeyPath": null
        }"#;

        let json_config: BootstrapConfigJson = serde_json::from_str(json).unwrap();
        let config: BootstrapConfig = json_config.into();

        assert_eq!(config.provisioner_name(), "manual");
        assert_eq!(config.ssh_key_path(), "/etc/ssh");
        assert!(!config.has_image_attr());
    }

    #[test]
    fn test_bootstrap_config_json_partial() {
        let json = r#"{
            "provisioner": "raw"
        }"#;

        let json_config: BootstrapConfigJson = serde_json::from_str(json).unwrap();
        let config: BootstrapConfig = json_config.into();

        assert_eq!(config.provisioner_name(), "raw");
        assert_eq!(config.ssh_key_path(), "/etc/ssh");
        assert!(!config.has_image_attr());
    }

    #[test]
    fn test_bootstrap_config_json_with_empty_strings() {
        let json = r#"{
            "provisioner": "",
            "imageAttr": "",
            "sshKeyPath": ""
        }"#;

        let json_config: BootstrapConfigJson = serde_json::from_str(json).unwrap();
        let config: BootstrapConfig = json_config.into();

        // Empty strings should be treated as None, falling back to defaults
        assert_eq!(config.provisioner_name(), "manual");
        assert_eq!(config.ssh_key_path(), "/etc/ssh");
        assert!(!config.has_image_attr());
    }

    #[test]
    fn test_root_fs_label_extracts_label_from_device_path() {
        let config = BootstrapConfig {
            root_fs_device: Some("/dev/disk/by-label/nixos".to_string()),
            ..Default::default()
        };
        assert_eq!(config.root_fs_label(), Some("nixos"));

        let config = BootstrapConfig {
            root_fs_device: Some("/dev/disk/by-label/NIXOS_SD".to_string()),
            ..Default::default()
        };
        assert_eq!(config.root_fs_label(), Some("NIXOS_SD"));
    }

    #[test]
    fn test_root_fs_label_returns_none_for_non_label_paths() {
        // UUID-based path
        let config = BootstrapConfig {
            root_fs_device: Some("/dev/disk/by-uuid/abc-123".to_string()),
            ..Default::default()
        };
        assert_eq!(config.root_fs_label(), None);

        // Direct device path
        let config = BootstrapConfig {
            root_fs_device: Some("/dev/sda2".to_string()),
            ..Default::default()
        };
        assert_eq!(config.root_fs_label(), None);

        // None
        let config = BootstrapConfig::default();
        assert_eq!(config.root_fs_label(), None);
    }
}
