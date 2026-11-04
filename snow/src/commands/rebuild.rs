use crate::marks;
use clap::Args;
use console::style;
use dialoguer::{theme::ColorfulTheme, Confirm, Select};
use snowlib::{
    ActivationMode, DeployBackend, DeployTarget, Flake, LocalDeployBackend, LocalRebootBackend,
    RebootBackend, RebootCheck, Result, SystemType,
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

    /// Asks what to do about a needed reboot, unless a flag already said
    fn prompt_reboot_action(&self, check: &RebootCheck) -> Result<RebootAction> {
        println!("\n{}", reboot_notice(check));

        if self.force {
            println!("--force: switching without rebooting\n\n{SWITCH_WARNING}");
            return Ok(RebootAction::SwitchRisky);
        }
        if self.skip_reboot {
            println!("--skip-reboot: {}", NEXT_BOOT.to_lowercase());
            return Ok(RebootAction::BootOnly);
        }

        let labels: Vec<&str> = MENU.iter().map(|(label, _)| *label).collect();
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("What now?")
            .items(&labels)
            .default(0)
            .interact()
            .map_err(prompt_error)?;
        let action = MENU[selection].1;
        if action != RebootAction::SwitchRisky {
            return Ok(action);
        }

        println!("\n{SWITCH_WARNING}");
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Switch anyway?")
            .default(false)
            .interact()
            .map_err(prompt_error)?;
        Ok(if confirmed {
            RebootAction::SwitchRisky
        } else {
            RebootAction::Cancel
        })
    }

    /// The error is printed by `main` too, but only the log keeps it
    pub async fn run(self, flake: &Flake) -> Result<()> {
        let result = self.rebuild(flake).await;
        if let Err(error) = &result {
            tracing::error!("rebuild failed: {}", snowlib::error::error_chain(error));
        }
        result
    }

    async fn rebuild(self, flake: &Flake) -> Result<()> {
        self.validate()?;

        let hostname = self.resolve_hostname()?;
        println!("Rebuilding configuration for {}", hostname);
        tracing::info!("{hostname}: rebuilding");

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
        tracing::info!("{hostname}: built {}", store_path.as_str());

        let target = DeployTarget {
            user: None,
            hostname: hostname.clone(),
            port: None,
        };

        let deploy_backend = LocalDeployBackend::new().with_output_mode(output_mode);
        let reboot_backend = LocalRebootBackend::default();

        let mode = self.activation_mode();

        let (deployer, host, system, name) = (&deploy_backend, &target, &store_path, &hostname);
        let activate = |mode| async move {
            deployer.activate(host, system, mode).await?;
            tracing::info!("{name}: activated ({mode:?} mode)");
            Result::Ok(())
        };

        // Handle non-switch modes early (dry-run, test, boot)
        if mode != ActivationMode::Switch {
            println!("Activating configuration...");
            activate(mode).await?;
            match mode {
                ActivationMode::DryActivate => println!("{} Dry run complete", marks::success()),
                ActivationMode::Test => println!("{} Test activation complete", marks::success()),
                ActivationMode::Boot => {
                    println!("{} {NEXT_BOOT}", marks::success())
                }
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
        if !reboot_check.is_required() {
            tracing::info!("{hostname}: no reboot required, switching");
            println!("{} No reboot needed", marks::success());
            println!("Activating configuration...");
            activate(mode).await?;
            println!("{} Switched to the new system", marks::success());
            return Ok(());
        }

        tracing::info!("{hostname}: reboot required: {}", reboot_check.summary());
        let action = self.prompt_reboot_action(&reboot_check)?;
        tracing::info!("{hostname}: chose to {}", action.describe());

        match action {
            RebootAction::BootAndReboot => {
                activate(ActivationMode::Boot).await?;
                println!("\n{} {NEXT_BOOT}", marks::success());
                println!("Rebooting...");
                tracing::info!("{hostname}: rebooting");
                reboot_backend.reboot_and_wait(&target).await?;
            }
            RebootAction::BootOnly => {
                activate(ActivationMode::Boot).await?;
                println!("\n{} {NEXT_BOOT}", marks::success());
            }
            RebootAction::SwitchRisky => {
                activate(mode).await?;
                println!(
                    "\n{} Switched; reboot when you can to finish",
                    marks::success()
                );
            }
            RebootAction::Cancel => {
                println!("\n{}", marks::skipped_line("Cancelled, nothing changed"));
                return Ok(());
            }
        }

        Ok(())
    }
}

/// Shown before asking to confirm a switch that needs a reboot. It can't say
/// what will break: that depends on what changed and on the host.
const SWITCH_WARNING: &str = "Until you reboot, the running system mixes old and new parts, and\n\
                              anything that depends on both may misbehave.";

const MENU: [(&str, RebootAction); 4] = [
    ("Reboot into it now", RebootAction::BootAndReboot),
    ("Use it from the next boot", RebootAction::BootOnly),
    ("Switch now, reboot later", RebootAction::SwitchRisky),
    ("Cancel", RebootAction::Cancel),
];

fn reboot_notice(check: &RebootCheck) -> String {
    std::iter::once(style("Reboot needed:").yellow().bold().to_string())
        .chain(check.reasons.iter().map(|reason| format!("  • {reason}")))
        .collect::<Vec<_>>()
        .join("\n")
}

const NEXT_BOOT: &str = "The new system starts at the next boot";

fn prompt_error(error: dialoguer::Error) -> snowlib::error::SnowError {
    snowlib::error::DeployError::Failed {
        host: "localhost".to_string(),
        source: std::io::Error::other(error.to_string()),
    }
    .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RebootAction {
    BootAndReboot,
    BootOnly,
    SwitchRisky,
    Cancel,
}

impl RebootAction {
    fn describe(&self) -> &'static str {
        match self {
            RebootAction::BootAndReboot => "reboot now",
            RebootAction::BootOnly => "activate on the next boot",
            RebootAction::SwitchRisky => "activate without rebooting",
            RebootAction::Cancel => "cancel",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowlib::backends::reboot::RebootReason;

    #[test]
    fn test_reboot_notice_lists_every_reason() {
        console::set_colors_enabled(false);
        let check = RebootCheck::from_reasons(vec![
            RebootReason::KernelChanged {
                old: "/nix/store/0n6kcz8q4lw1-linux-6.12.58/bzImage".to_string(),
                new: "/nix/store/9s2mf0yh7l3a-linux-6.12.60/bzImage".to_string(),
            },
            RebootReason::CriticalService("network-setup.service".to_string()),
        ]);

        assert_eq!(
            reboot_notice(&check),
            "Reboot needed:\n  • kernel linux-6.12.58 → linux-6.12.60\n  • network-setup.service would restart"
        );
    }

    #[test]
    fn test_menu_order_matches_its_actions() {
        let actions: Vec<_> = MENU.iter().map(|(_, action)| *action).collect();

        assert_eq!(
            actions,
            vec![
                RebootAction::BootAndReboot,
                RebootAction::BootOnly,
                RebootAction::SwitchRisky,
                RebootAction::Cancel,
            ]
        );
    }

    fn args_with(force: bool, skip_reboot: bool) -> RebuildArgs {
        RebuildArgs {
            host: None,
            dry_run: false,
            switch: false,
            boot: false,
            test: false,
            skip_reboot,
            force,
            quiet: false,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        }
    }

    fn initrd_changed() -> RebootCheck {
        RebootCheck::required(RebootReason::InitrdChanged {
            old: "/nix/store/b5h2sjpj-initrd-linux-6.12.58/initrd".to_string(),
            new: "/nix/store/96wpyhsq-initrd-linux-6.12.58/initrd".to_string(),
        })
    }

    #[test]
    fn test_force_switches_without_asking() {
        let action = args_with(true, false).prompt_reboot_action(&initrd_changed());

        assert_eq!(action.unwrap(), RebootAction::SwitchRisky);
    }

    #[test]
    fn test_skip_reboot_uses_the_new_system_from_the_next_boot_without_asking() {
        let action = args_with(false, true).prompt_reboot_action(&initrd_changed());

        assert_eq!(action.unwrap(), RebootAction::BootOnly);
    }

    #[test]
    fn test_switch_warning_fits_an_80_column_terminal() {
        assert!(SWITCH_WARNING
            .lines()
            .all(|line| line.chars().count() <= 80));
    }

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
