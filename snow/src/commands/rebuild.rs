use clap::Args;
use dialoguer::{theme::ColorfulTheme, Confirm, Select};
use snowlib::{
    ActivationMode, DeployBackend, DeployTarget, Flake, LocalDeployBackend, LocalRebootBackend,
    RebootBackend, Result, SystemType,
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

    /// Skip automatic reboot execution (still respects reboot policies for activation mode)
    #[arg(long)]
    pub skip_reboot: bool,

    /// Override policies and safety checks (skip interactive prompts and force requested mode)
    #[arg(long)]
    pub force: bool,

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

    /// Determine activation mode from flags
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
        if self.force {
            println!("⚠️  Reboot required ({}), but --force specified", reason);
            println!("   Forcing switch mode (may leave system in inconsistent state)");
            return Ok(RebootAction::SwitchRisky);
        }

        if self.skip_reboot {
            println!(
                "⚠️  Reboot required ({}), but --skip-reboot specified",
                reason
            );
            println!("   Using boot mode (changes will apply on next boot)");
            return Ok(RebootAction::BootOnly);
        }

        println!("\n⚠️  Reboot required: {}", reason);

        let options = vec![
            "Reboot now and activate",
            "Activate on next boot",
            "Activate immediately",
            "Cancel",
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
            2 => {
                let confirmed = Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("This will activate without rebooting, which may leave the system in an inconsistent state. Continue?")
                    .default(false)
                    .interact()
                    .map_err(|e| snowlib::error::DeployError::Failed {
                        host: "localhost".to_string(),
                        source: std::io::Error::other(e.to_string()),
                    })?;

                if confirmed {
                    Ok(RebootAction::SwitchRisky)
                } else {
                    Ok(RebootAction::Cancel)
                }
            }
            3 => Ok(RebootAction::Cancel),
            _ => unreachable!(),
        }
    }

    pub async fn run(self, flake: &Flake) -> Result<()> {
        self.validate()?;

        let hostname = self.resolve_hostname()?;
        println!("Rebuilding configuration for {}", hostname);

        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        let output_mode = if self.quiet {
            snowlib::OutputMode::Capture
        } else {
            snowlib::OutputMode::Inherit
        };

        let system_type = SystemType::detect_current();

        println!("Building configuration...");
        let store_path = nix_backend
            .build_host(flake, &hostname, system_type, output_mode)
            .await?;
        println!("Built {}", store_path.as_str());

        let target = DeployTarget {
            user: None,
            hostname: hostname.clone(),
            port: None,
        };

        let deploy_backend = LocalDeployBackend::new().with_output_mode(output_mode);
        let reboot_backend = LocalRebootBackend::default();

        let mode = self.activation_mode();

        // Handle non-switch modes early (dry-run, test, boot)
        if mode != ActivationMode::Switch {
            println!("Activating configuration...");
            deploy_backend.activate(&target, &store_path, mode).await?;
            match mode {
                ActivationMode::DryActivate => println!("✅ Dry run complete"),
                ActivationMode::Test => println!("✅ Test activation complete"),
                ActivationMode::Boot => println!("✅ Configuration will apply on next boot"),
                ActivationMode::Switch => unreachable!(),
            }
            return Ok(());
        }

        // For switch mode, check if reboot is required
        println!("Checking if reboot is required...");
        let reboot_check = reboot_backend
            .check_reboot_required(&target, &store_path)
            .await?;

        // If no reboot needed, activate immediately and exit
        if !reboot_check.required {
            println!("✅ No reboot required");
            println!("Activating configuration...");
            deploy_backend.activate(&target, &store_path, mode).await?;
            println!("✅ Configuration activated successfully");
            return Ok(());
        }

        // Reboot is required
        let reason = reboot_check
            .reason
            .map(|r| r.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Prompt user for action when reboot is required
        let action = self.prompt_reboot_action(&reason)?;

        match action {
            RebootAction::BootAndReboot => {
                println!("\nActivating in boot mode...");
                deploy_backend
                    .activate(&target, &store_path, ActivationMode::Boot)
                    .await?;
                println!("✅ Configuration activated for next boot");
                println!("Rebooting...");
                reboot_backend.reboot_and_wait(&target).await?;
            }
            RebootAction::BootOnly => {
                println!("\nActivating in boot mode...");
                deploy_backend
                    .activate(&target, &store_path, ActivationMode::Boot)
                    .await?;
                println!("✅ Configuration will apply on next boot");
                println!("Remember to reboot when convenient");
            }
            RebootAction::SwitchRisky => {
                println!("\n⚠️  Switching without reboot (risky)");
                deploy_backend.activate(&target, &store_path, mode).await?;
                println!("Configuration activated (may be unstable)");
            }
            RebootAction::Cancel => {
                println!("\n❌ Rebuild cancelled");
                return Ok(());
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
            skip_reboot: false,
            force: false,
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
            skip_reboot: false,
            force: false,
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
            skip_reboot: false,
            force: false,
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
            skip_reboot: false,
            force: false,
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
            skip_reboot: false,
            force: false,
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
            skip_reboot: false,
            force: false,
            quiet: false,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.resolve_hostname().unwrap(), "myhost");
    }
}
