use crate::plural::plural;
use clap::Args;
use snowlib::{
    get_provisioner, Flake, InjectionResult, KeyGenBackend, NixBackend, OutputMode,
    ProvisionerConfig, ProvisioningResult, Result, SopsBackend, SystemKeyGenBackend,
    SystemNixBackend, SystemSopsBackend, SystemType, TargetType,
};
use std::io::{self, Write};
use std::path::Path;

/// Bootstrap a new host with full automation
///
/// Generates cryptographic identity, builds a bootable image, and optionally
/// deploys it to a target. Requires the host to have snowglobe.bootstrap
/// configured in its NixOS configuration.
#[derive(Debug, Args)]
pub struct BootstrapArgs {
    /// Hostname to bootstrap
    pub hostname: String,

    /// Override the provisioner (ignores host config)
    #[arg(long)]
    pub provisioner: Option<String>,

    /// Update .sops.yaml with the new host's age key
    #[arg(long)]
    pub update_sops: bool,

    /// Deploy the provisioned image to a target
    ///
    /// Target types:
    ///   /dev/sdX         - Flash to block device (sd-image, raw)
    ///   proxmox-host     - Deploy to Proxmox host (proxmox-vma)
    #[arg(long)]
    pub target: Option<String>,

    /// Skip confirmation prompts for destructive operations
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Show Nix build output
    #[arg(long, short)]
    pub verbose: bool,

    #[cfg(debug_assertions)]
    #[arg(long, hide = true)]
    pub mock: bool,
}

impl BootstrapArgs {
    pub async fn run(self, flake: &Flake) -> Result<()> {
        let (keygen_backend, nix_backend, sops_backend) = create_backends(&self);

        let bootstrap_config = nix_backend
            .get_bootstrap_config(flake, &self.hostname, SystemType::NixOS)
            .await?;

        let image_attr = bootstrap_config.image_attr.as_ref().ok_or_else(|| {
            snowlib::ProvisionerError::NoImageAttr {
                host: self.hostname.clone(),
            }
        })?;

        let provisioner_name = self
            .provisioner
            .as_deref()
            .unwrap_or_else(|| bootstrap_config.provisioner_name());

        println!("Bootstrapping {} ({})...", self.hostname, provisioner_name);

        // Generate keys
        keygen_backend.check_dependencies()?;
        println!("  → Generating SSH host key...");
        let keys = snowlib::bootstrap::bootstrap_host(&self.hostname, keygen_backend.as_ref())?;
        println!("  ✓ Generated SSH host key");

        // Update SOPS before building so secrets are encrypted for the new host
        if self.update_sops {
            println!("  → Updating .sops.yaml...");
            let result = sops_backend.update_sops(
                flake.flake_root(),
                &self.hostname,
                &keys.age_public_key,
            )?;
            println!("  ✓ {}", sops_updated(result.files_updated));
        }

        // Build image
        println!("  → Building image...");
        let output_mode = if self.verbose {
            OutputMode::Inherit
        } else {
            OutputMode::Capture
        };
        let store_path = nix_backend
            .build_image(
                flake,
                &self.hostname,
                SystemType::NixOS,
                image_attr,
                output_mode,
            )
            .await?;
        println!("  ✓ Built image");

        // Inject keys
        println!("  → Injecting keys...");
        let provisioner = get_provisioner(provisioner_name)?;
        provisioner.check_dependencies()?;

        let mut config =
            ProvisionerConfig::new().with_ssh_key_path(bootstrap_config.ssh_key_path().to_string());
        if let Some(device) = &bootstrap_config.root_fs_device {
            config = config.with_root_fs_device(device.clone());
        }
        if let Some(label) = bootstrap_config.root_fs_label() {
            config = config.with_root_fs_label(label.to_string());
        }

        let image_path =
            match provisioner.inject_keys(Path::new(store_path.as_str()), &keys, &config)? {
                InjectionResult::Success { image_path } => {
                    println!("  ✓ Injected SSH keys");
                    image_path
                }
                InjectionResult::ManualRequired { .. } => {
                    return Err(snowlib::ProvisionerError::InjectionFailed {
                        provisioner: provisioner_name.to_string(),
                        reason: "Provisioner requires manual key injection".to_string(),
                    }
                    .into());
                }
            };

        // Deploy if target specified
        if let Some(target_str) = &self.target {
            let target = TargetType::parse(target_str);
            provisioner.validate_target(&target)?;

            if !self.yes && requires_confirmation(&target) && !confirm_operation(&target) {
                println!("  ✗ Deployment cancelled");
                return Ok(());
            }

            println!("  → Deploying to {}...", target);
            match provisioner.deploy_to_target(&image_path, &target, &self.hostname)? {
                ProvisioningResult::Success { message } => {
                    println!("  ✓ {}", message);
                }
                ProvisioningResult::NotSupported { reason } => {
                    return Err(snowlib::ProvisionerError::DeploymentNotSupported {
                        provisioner: provisioner_name.to_string(),
                        reason,
                    }
                    .into());
                }
            }
        }

        // Success output
        println!();
        println!("Age public key: {}", keys.age_public_key);

        if self.target.is_some() {
            println!();
            println!("After the host is running:");
            println!("  snow push {}", self.hostname);
        } else {
            println!("Image: {}", image_path.display());
        }

        Ok(())
    }
}

#[cfg(debug_assertions)]
fn create_backends(
    args: &BootstrapArgs,
) -> (
    Box<dyn KeyGenBackend>,
    Box<dyn NixBackend>,
    Box<dyn SopsBackend>,
) {
    if args.mock {
        (
            Box::new(snowlib::MockKeyGenBackend::new()),
            Box::new(snowlib::MockNixBackend::new()),
            Box::new(snowlib::MockSopsBackend::new()),
        )
    } else {
        (
            Box::new(SystemKeyGenBackend::new()),
            Box::new(SystemNixBackend::new()),
            Box::new(SystemSopsBackend::new()),
        )
    }
}

#[cfg(not(debug_assertions))]
fn create_backends(
    _args: &BootstrapArgs,
) -> (
    Box<dyn KeyGenBackend>,
    Box<dyn NixBackend>,
    Box<dyn SopsBackend>,
) {
    (
        Box::new(SystemKeyGenBackend::new()),
        Box::new(SystemNixBackend::new()),
        Box::new(SystemSopsBackend::new()),
    )
}

fn sops_updated(files_updated: usize) -> String {
    match files_updated {
        0 => "Updated .sops.yaml".to_string(),
        n => format!(
            "Updated .sops.yaml and re-encrypted {n} secret {}",
            plural(n, "file", "files")
        ),
    }
}

fn requires_confirmation(target: &TargetType) -> bool {
    matches!(target, TargetType::BlockDevice(_))
}

fn confirm_operation(target: &TargetType) -> bool {
    let warning = match target {
        TargetType::BlockDevice(path) => {
            format!(
                "WARNING: This will OVERWRITE all data on {}",
                path.display()
            )
        }
        TargetType::ProxmoxHost(t) => {
            format!("This will create a new VM on {}", t.hostname)
        }
    };

    print!("{}\nContinue? [y/N] ", warning);
    let _ = io::stdout().flush();

    let mut input = String::new();
    io::stdin().read_line(&mut input).is_ok() && input.trim().eq_ignore_ascii_case("y")
}

#[cfg(test)]
mod tests {
    use super::sops_updated;
    use crate::test_utils::snow_cmd;
    use predicates::prelude::*;

    #[test]
    fn test_sops_updated_counts_re_encrypted_files() {
        assert_eq!(sops_updated(0), "Updated .sops.yaml");
        assert_eq!(
            sops_updated(1),
            "Updated .sops.yaml and re-encrypted 1 secret file"
        );
        assert_eq!(
            sops_updated(3),
            "Updated .sops.yaml and re-encrypted 3 secret files"
        );
    }

    #[test]
    fn test_bootstrap_help() {
        snow_cmd()
            .arg("bootstrap")
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("Bootstrap a new host"))
            .stdout(predicate::str::contains("HOSTNAME"))
            .stdout(predicate::str::contains("--provisioner"))
            .stdout(predicate::str::contains("--verbose"));
    }

    #[test]
    fn test_bootstrap_requires_hostname() {
        snow_cmd()
            .arg("bootstrap")
            .assert()
            .failure()
            .stderr(predicate::str::contains("required"));
    }

    #[test]
    fn test_bootstrap_help_shows_update_sops_flag() {
        snow_cmd()
            .arg("bootstrap")
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("--update-sops"))
            .stdout(predicate::str::contains(".sops.yaml"));
    }

    #[test]
    fn test_bootstrap_help_shows_target_flag() {
        snow_cmd()
            .arg("bootstrap")
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("--target"))
            .stdout(predicate::str::contains("/dev/sdX"))
            .stdout(predicate::str::contains("proxmox-host"));
    }

    #[test]
    fn test_bootstrap_help_shows_yes_flag() {
        snow_cmd()
            .arg("bootstrap")
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("--yes"))
            .stdout(predicate::str::contains("-y"))
            .stdout(predicate::str::contains("Skip confirmation"));
    }

    // --mock only exists in debug builds
    #[cfg(debug_assertions)]
    mod mock_flag {
        use super::*;

        #[test]
        fn test_bootstrap_with_mock_generates_age_key() {
            snow_cmd()
                .arg("bootstrap")
                .arg("testhost")
                .arg("--mock")
                .assert()
                .success()
                .stdout(predicate::str::contains("Age public key: age1"))
                .stdout(predicate::str::contains("Generated SSH host key"));
        }

        #[test]
        fn test_bootstrap_shows_provisioner_in_output() {
            snow_cmd()
                .arg("bootstrap")
                .arg("testhost")
                .arg("--mock")
                .assert()
                .success()
                .stdout(predicate::str::contains("Bootstrapping testhost"));
        }

        #[test]
        fn test_bootstrap_with_provisioner_override() {
            snow_cmd()
                .arg("bootstrap")
                .arg("testhost")
                .arg("--mock")
                .arg("--provisioner")
                .arg("mock")
                .assert()
                .success()
                .stdout(predicate::str::contains("Bootstrapping testhost (mock)"));
        }

        #[test]
        fn test_bootstrap_with_update_sops_updates_sops_yaml() {
            snow_cmd()
                .arg("bootstrap")
                .arg("testhost")
                .arg("--mock")
                .arg("--update-sops")
                .assert()
                .success()
                .stdout(predicate::str::contains("Updating .sops.yaml"))
                .stdout(predicate::str::contains("Updated .sops.yaml"));
        }
    }
}
