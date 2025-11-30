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
}

/// JSON structure for parsing bootstrap config from Nix evaluation
#[derive(Debug, Deserialize)]
pub(crate) struct BootstrapConfigJson {
    pub provisioner: Option<String>,
    #[serde(rename = "imageAttr")]
    pub image_attr: Option<String>,
    #[serde(rename = "sshKeyPath")]
    pub ssh_key_path: Option<String>,
}

impl From<BootstrapConfigJson> for BootstrapConfig {
    fn from(json: BootstrapConfigJson) -> Self {
        Self {
            provisioner: json.provisioner.filter(|s| !s.is_empty()),
            image_attr: json.image_attr.filter(|s| !s.is_empty()),
            ssh_key_path: json.ssh_key_path.filter(|s| !s.is_empty()),
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
        };
        assert_eq!(config.provisioner_name(), "sd-image");
        assert_eq!(config.ssh_key_path(), "/custom/ssh");
        assert!(config.has_image_attr());
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
}
