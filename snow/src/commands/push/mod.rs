mod colors;
mod observer;
mod styles;
mod summary;

use clap::Args;
use snowlib::{Flake, PushStrategy, Result};

/// Deploy configurations to hosts
#[derive(Debug, Args)]
pub struct PushArgs {
    #[command(flatten)]
    pub selection: crate::host_selection::HostSelectionArgs,

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

    /// Deploy hosts even if they already run the built system
    ///
    /// Use this to retry a host whose last activation partly failed: NixOS
    /// records the new system as current before starting services.
    #[arg(long)]
    pub redeploy: bool,

    /// Override policies and safety checks (use with --switch to ignore reboot policies and detection)
    #[arg(long)]
    pub force: bool,

    /// Quiet mode - suppress subprocess output, only show summary
    #[arg(short, long)]
    pub quiet: bool,

    /// SSH user to use for deployment (defaults to SSH config or current user)
    #[arg(long)]
    pub user: Option<String>,

    /// Maximum number of hosts to push in parallel (must be at least 1)
    #[arg(long, default_value = "3", value_parser = clap::value_parser!(u32).range(1..))]
    pub concurrency: u32,

    /// Ignore push phases and deploy all hosts in parallel
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

impl PushArgs {
    /// Validate flag combinations
    fn validate(&self) -> Result<()> {
        // Check for mutually exclusive activation modes
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

        // --force only makes sense with --switch
        if self.force && !self.switch {
            return Err(snowlib::error::DeployError::Failed {
                host: "validation".to_string(),
                source: std::io::Error::other("--force can only be used with --switch"),
            }
            .into());
        }

        Ok(())
    }

    /// Determine the push strategy from flags
    fn push_strategy(&self) -> PushStrategy {
        if self.dry_run {
            PushStrategy::DryRun
        } else if self.test {
            PushStrategy::Test
        } else if self.boot {
            PushStrategy::Boot
        } else if self.switch {
            PushStrategy::Switch
        } else {
            PushStrategy::Auto
        }
    }

    /// Main entry point for the push command
    ///
    /// Orchestrates the entire deployment process by delegating to snowlib's `push()` function.
    /// The CLI's only responsibilities are:
    /// - Parse command-line arguments
    /// - Create backend implementations and observer
    /// - Call snowlib::push()
    /// - Print results table
    pub async fn run(self, flake: &Flake) -> Result<()> {
        self.validate()?;

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

        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
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
        let reboot_backend = crate::backend_factory::create_reboot_backend(self.user.as_deref())?;
        let observer = observer::CliObserver::new();

        let hostnames = self.selection.resolve(flake, nix_backend.as_ref()).await?;
        let hosts = nix_backend.load_host_metadata(flake, &hostnames).await?;

        let config = snowlib::PushConfiguration {
            strategy: self.push_strategy(),
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

        let push_ctx = snowlib::PushContext::new(
            flake,
            nix_backend.as_ref(),
            deploy_backend.as_ref(),
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );

        // Execute the push - snowlib handles everything!
        let results = snowlib::push(&push_ctx, hosts).await?;

        summary::print_summary(&results);

        if !results.all_succeeded() {
            return Err(snowlib::error::DeployError::Failed {
                host: "multiple".to_string(),
                source: std::io::Error::other("Some deployments failed"),
            }
            .into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowlib::{
        ActivationMode, DeployBackend, DeployTarget, DeploymentFixture, HostFixture,
        MockDeployBackend, OperationFixture, PushStrategy, StorePath,
    };

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
            .activate(&target, &store_path, ActivationMode::Switch)
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
                hostname: "hydrogen.example.com".to_string(),
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
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_from_file() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/push_scenarios/success.json");

        let deployer = MockDeployBackend::from_file(&fixture_path).unwrap();

        // Test with hydrogen
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_partial_failure() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/push_scenarios/partial_failure.json");

        let deployer = MockDeployBackend::from_file(&fixture_path).unwrap();

        // Hydrogen should succeed
        let target_hydrogen = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer
            .copy_closure(&target_hydrogen, &store_path)
            .await
            .is_ok());
        assert!(deployer
            .activate(&target_hydrogen, &store_path, ActivationMode::Switch)
            .await
            .is_ok());

        // Nitrogen should fail on activation
        let target_nitrogen = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nitrogen.example.com".to_string(),
            port: None,
        };

        assert!(deployer
            .copy_closure(&target_nitrogen, &store_path)
            .await
            .is_ok());
        assert!(deployer
            .activate(&target_nitrogen, &store_path, ActivationMode::Switch)
            .await
            .is_err());
    }

    fn push_args(flags: &[&str]) -> PushArgs {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(flatten)]
            args: PushArgs,
        }
        <Cli as clap::Parser>::parse_from(std::iter::once("push").chain(flags.iter().copied())).args
    }

    #[test]
    fn test_validate_mutually_exclusive_modes() {
        assert!(push_args(&["--dry-run", "--switch"]).validate().is_err());
    }

    #[test]
    fn test_validate_force_without_switch() {
        assert!(push_args(&["--force"]).validate().is_err());
    }

    #[test]
    fn test_validate_valid_combination() {
        assert!(push_args(&["--switch", "--force"]).validate().is_ok());
        assert!(push_args(&[]).validate().is_ok());
    }

    #[test]
    fn test_push_strategy_auto() {
        assert_eq!(push_args(&[]).push_strategy(), PushStrategy::Auto);
    }

    #[test]
    fn test_push_strategy_switch() {
        assert_eq!(
            push_args(&["--switch"]).push_strategy(),
            PushStrategy::Switch
        );
    }

    #[test]
    fn test_redeploy_flag() {
        assert!(push_args(&["--redeploy", "hydrogen"]).redeploy);
        assert!(!push_args(&["hydrogen"]).redeploy);
    }
}
