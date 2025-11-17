use clap::Args;
use dialoguer::{theme::ColorfulTheme, Select};
use snowlib::{
    ActivationMode, DeployBackend, DeployTarget, Flake, LocalDeployBackend, LocalRebootBackend,
    RebootBackend, Result,
};

/// Rebuild the current host
#[derive(Debug, Args)]
pub struct RebuildArgs {
    /// Host to rebuild (defaults to auto-detected hostname from /etc/hostname)
    #[arg(long)]
    pub host: Option<String>,

    /// Dry run - show what would change without activating
    #[arg(long)]
    pub dry_run: bool,

    /// Force immediate activation (switch mode)
    #[arg(long)]
    pub switch: bool,

    /// Only set for next boot (don't activate now)
    #[arg(long)]
    pub boot: bool,

    /// Test activation without making permanent
    #[arg(long)]
    pub test: bool,

    /// Never automatically reboot (even if reboot is required)
    #[arg(long)]
    pub no_reboot: bool,

    /// Quiet mode - suppress subprocess output
    #[arg(short, long)]
    pub quiet: bool,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock backend (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

impl RebuildArgs {
    /// Validate flag combinations
    fn validate(&self) -> Result<()> {
        let mode_count = [self.dry_run, self.switch, self.boot, self.test]
            .iter()
            .filter(|&&x| x)
            .count();

        if mode_count > 1 {
            return Err(snowlib::error::DeployError::Failed {
                host: "validation".to_string(),
                source: std::io::Error::other(
                    "Cannot use multiple activation modes (--dry-run, --switch, --boot, --test)",
                ),
            }
            .into());
        }

        Ok(())
    }

    /// Determine the hostname to rebuild
    fn resolve_hostname(&self) -> Result<String> {
        if let Some(ref host) = self.host {
            Ok(host.clone())
        } else {
            snowlib::get_hostname()
        }
    }

    /// Determine the activation mode from flags
    fn activation_mode(&self) -> ActivationMode {
        if self.dry_run {
            ActivationMode::DryActivate
        } else if self.test {
            ActivationMode::Test
        } else if self.boot {
            ActivationMode::Boot
        } else {
            ActivationMode::Switch
        }
    }

    /// Show interactive reboot prompt and return the selected activation mode
    fn prompt_reboot_action(&self, reason: &str) -> Result<RebootAction> {
        if self.no_reboot {
            println!(
                "⚠️  Reboot required ({}), but --no-reboot specified",
                reason
            );
            println!("   Using boot mode (changes will apply on next boot)");
            return Ok(RebootAction::BootOnly);
        }

        println!("\n⚠️  Reboot required: {}", reason);

        let options = vec![
            "Reboot now (boot + reboot)",
            "Reboot later (boot only)",
            "Switch without rebooting (risky)",
            "Cancel (don't activate)",
        ];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("What would you like to do?")
            .items(&options)
            .default(0)
            .interact()
            .map_err(|e| snowlib::error::DeployError::Failed {
                host: "localhost".to_string(),
                source: std::io::Error::other(e.to_string()),
            })?;

        match selection {
            0 => Ok(RebootAction::BootAndReboot),
            1 => Ok(RebootAction::BootOnly),
            2 => Ok(RebootAction::SwitchRisky),
            3 => Ok(RebootAction::Cancel),
            _ => unreachable!(),
        }
    }

    pub fn run(self, flake: &Flake) -> Result<()> {
        self.validate()?;

        let hostname = self.resolve_hostname()?;
        println!("🔨 Rebuilding configuration for {}", hostname);

        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        println!("📦 Building configuration...");
        let store_path = nix_backend.build_host(flake, &hostname, self.quiet)?;
        println!("✅ Built: {}", store_path.as_str());

        let target = DeployTarget {
            user: None,
            hostname: hostname.clone(),
            port: None,
        };

        let deploy_backend = LocalDeployBackend::new().with_quiet(self.quiet);
        let reboot_backend = LocalRebootBackend::default();

        // Determine activation mode
        let mut mode = self.activation_mode();

        // Check if reboot is required (unless in dry-run or test mode)
        if mode == ActivationMode::Switch && !self.dry_run && !self.test {
            println!("🔍 Checking if reboot is required...");
            let reboot_check = reboot_backend.check_reboot_required(&target, &store_path)?;

            if reboot_check.required {
                let reason = reboot_check
                    .reason
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let action = self.prompt_reboot_action(&reason)?;

                match action {
                    RebootAction::BootAndReboot => {
                        mode = ActivationMode::Boot;
                        println!("\n🚀 Activating in boot mode...");
                        deploy_backend.activate(&target, &store_path, mode)?;
                        println!("✅ Configuration activated for next boot");
                        println!("🔄 Rebooting...");
                        reboot_backend.reboot_and_wait(&target)?;
                    }
                    RebootAction::BootOnly => {
                        mode = ActivationMode::Boot;
                        println!("\n🚀 Activating in boot mode...");
                        deploy_backend.activate(&target, &store_path, mode)?;
                        println!("✅ Configuration will apply on next boot");
                        println!("💡 Remember to reboot when convenient");
                        return Ok(());
                    }
                    RebootAction::SwitchRisky => {
                        println!("\n⚠️  Switching without reboot (risky)...");
                        deploy_backend.activate(&target, &store_path, mode)?;
                        println!("✅ Configuration activated (but may be unstable)");
                        return Ok(());
                    }
                    RebootAction::Cancel => {
                        println!("\n❌ Rebuild cancelled");
                        return Err(snowlib::error::DeployError::Failed {
                            host: hostname,
                            source: std::io::Error::other("Cancelled by user"),
                        }
                        .into());
                    }
                }
            } else {
                println!("✅ No reboot required");
                println!("🚀 Activating configuration...");
                deploy_backend.activate(&target, &store_path, mode)?;
                println!("✅ Configuration activated successfully");
            }
        } else {
            println!("🚀 Activating configuration...");
            deploy_backend.activate(&target, &store_path, mode)?;
            match mode {
                ActivationMode::DryActivate => println!("✅ Dry run complete"),
                ActivationMode::Test => println!("✅ Test activation complete"),
                ActivationMode::Boot => println!("✅ Configuration will apply on next boot"),
                ActivationMode::Switch => println!("✅ Configuration activated successfully"),
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RebootAction {
    BootAndReboot,
    BootOnly,
    SwitchRisky,
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_mutually_exclusive_modes() {
        let args = RebuildArgs {
            host: None,
            dry_run: true,
            switch: true,
            boot: false,
            test: false,
            no_reboot: false,
            quiet: false,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_valid_combination() {
        let args = RebuildArgs {
            host: None,
            dry_run: false,
            switch: true,
            boot: false,
            test: false,
            no_reboot: false,
            quiet: false,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_activation_mode_switch() {
        let args = RebuildArgs {
            host: None,
            dry_run: false,
            switch: true,
            boot: false,
            test: false,
            no_reboot: false,
            quiet: false,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.activation_mode(), ActivationMode::Switch);
    }

    #[test]
    fn test_activation_mode_boot() {
        let args = RebuildArgs {
            host: None,
            dry_run: false,
            switch: false,
            boot: true,
            test: false,
            no_reboot: false,
            quiet: false,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.activation_mode(), ActivationMode::Boot);
    }

    #[test]
    fn test_activation_mode_dry_run() {
        let args = RebuildArgs {
            host: None,
            dry_run: true,
            switch: false,
            boot: false,
            test: false,
            no_reboot: false,
            quiet: false,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.activation_mode(), ActivationMode::DryActivate);
    }

    #[test]
    fn test_resolve_hostname_with_explicit_host() {
        let args = RebuildArgs {
            host: Some("myhost".to_string()),
            dry_run: false,
            switch: false,
            boot: false,
            test: false,
            no_reboot: false,
            quiet: false,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.resolve_hostname().unwrap(), "myhost");
    }
}
