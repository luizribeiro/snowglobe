use clap::Args;
use snowlib::{
    get_provisioner, BootstrapConfig, Flake, InjectionResult, KeyGenBackend, NixBackend,
    OutputMode, ProvisionerConfig, ProvisioningResult, Result, SopsBackend, SystemKeyGenBackend,
    SystemNixBackend, SystemSopsBackend, SystemType, TargetType,
};
use std::io::{self, Write};
use std::path::Path;

/// Bootstrap a new host by generating SSH keys and deriving an age key
///
/// Generates the cryptographic identity for a new NixOS host:
/// - An ed25519 SSH host key pair
/// - An age public key derived from the SSH key (for use with SOPS)
///
/// If the host has a bootstrap configuration (snowglobe.bootstrap in NixOS config),
/// will also build the image and inject keys using the configured provisioner.
#[derive(Debug, Args)]
pub struct BootstrapArgs {
    /// Hostname to bootstrap
    pub hostname: String,

    /// Override the provisioner (ignores host config)
    #[arg(long)]
    pub provisioner: Option<String>,

    /// Update .sops.yaml with the new host's age key
    ///
    /// Adds the age key to the keys section and runs `sops updatekeys`
    /// on secret files in the secrets/ directory.
    #[arg(long)]
    pub update_sops: bool,

    /// Deploy the provisioned image to a target
    ///
    /// Target types:
    ///   /dev/sdX         - Flash to block device (sd-image, raw)
    ///   proxmox-host     - Deploy to Proxmox host (proxmox-vma)
    ///   user@host        - Deploy via SSH (future)
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
        let keygen_backend: Box<dyn KeyGenBackend> = {
            #[cfg(debug_assertions)]
            {
                if self.mock {
                    Box::new(snowlib::MockKeyGenBackend::new())
                } else {
                    Box::new(SystemKeyGenBackend::new())
                }
            }
            #[cfg(not(debug_assertions))]
            {
                Box::new(SystemKeyGenBackend::new())
            }
        };

        let nix_backend: Box<dyn NixBackend> = {
            #[cfg(debug_assertions)]
            {
                if self.mock {
                    Box::new(snowlib::MockNixBackend::new())
                } else {
                    Box::new(SystemNixBackend::new())
                }
            }
            #[cfg(not(debug_assertions))]
            {
                Box::new(SystemNixBackend::new())
            }
        };

        let sops_backend: Box<dyn SopsBackend> = {
            #[cfg(debug_assertions)]
            {
                if self.mock {
                    Box::new(snowlib::MockSopsBackend::new())
                } else {
                    Box::new(SystemSopsBackend::new())
                }
            }
            #[cfg(not(debug_assertions))]
            {
                Box::new(SystemSopsBackend::new())
            }
        };

        // Get bootstrap config from NixOS module
        let bootstrap_config = match nix_backend
            .get_bootstrap_config(flake, &self.hostname, SystemType::NixOS)
            .await
        {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Warning: Failed to evaluate bootstrap config: {}", e);
                eprintln!("  Proceeding with default config (no image building)");
                BootstrapConfig::default()
            }
        };

        let provisioner_name = self
            .provisioner
            .as_deref()
            .unwrap_or_else(|| bootstrap_config.provisioner_name());

        println!("Bootstrapping {} ({})...", self.hostname, provisioner_name);

        // Build image first if configured (fail fast before generating keys)
        let image_path = if let Some(image_attr) = &bootstrap_config.image_attr {
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
            println!(
                "  ✓ Built image: .#nixosConfigurations.{}.{}",
                self.hostname, image_attr
            );
            Some(store_path)
        } else {
            None
        };

        // Generate keys
        keygen_backend.check_dependencies()?;
        println!("  → Generating SSH host key...");
        let keys = snowlib::bootstrap::bootstrap_host(&self.hostname, keygen_backend.as_ref())?;
        println!("  ✓ Generated SSH host key");

        // Update SOPS if requested
        if self.update_sops {
            println!("  → Updating .sops.yaml...");
            let result = sops_backend.update_sops(
                flake.flake_root(),
                &self.hostname,
                &keys.age_public_key,
            )?;
            if result.files_updated > 0 {
                println!(
                    "  ✓ Updated .sops.yaml and re-encrypted {} secret file(s)",
                    result.files_updated
                );
            } else {
                println!("  ✓ Updated .sops.yaml");
            }
        }

        // Inject keys if we have an image
        if let Some(store_path) = &image_path {
            println!("  → Injecting keys...");
            let provisioner = get_provisioner(provisioner_name)?;
            provisioner.check_dependencies()?;

            let config = ProvisionerConfig::new()
                .with_ssh_key_path(bootstrap_config.ssh_key_path().to_string());

            match provisioner.inject_keys(Path::new(store_path.as_str()), &keys, &config)? {
                InjectionResult::Success { image_path } => {
                    println!("  ✓ Injected SSH keys");

                    // Deploy to target if specified
                    if let Some(target_str) = &self.target {
                        let target = TargetType::parse(target_str);
                        provisioner.validate_target(&target)?;

                        if !self.yes
                            && requires_confirmation(&target)
                            && !confirm_destructive_operation(&target)
                        {
                            println!("  ✗ Deployment cancelled");
                            return Ok(());
                        }

                        println!("  → Deploying to {}...", target);
                        match provisioner.deploy_to_target(&image_path, &target, &self.hostname)? {
                            ProvisioningResult::Success { message } => {
                                println!("  ✓ {}", message);
                            }
                            ProvisioningResult::NotSupported { reason } => {
                                println!("  ⚠ Deployment not supported: {}", reason);
                            }
                        }
                    }

                    println!();
                    println!("  Age public key: {}", keys.age_public_key);
                    println!();

                    if self.target.is_none() {
                        println!("Output: {}", image_path.display());
                        println!();
                        let instructions =
                            provisioner.deployment_instructions(&image_path, &self.hostname);
                        println!("{}", instructions);
                    } else {
                        println!("After the host is running:");
                        println!("  snow push {}", self.hostname);
                    }
                }
                InjectionResult::ManualRequired { instructions } => {
                    println!("  ⚠ Manual key injection required");
                    println!();
                    println!("  Age public key: {}", keys.age_public_key);
                    println!();
                    println!("{}", instructions);
                    print_keys_info(&keys);
                }
            }
        } else {
            // No image config - just output key info
            print_keys_and_sops_info(&keys, &self.hostname, self.update_sops);
        }

        Ok(())
    }
}

fn requires_confirmation(target: &TargetType) -> bool {
    matches!(target, TargetType::BlockDevice(_))
}

fn confirm_destructive_operation(target: &TargetType) -> bool {
    let warning = match target {
        TargetType::BlockDevice(path) => {
            format!(
                "⚠  WARNING: This will OVERWRITE all data on {}",
                path.display()
            )
        }
        TargetType::ProxmoxHost(deploy_target) => {
            format!(
                "⚠  This will create a new VM on Proxmox host {}",
                deploy_target.hostname
            )
        }
    };

    print!("{}\nAre you sure you want to continue? [y/N] ", warning);
    let _ = io::stdout().flush();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    input.trim().eq_ignore_ascii_case("y")
}

fn print_keys_info(keys: &snowlib::bootstrap::HostKeys) {
    println!(
        "SSH keys stored in: {}",
        keys.temp_dir_root.join(&keys.hostname).display()
    );
    println!("  Private key: {}", keys.ssh_private_key.display());
    println!("  Public key:  {}", keys.ssh_public_key.display());
    println!();
    println!("⚠  Remember to delete the keys after copying to your image:");
    println!("   rm -rf {}", keys.temp_dir_root.display());
}

fn print_keys_and_sops_info(
    keys: &snowlib::bootstrap::HostKeys,
    hostname: &str,
    sops_updated: bool,
) {
    println!();
    println!("  Age public key: {}", keys.age_public_key);
    println!();
    if !sops_updated {
        println!("To use with SOPS, add to .sops.yaml:");
        println!("  keys:");
        println!("    - &{} {}", hostname, keys.age_public_key);
        println!();
        println!("Then run: sops updatekeys <your-secrets-file>");
        println!();
    }
    print_keys_info(keys);
}

#[cfg(test)]
mod tests {
    use crate::test_utils::snow_cmd;
    use predicates::prelude::*;

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
    fn test_bootstrap_shows_sops_instructions() {
        snow_cmd()
            .arg("bootstrap")
            .arg("myhost")
            .arg("--mock")
            .assert()
            .success()
            .stdout(predicate::str::contains("To use with SOPS"))
            .stdout(predicate::str::contains(".sops.yaml"));
    }

    #[test]
    fn test_bootstrap_shows_cleanup_warning() {
        snow_cmd()
            .arg("bootstrap")
            .arg("testhost")
            .arg("--mock")
            .assert()
            .success()
            .stdout(predicate::str::contains("Remember to delete the keys"))
            .stdout(predicate::str::contains("rm -rf"));
    }

    #[test]
    fn test_bootstrap_shows_provisioner_in_output() {
        snow_cmd()
            .arg("bootstrap")
            .arg("testhost")
            .arg("--mock")
            .assert()
            .success()
            .stdout(predicate::str::contains("Bootstrapping testhost (manual)"));
    }

    #[test]
    fn test_bootstrap_with_provisioner_override() {
        snow_cmd()
            .arg("bootstrap")
            .arg("testhost")
            .arg("--mock")
            .arg("--provisioner")
            .arg("manual")
            .assert()
            .success()
            .stdout(predicate::str::contains("Bootstrapping testhost (manual)"));
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

    #[test]
    fn test_bootstrap_with_update_sops_skips_sops_instructions() {
        snow_cmd()
            .arg("bootstrap")
            .arg("testhost")
            .arg("--mock")
            .arg("--update-sops")
            .assert()
            .success()
            .stdout(predicate::str::contains("Age public key: age1"))
            // Should NOT contain manual SOPS instructions when --update-sops is used
            .stdout(predicate::str::contains("To use with SOPS").not());
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
}
