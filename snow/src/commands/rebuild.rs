use super::deploy::{failed_host, format_elapsed, mode_label, success_line, HostLine};
use crate::cursor::Cursor;
use crate::marks;
use crate::prompt;
use clap::Args;
use console::style;
use snowlib::{
    activate_reporting_failed_units, ActivationMode, DeployTarget, Flake, LocalDeployBackend,
    LocalRebootBackend, PipelineStep, RebootBackend, RebootCheck, Result, Sudo, SudoPrompt,
    SudoTerminal, SystemType,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Rebuild the current host
#[derive(Debug, Args)]
#[command(group = super::activation_modes())]
pub struct RebuildArgs {
    /// Host to rebuild (defaults to this machine's hostname: /etc/hostname on NixOS, `hostname -s` on macOS)
    #[arg(long)]
    pub host: Option<String>,

    /// Dry run - show what would change without activating
    #[arg(long)]
    pub dry_run: bool,

    /// Switch to the new system now, asking first if it needs a reboot (the default)
    #[arg(long)]
    pub switch: bool,

    /// Only set for next boot (don't activate now)
    #[arg(long)]
    pub boot: bool,

    /// Test activation without making permanent
    #[arg(long)]
    pub test: bool,

    /// When switching to a system that needs a reboot, use it from the next boot without asking
    #[arg(long)]
    pub skip_reboot: bool,

    /// When switching to a system that needs a reboot, switch now without asking
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

    /// Fails before the build if this machine can't activate in the asked mode
    fn check_mode(&self, hostname: &str, system_type: SystemType) -> Result<()> {
        Ok(snowlib::check_mode_supported(
            self.activation_mode(),
            [(hostname, system_type)],
        )?)
    }

    /// Asks what to do about a needed reboot, unless a flag already said
    fn prompt_reboot_action(&self, check: &RebootCheck) -> Result<RebootAction> {
        println!("{}", reboot_notice(check));

        if self.force {
            println!(
                "--force: switching without rebooting\n\n{}",
                marks::warning_lines(SWITCH_WARNING)
            );
            return Ok(RebootAction::SwitchRisky);
        }
        if self.skip_reboot {
            println!("--skip-reboot: {}", NEXT_BOOT.to_lowercase());
            return Ok(RebootAction::BootOnly);
        }

        let labels: Vec<&str> = MENU.iter().map(|(label, _)| *label).collect();
        let selection = prompt::select("What now?", &labels)?;
        let action = MENU[selection].1;
        if action != RebootAction::SwitchRisky {
            return Ok(action);
        }

        println!("\n{}", marks::warning_lines(SWITCH_WARNING));
        let confirmed = prompt::confirm("Switch anyway?")?;
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
        let hostname = self.resolve_hostname()?;
        tracing::info!("{hostname}: rebuilding");
        let started = Instant::now();
        let line = Arc::new(HostLine::new(&hostname));

        let report = |ending: Ending| {
            line.hide();
            println!("{}", ending.line(&hostname, started.elapsed()));
        };

        match self.rebuild_with(flake, &hostname, &line, &report).await {
            Ok(Some(ending)) => report(ending),
            Ok(None) => {}
            Err(error) => {
                line.hide();
                println!("{}", failed_line(&hostname, line.step(), started.elapsed()));
                return Err(error);
            }
        }
        Ok(())
    }

    /// None once the ending went to `report` already
    async fn rebuild_with(
        &self,
        flake: &Flake,
        hostname: &str,
        line: &Arc<HostLine>,
        report: &dyn Fn(Ending),
    ) -> Result<Option<Ending>> {
        let system_type = SystemType::detect_current();
        self.check_mode(hostname, system_type)?;

        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        let output_mode = if self.quiet {
            snowlib::OutputMode::Capture
        } else {
            snowlib::OutputMode::Display
        };

        // Asked before the build, so a long build doesn't wait for a
        // password typed only once it is done
        let sudo = Arc::new(
            Sudo::start(Arc::new(SudoView {
                hostname: hostname.to_string(),
                line: line.clone(),
            }))
            .await?,
        );

        line.start(PipelineStep::Build);
        let store_path = nix_backend
            .build_host(flake, hostname, system_type, output_mode)
            .await?;
        tracing::info!("{hostname}: built {}", store_path.as_str());

        let target = DeployTarget {
            user: None,
            hostname: hostname.to_string(),
            port: None,
        };

        let deploy_backend = LocalDeployBackend::new(sudo.clone()).with_output_mode(output_mode);
        let reboot_backend = LocalRebootBackend::new(sudo);

        let (deployer, host, system) = (&deploy_backend, &target, &store_path);
        let activate = |mode| async move {
            line.start(PipelineStep::Activate);
            // Reading the host's ignoredFailedUnits would take an evaluation
            // and a hooks build of its own, so rebuild ignores no units
            activate_reporting_failed_units(deployer, host, system, mode, system_type, &[]).await?;
            tracing::info!("{hostname}: activated ({mode:?} mode)");
            Result::Ok(Some(Ending::Activated {
                mode,
                rebooting: false,
            }))
        };

        let mode = self.activation_mode();
        if mode != ActivationMode::Switch {
            return activate(mode).await;
        }

        line.start(PipelineStep::Check);
        let reboot_check = reboot_backend
            .check_reboot_required(&target, &store_path)
            .await?;
        if !reboot_check.is_required() {
            tracing::info!("{hostname}: no reboot required, switching");
            return activate(mode).await;
        }

        tracing::info!("{hostname}: reboot required: {}", reboot_check.summary());
        let action = line.paused(|| self.prompt_reboot_action(&reboot_check))?;
        tracing::info!("{hostname}: chose to {}", action.describe());

        match action {
            RebootAction::BootAndReboot => {
                activate(ActivationMode::Boot).await?;
                line.start(PipelineStep::Reboot);
                tracing::info!("{hostname}: rebooting");
                // Before the reboot, which may take snow down before it says so
                report(Ending::Activated {
                    mode: ActivationMode::Boot,
                    rebooting: true,
                });
                reboot_backend.reboot_and_wait(&target).await?;
                Ok(None)
            }
            RebootAction::BootOnly => activate(ActivationMode::Boot).await,
            RebootAction::SwitchRisky => {
                activate(mode).await?;
                Ok(Some(Ending::SwitchedBeforeReboot))
            }
            RebootAction::Cancel => Ok(Some(Ending::Cancelled)),
        }
    }
}

/// How a rebuild that didn't fail ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// `rebooting` once the reboot is queued: snow goes down with the host
    Activated {
        mode: ActivationMode,
        rebooting: bool,
    },
    SwitchedBeforeReboot,
    Cancelled,
}

impl Ending {
    /// As `snow deploy` reports a host, with how long the rebuild took
    fn line(&self, hostname: &str, took: Duration) -> String {
        let outcome = match *self {
            Ending::Activated { mode, rebooting } => {
                let labels = [mode_label(mode), "rebooting"];
                success_line(hostname, &labels[..if rebooting { 2 } else { 1 }])
            }
            Ending::SwitchedBeforeReboot => success_line(
                hostname,
                &[mode_label(ActivationMode::Switch), "reboot to finish"],
            ),
            Ending::Cancelled => marks::skipped_line(&format!("{hostname} (cancelled)")),
        };
        with_time(outcome, took)
    }
}

/// The error itself follows, printed by `main`
fn failed_line(hostname: &str, step: Option<PipelineStep>, took: Duration) -> String {
    with_time(failed_host(hostname, step), took)
}

fn with_time(line: String, took: Duration) -> String {
    format!(
        "{line} {}",
        style(format!("in {}", format_elapsed(took))).dim()
    )
}

/// Makes way for sudo, with the cursor shown as at any typed answer
struct SudoView {
    hostname: String,
    line: Arc<HostLine>,
}

impl SudoTerminal for SudoView {
    fn pause(&self, prompt: SudoPrompt) {
        self.line.pause();
        if prompt == SudoPrompt::Password {
            println!("{} needs sudo", self.hostname);
        }
        Cursor::stderr().show();
    }

    fn resume(&self) {
        Cursor::stderr().hide();
        self.line.resume();
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
    fn test_endings_read_as_deploy_reports_a_host() {
        console::set_colors_enabled(false);
        let took = Duration::from_secs(41);
        let line = |ending: Ending| ending.line("bravo", took);

        assert_eq!(
            line(Ending::Activated {
                mode: ActivationMode::Switch,
                rebooting: false
            }),
            "✓ bravo (switch) in 0:41"
        );
        assert_eq!(
            line(Ending::Activated {
                mode: ActivationMode::Boot,
                rebooting: true
            }),
            "✓ bravo (boot, rebooting) in 0:41"
        );
        assert_eq!(
            line(Ending::SwitchedBeforeReboot),
            "✓ bravo (switch, reboot to finish) in 0:41"
        );
        assert_eq!(line(Ending::Cancelled), "⊘ bravo (cancelled) in 0:41");
    }

    #[test]
    fn test_failure_says_at_which_step() {
        console::set_colors_enabled(false);
        let took = Duration::from_secs(72);

        assert_eq!(
            failed_line("bravo", Some(PipelineStep::Activate), took),
            "✗ bravo at activate in 1:12"
        );
        assert_eq!(failed_line("bravo", None, took), "✗ bravo in 1:12");
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
        let warning = console::strip_ansi_codes(&marks::warning_lines(SWITCH_WARNING)).into_owned();
        assert!(warning.lines().all(|line| line.chars().count() <= 80));
    }

    #[test]
    fn test_activation_modes_are_mutually_exclusive() {
        let error = crate::test_utils::try_parse_args::<RebuildArgs>(&["--dry-run", "--switch"])
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("the argument '--dry-run' cannot be used with '--switch'"));
    }

    #[test]
    fn test_force_goes_with_any_activation_mode() {
        for mode in ["--dry-run", "--switch", "--boot", "--test"] {
            let args = crate::test_utils::try_parse_args::<RebuildArgs>(&[mode, "--force"]);

            assert!(args.unwrap().force, "{mode} --force");
        }
    }

    #[test]
    fn test_darwin_rebuild_refuses_modes_other_than_switch() {
        let parse =
            |flags: &[&str]| crate::test_utils::try_parse_args::<RebuildArgs>(flags).unwrap();

        for mode in ["--boot", "--test", "--dry-run"] {
            let result = parse(&[mode]).check_mode("falcon", SystemType::Darwin);

            assert!(
                matches!(
                    result,
                    Err(snowlib::error::SnowError::Deploy(
                        snowlib::error::DeployError::DarwinOnlySwitches { .. }
                    ))
                ),
                "{mode}: {result:?}"
            );
        }
        assert!(parse(&[]).check_mode("falcon", SystemType::Darwin).is_ok());
        assert!(parse(&["--boot"])
            .check_mode("bravo", SystemType::NixOS)
            .is_ok());
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
