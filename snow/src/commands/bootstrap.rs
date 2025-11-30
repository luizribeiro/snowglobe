use clap::Args;
use snowlib::{Flake, KeyGenBackend, Result, SystemKeyGenBackend};

/// Bootstrap a new host by generating SSH keys and deriving an age key
///
/// Generates the cryptographic identity for a new NixOS host:
/// - An ed25519 SSH host key pair
/// - An age public key derived from the SSH key (for use with SOPS)
#[derive(Debug, Args)]
pub struct BootstrapArgs {
    /// Hostname to bootstrap
    pub hostname: String,

    #[cfg(debug_assertions)]
    #[arg(long, hide = true)]
    pub mock: bool,
}

impl BootstrapArgs {
    pub async fn run(self, _flake: &Flake) -> Result<()> {
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

        println!("Bootstrapping {}...", self.hostname);

        keygen_backend.check_dependencies()?;

        println!("  → Generating SSH host key...");
        let keys = snowlib::bootstrap::bootstrap_host(&self.hostname, keygen_backend.as_ref())?;
        println!("  ✓ Generated SSH host key");

        println!();
        println!("  Age public key: {}", keys.age_public_key);
        println!();
        println!("To use with SOPS, add to .sops.yaml:");
        println!("  keys:");
        println!("    - &{} {}", self.hostname, keys.age_public_key);
        println!();
        println!("Then run: sops updatekeys <your-secrets-file>");
        println!();
        println!(
            "SSH keys stored in: {}",
            keys.temp_dir_root.join(&self.hostname).display()
        );
        println!("  Private key: {}", keys.ssh_private_key.display());
        println!("  Public key:  {}", keys.ssh_public_key.display());
        println!();
        println!("⚠  Remember to delete the keys after copying to your image:");
        println!("   rm -rf {}", keys.temp_dir_root.display());

        Ok(())
    }
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
            .stdout(predicate::str::contains("HOSTNAME"));
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
}
