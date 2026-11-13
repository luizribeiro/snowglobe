mod board;
mod colors;
mod host_line;
mod interrupted;
mod observer;
mod styles;
mod summary;

pub(super) use board::format_elapsed;
pub(super) use host_line::HostLine;
pub(super) use summary::{failed_host, success_line};

use clap::Args;
use snowlib::{DeployStrategy, Flake, Result};
use std::future::Future;
use tracing_indicatif::span_ext::IndicatifSpanExt;

/// So the command snow suggests works from any directory
fn absolute_flake_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|absolute| absolute.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Shows a spinner with `message` until `work` is done: evaluating a flake
/// can take minutes and prints nothing on its own
async fn with_spinner<T>(message: &str, work: impl Future<Output = T>) -> T {
    let span = tracing::info_span!("waiting");
    span.pb_set_style(&styles::waiting_style());
    span.pb_set_message(message);
    span.pb_start();
    work.await
}

/// Deploy configurations to hosts
#[derive(Debug, Args)]
#[command(group = super::activation_modes())]
pub struct DeployArgs {
    #[command(flatten)]
    pub selection: crate::host_selection::HostSelectionArgs,

    /// Show what would change without activating
    #[arg(long)]
    pub dry_run: bool,

    /// Switch now, blocking hosts that need a reboot instead of rebooting them; --force switches them anyway and ignores reboot policies
    #[arg(long)]
    pub switch: bool,

    /// Make the new system the boot default without activating it or rebooting
    #[arg(long)]
    pub boot: bool,

    /// Activate the new system without making it the boot default
    #[arg(long)]
    pub test: bool,

    /// Never reboot hosts (reboot policies still choose the activation mode)
    #[arg(long)]
    pub skip_reboot: bool,

    /// Deploy hosts even if they already run the built system
    ///
    /// Use this to retry a host whose last activation partly failed: NixOS
    /// records the new system as current before starting services.
    #[arg(long)]
    pub redeploy: bool,

    /// With --switch, ignore reboot policies and reboot detection
    // clap waives `requires` once another mode is given, so those modes are
    // excluded explicitly
    #[arg(long, requires = "switch", conflicts_with_all = ["dry_run", "boot", "test"])]
    pub force: bool,

    /// Suppress subprocess output and show only the summary
    #[arg(short, long)]
    pub quiet: bool,

    /// SSH user to use for deployment (defaults to SSH config or current user)
    #[arg(long)]
    pub user: Option<String>,

    /// Seconds to wait for a rebooted host to come back before failing it
    #[arg(
        long,
        default_value_t = snowlib::DEFAULT_REBOOT_MAX_WAIT_SECS,
        value_name = "SECONDS",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub reboot_timeout: u32,

    /// Maximum number of hosts to deploy in parallel (must be at least 1)
    #[arg(long, default_value = "3", value_parser = clap::value_parser!(u32).range(1..))]
    pub concurrency: u32,

    /// Ignore phases and deploy all hosts in parallel
    ///
    /// By default, hosts are deployed sequentially by phase. This flag disables
    /// phase-based deployment and runs all hosts in parallel (respecting concurrency limit).
    #[arg(long)]
    pub ignore_phases: bool,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock deployer (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

impl DeployArgs {
    /// Determine the deploy strategy from flags
    fn deploy_strategy(&self) -> DeployStrategy {
        if self.dry_run {
            DeployStrategy::DryRun
        } else if self.test {
            DeployStrategy::Test
        } else if self.boot {
            DeployStrategy::Boot
        } else if self.switch {
            DeployStrategy::Switch
        } else {
            DeployStrategy::Auto
        }
    }

    /// The command that deploys hosts again as this one would, before their
    /// names. Host selection is left out, as the hosts are named, and so are
    /// flags that only change what snow shows.
    fn redeploy_command(&self, flake_path: Option<&str>) -> Vec<String> {
        let mut words = vec!["snow".to_string()];
        if let Some(path) = flake_path {
            words.extend(["-f".to_string(), path.to_string()]);
        }
        words.extend(["deploy".to_string(), "--redeploy".to_string()]);
        for (set, flag) in [
            (self.switch, "--switch"),
            (self.boot, "--boot"),
            (self.test, "--test"),
            (self.force, "--force"),
            (self.skip_reboot, "--skip-reboot"),
            (self.ignore_phases, "--ignore-phases"),
        ] {
            if set {
                words.push(flag.to_string());
            }
        }
        if let Some(user) = &self.user {
            words.extend(["--user".to_string(), user.clone()]);
        }
        if self.reboot_timeout != snowlib::DEFAULT_REBOOT_MAX_WAIT_SECS {
            words.extend([
                "--reboot-timeout".to_string(),
                self.reboot_timeout.to_string(),
            ]);
        }
        words
    }

    /// Main entry point for the deploy command
    ///
    /// Orchestrates the entire deployment process by delegating to snowlib's `deploy()` function.
    /// The CLI's only responsibilities are:
    /// - Parse command-line arguments
    /// - Create backend implementations and observer
    /// - Call snowlib::deploy()
    /// - Print results table
    pub async fn run(
        self,
        flake: &Flake,
        flake_path: Option<&str>,
        log_file: Option<&std::path::Path>,
    ) -> Result<()> {
        let left_behind = interrupted::LeftBehind::new(
            self.redeploy_command(flake_path.map(absolute_flake_path).as_deref()),
        );
        crate::signals::report_on_exit({
            let left_behind = left_behind.clone();
            move || left_behind.report()
        });

        // When using --mock-fixture, use the mock flake instead of the real one
        #[cfg(debug_assertions)]
        let mock_flake;
        #[cfg(debug_assertions)]
        let flake = if self.mock_fixture.is_some() {
            // Use the mock flake bundled with the test fixtures
            let mock_flake_path =
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
            mock_flake = Flake::new(Some(mock_flake_path.to_str().unwrap()))?;
            &mock_flake
        } else {
            flake
        };

        // Lives until the deployment ends: its links stop the GC from collecting the
        // hooks and systems built for this deployment
        let gc_roots = tempfile::Builder::new()
            .prefix("snow-deploy-")
            .tempdir()
            .map_err(|source| snowlib::error::IoError::TempDir { source })?;
        let gc_roots_path = std::path::absolute(gc_roots.path())
            .map_err(|source| snowlib::error::IoError::TempDir { source })?;
        let nix_backend = crate::backend_factory::create_deploy_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
            &gc_roots_path,
        )?;

        let output_mode = if self.quiet {
            snowlib::OutputMode::Capture
        } else {
            snowlib::OutputMode::Display
        };

        let deploy_backend = crate::backend_factory::create_deploy_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
            output_mode,
        )?;
        let reboot_backend = crate::backend_factory::create_reboot_backend(
            self.user.as_deref(),
            self.reboot_timeout,
        )?;
        let hostnames = with_spinner(
            "Finding the hosts to deploy",
            self.selection.resolve(flake, nix_backend.as_ref()),
        )
        .await?;
        let hosts = with_spinner(
            &format!(
                "Reading the settings of {} {}",
                hostnames.len(),
                board::plural(hostnames.len(), "host")
            ),
            nix_backend.load_host_metadata(flake, &hostnames),
        )
        .await?;
        for host in &hosts {
            left_behind.has_hooks(&host.name, host.hooks.iter().map(|hook| hook.stage));
        }

        let phases = snowlib::phases(&hosts, self.ignore_phases);
        println!("{}\n", summary::phase_overview(&phases));
        let observer = snowlib::observer::CompositeObserver::new(vec![
            Box::new(snowlib::LogObserver::default()),
            Box::new(left_behind),
            Box::new(observer::CliObserver::new(
                log_file.map(Into::into),
                phases.len() > 1,
            )),
        ]);

        let config = snowlib::DeployConfiguration {
            strategy: self.deploy_strategy(),
            force: self.force,
            concurrency: self.concurrency as usize,
            ignore_phases: self.ignore_phases,
            output_mode,
            ssh_user: self.user.clone(),
            allow_reboot: !self.skip_reboot,
            redeploy: self.redeploy,
        };

        // Create hook executor from host metadata
        let hook_backend =
            crate::backend_factory::create_hook_backend(self.user.clone(), output_mode);
        let hooks_by_host = hosts
            .iter()
            .map(|h| (h.name.clone(), h.hooks.clone()))
            .collect();
        let hook_executor = snowlib::HookExecutor::new(hook_backend, hooks_by_host);

        let deploy_ctx = snowlib::DeployContext::new(
            flake,
            nix_backend.as_ref(),
            deploy_backend.as_ref(),
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );

        // Execute the deployment - snowlib handles everything!
        let results = snowlib::deploy(&deploy_ctx, hosts).await?;

        summary::print_summary(&results);

        if let Some(error) = results.failure_error() {
            return Err(error.into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowlib::{
        ActivationMode, DeployBackend, DeployStrategy, DeployTarget, DeploymentFixture,
        HostFixture, MockDeployBackend, OperationFixture, StorePath, SystemType,
    };

    #[test]
    fn test_waiting_style_has_a_valid_template() {
        drop(styles::waiting_style());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_success() {
        let deployer = MockDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "test.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_ok());
        assert!(deployer
            .activate(
                &target,
                &store_path,
                ActivationMode::Switch,
                SystemType::NixOS
            )
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_with_failure() {
        let deployer = MockDeployBackend::new().with_failures(vec!["fail.example.com".to_string()]);

        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "fail.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_err());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_from_fixture() {
        let fixture = DeploymentFixture {
            name: "Test deployment".to_string(),
            description: Some("Test scenario".to_string()),
            failures: std::collections::HashMap::new(),
            hosts: vec![HostFixture {
                hostname: "hedgehog.example.com".to_string(),
                metadata: None,
                build: None,
                copy_closure: OperationFixture {
                    success: true,
                    stdout: "copying...\n".to_string(),
                    stderr: "".to_string(),
                    duration_ms: 10,
                },
                activate: OperationFixture {
                    success: true,
                    stdout: "activating...\n".to_string(),
                    stderr: "".to_string(),
                    duration_ms: 5,
                },
            }],
        };

        let deployer = MockDeployBackend::from_fixture(fixture);
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_ok());
        assert!(deployer
            .activate(
                &target,
                &store_path,
                ActivationMode::Switch,
                SystemType::NixOS
            )
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_from_file() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/deploy_scenarios/success.json");

        let deployer = MockDeployBackend::from_file(&fixture_path).unwrap();

        // Test with hedgehog
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_ok());
        assert!(deployer
            .activate(
                &target,
                &store_path,
                ActivationMode::Switch,
                SystemType::NixOS
            )
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_partial_failure() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/deploy_scenarios/partial_failure.json");

        let deployer = MockDeployBackend::from_file(&fixture_path).unwrap();

        // Hedgehog should succeed
        let target_hedgehog = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer
            .copy_closure(&target_hedgehog, &store_path)
            .await
            .is_ok());
        assert!(deployer
            .activate(
                &target_hedgehog,
                &store_path,
                ActivationMode::Switch,
                SystemType::NixOS
            )
            .await
            .is_ok());

        // Nuthatch should fail on activation
        let target_nuthatch = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nuthatch.example.com".to_string(),
            port: None,
        };

        assert!(deployer
            .copy_closure(&target_nuthatch, &store_path)
            .await
            .is_ok());
        assert!(deployer
            .activate(
                &target_nuthatch,
                &store_path,
                ActivationMode::Switch,
                SystemType::NixOS
            )
            .await
            .is_err());
    }

    fn try_deploy_args(flags: &[&str]) -> std::result::Result<DeployArgs, clap::Error> {
        crate::test_utils::try_parse_args(flags)
    }

    fn deploy_args(flags: &[&str]) -> DeployArgs {
        try_deploy_args(flags).unwrap()
    }

    fn rejection(flags: &[&str]) -> clap::error::ErrorKind {
        try_deploy_args(flags).unwrap_err().kind()
    }

    #[test]
    fn test_activation_modes_are_mutually_exclusive() {
        use clap::error::ErrorKind::ArgumentConflict;
        assert_eq!(rejection(&["--dry-run", "--switch"]), ArgumentConflict);
        assert_eq!(rejection(&["--boot", "--test"]), ArgumentConflict);
    }

    #[test]
    fn test_force_requires_switch() {
        assert_eq!(
            rejection(&["--force"]),
            clap::error::ErrorKind::MissingRequiredArgument
        );
        for mode in ["--dry-run", "--boot", "--test"] {
            assert_eq!(
                rejection(&[mode, "--force"]),
                clap::error::ErrorKind::ArgumentConflict
            );
        }
    }

    #[test]
    fn test_force_with_switch_is_accepted() {
        assert!(deploy_args(&["--switch", "--force"]).force);
        assert!(!deploy_args(&[]).force);
    }

    #[test]
    fn test_deploy_strategy_auto() {
        assert_eq!(deploy_args(&[]).deploy_strategy(), DeployStrategy::Auto);
    }

    #[test]
    fn test_deploy_strategy_switch() {
        assert_eq!(
            deploy_args(&["--switch"]).deploy_strategy(),
            DeployStrategy::Switch
        );
    }

    #[test]
    fn test_reboot_timeout_flag() {
        assert_eq!(
            deploy_args(&[]).reboot_timeout,
            snowlib::DEFAULT_REBOOT_MAX_WAIT_SECS
        );
        assert_eq!(deploy_args(&["--reboot-timeout", "45"]).reboot_timeout, 45);
    }

    #[test]
    fn test_reboot_timeout_must_be_positive() {
        assert!(try_deploy_args(&["--reboot-timeout", "0"]).is_err());
    }

    fn redeploy_command(flags: &[&str], flake_path: Option<&str>) -> String {
        deploy_args(flags).redeploy_command(flake_path).join(" ")
    }

    #[test]
    fn test_redeploy_command_of_a_plain_run() {
        assert_eq!(
            redeploy_command(&["--quiet", "--phase", "web", "--concurrency", "5"], None),
            "snow deploy --redeploy"
        );
    }

    #[test]
    fn test_redeploy_command_keeps_the_activation_mode() {
        assert_eq!(
            redeploy_command(&["--boot"], None),
            "snow deploy --redeploy --boot"
        );
        assert_eq!(
            redeploy_command(&["--switch", "--force"], None),
            "snow deploy --redeploy --switch --force"
        );
    }

    #[test]
    fn test_redeploy_command_keeps_how_hosts_are_reached_and_rebooted() {
        assert_eq!(
            redeploy_command(
                &[
                    "--skip-reboot",
                    "--user",
                    "deploy",
                    "--reboot-timeout",
                    "45"
                ],
                None
            ),
            "snow deploy --redeploy --skip-reboot --user deploy --reboot-timeout 45"
        );
    }

    #[test]
    fn test_redeploy_command_keeps_ignoring_phases() {
        assert_eq!(
            redeploy_command(&["--ignore-phases"], None),
            "snow deploy --redeploy --ignore-phases"
        );
    }

    #[test]
    fn test_relative_flake_path_is_made_absolute() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let relative = "tests/../tests/fixtures";
        assert!(
            std::path::Path::new(relative).exists(),
            "tests run in the crate's directory"
        );

        assert_eq!(
            absolute_flake_path(relative),
            std::fs::canonicalize(fixtures)
                .unwrap()
                .display()
                .to_string()
        );
    }

    #[test]
    fn test_missing_flake_path_is_kept_as_given() {
        assert_eq!(absolute_flake_path("no/such flake"), "no/such flake");
    }

    #[test]
    fn test_redeploy_command_keeps_the_flake_path() {
        assert_eq!(
            deploy_args(&["--test"]).redeploy_command(Some("/srv/my flake")),
            [
                "snow",
                "-f",
                "/srv/my flake",
                "deploy",
                "--redeploy",
                "--test"
            ]
        );
    }

    #[test]
    fn test_redeploy_flag() {
        assert!(deploy_args(&["--redeploy", "hedgehog"]).redeploy);
        assert!(!deploy_args(&["hedgehog"]).redeploy);
    }
}
