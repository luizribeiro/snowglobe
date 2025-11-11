use clap::Args;
use snowlib::{
    push_parallel, ActivationMode, DeployTarget, PushOptions, Result, SnowContext, SshDeployer,
};
use tabled::{Table, Tabled};

#[cfg(debug_assertions)]
use snowlib::MockDeployer;

/// Deploy configurations to hosts
#[derive(Debug, Args)]
pub struct PushArgs {
    /// Hosts to push (defaults to all hosts if not specified)
    pub hosts: Vec<String>,

    /// Dry run - show what would change without activating
    #[arg(long)]
    pub dry_run: bool,

    /// Only set for next boot (don't activate now)
    #[arg(long)]
    pub boot: bool,

    /// SSH user to use for deployment (defaults to SSH config or current user)
    #[arg(long)]
    pub user: Option<String>,

    /// Maximum number of hosts to push in parallel (must be at least 1)
    #[arg(long, default_value = "3", value_parser = clap::value_parser!(u32).range(1..))]
    pub concurrency: u32,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock deployer (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

#[derive(Debug, Tabled)]
struct DeploymentResult {
    #[tabled(rename = "Host")]
    host: String,

    #[tabled(rename = "Status")]
    status: String,

    #[tabled(rename = "Message")]
    message: String,
}

impl PushArgs {
    /// Resolve and validate the hosts to push
    ///
    /// If no hosts are provided, returns all hosts from the flake.
    /// If hosts are provided, validates they exist in the flake.
    fn resolve_hosts(&self, ctx: &SnowContext) -> Result<Vec<String>> {
        let all_hosts = snowlib::list_hosts(ctx, None)?;

        if self.hosts.is_empty() {
            // No hosts specified, push all hosts
            Ok(all_hosts.iter().map(|h| h.name.clone()).collect())
        } else {
            // Validate that all provided hosts exist in the flake
            let valid_hosts: std::collections::HashSet<String> =
                all_hosts.iter().map(|h| h.name.clone()).collect();

            let invalid_hosts: Vec<String> = self
                .hosts
                .iter()
                .filter(|h| !valid_hosts.contains(*h))
                .cloned()
                .collect();

            if !invalid_hosts.is_empty() {
                return Err(snowlib::error::FlakeError::InvalidHosts {
                    invalid_hosts: invalid_hosts.join(", "),
                    available_hosts: valid_hosts.iter().cloned().collect::<Vec<_>>().join(", "),
                }
                .into());
            }

            Ok(self.hosts.clone())
        }
    }

    /// Create the appropriate NixOps implementation based on settings
    fn get_nix_ops(&self) -> Result<Box<dyn snowlib::NixOps>> {
        #[cfg(debug_assertions)]
        {
            if let Some(fixture_path) = &self.mock_fixture {
                println!("⚠ Using mock deployer");
                let path = std::path::Path::new(fixture_path);
                let fixture = snowlib::DeploymentFixture::from_file(path)?;
                return Ok(Box::new(snowlib::MockNixOps::from_fixture(fixture)));
            }
        }

        Ok(Box::new(snowlib::SystemNixOps::new()))
    }

    /// Create the appropriate Deployer implementation based on settings
    fn get_deployer(&self) -> Result<Box<dyn snowlib::Deployer>> {
        #[cfg(debug_assertions)]
        {
            if let Some(fixture_path) = &self.mock_fixture {
                let path = std::path::Path::new(fixture_path);
                return Ok(Box::new(MockDeployer::from_file(path)?));
            }
        }

        Ok(Box::new(SshDeployer::new()))
    }

    pub fn run(self, ctx: &SnowContext) -> Result<()> {
        // When using --mock-fixture, use the mock flake instead of the real one
        #[cfg(debug_assertions)]
        let mock_ctx;
        #[cfg(debug_assertions)]
        let ctx = if self.mock_fixture.is_some() {
            // Use the mock flake bundled with the test fixtures
            let mock_flake_path =
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
            mock_ctx = SnowContext::new(Some(mock_flake_path.to_str().unwrap()))?;
            &mock_ctx
        } else {
            ctx
        };

        // Determine activation mode
        let activation_mode = if self.dry_run {
            ActivationMode::DryActivate
        } else if self.boot {
            ActivationMode::Boot
        } else {
            ActivationMode::Switch
        };

        let nix_ops = self.get_nix_ops()?;
        let hosts = self.resolve_hosts(ctx)?;

        // UNIFIED CODE PATH: Build all hosts (same for mock and real)
        println!("→ Building configurations...");

        let mut host_data = Vec::new();
        for host in &hosts {
            let metadata = nix_ops.get_host_metadata(ctx, host)?;
            let store_path = nix_ops.build_host(ctx, host)?;

            let target = DeployTarget {
                user: self.user.clone(),
                hostname: metadata.fqdn.clone(),
                port: None,
            };

            host_data.push((metadata, target, store_path));
        }

        println!("✓ Built {} configuration(s)", hosts.len());

        let deployer = self.get_deployer()?;

        // UNIFIED CODE PATH: Deploy to all hosts (same for mock and real)
        println!("\n→ Deploying...");

        let opts = PushOptions {
            activation_mode,
            concurrency: self.concurrency as usize,
        };

        let results = push_parallel(deployer.as_ref(), host_data, &opts)?;

        // Format results
        let table_data: Vec<DeploymentResult> = results
            .results
            .iter()
            .map(|(host, result)| DeploymentResult {
                host: host.clone(),
                status: if result.success { "✓" } else { "✗" }.to_string(),
                message: result.message.clone(),
            })
            .collect();

        println!("\nPush Results\n");
        let table = Table::new(&table_data).to_string();
        println!("{}", table);

        // Summary
        println!(
            "\n{}/{} hosts deployed successfully",
            results.success_count(),
            results.results.len()
        );

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
    use snowlib::{DeployTarget, StorePath};
    use snowlib::{Deployer, DeploymentFixture, HostFixture, MockDeployer, OperationFixture};

    #[test]
    fn test_mock_deployer_success() {
        let deployer = MockDeployer::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "test.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .is_ok());
    }

    #[test]
    fn test_mock_deployer_with_failure() {
        let deployer = MockDeployer::new().with_failures(vec!["fail.example.com".to_string()]);

        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "fail.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_err());
    }

    #[test]
    fn test_mock_deployer_from_fixture() {
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

        let deployer = MockDeployer::from_fixture(fixture);
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .is_ok());
    }

    #[test]
    fn test_mock_deployer_from_file() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/push_scenarios/success.json");

        let deployer = MockDeployer::from_file(&fixture_path).unwrap();

        // Test with hydrogen
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .is_ok());
    }

    #[test]
    fn test_mock_deployer_partial_failure() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/push_scenarios/partial_failure.json");

        let deployer = MockDeployer::from_file(&fixture_path).unwrap();

        // Hydrogen should succeed
        let target_hydrogen = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target_hydrogen, &store_path).is_ok());
        assert!(deployer
            .activate(&target_hydrogen, &store_path, ActivationMode::Switch)
            .is_ok());

        // Nitrogen should fail on activation
        let target_nitrogen = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nitrogen.example.com".to_string(),
            port: None,
        };

        assert!(deployer.copy_closure(&target_nitrogen, &store_path).is_ok());
        assert!(deployer
            .activate(&target_nitrogen, &store_path, ActivationMode::Switch)
            .is_err());
    }
}
