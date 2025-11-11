use clap::Args;
use snowlib::{
    push_parallel, ActivationMode, DeployTarget, PushOptions, Result, SnowContext, SshDeployer,
};
use tabled::{Table, Tabled};

#[cfg(debug_assertions)]
use snowlib::{FixtureDeployer, MockDeployer};

/// Deploy configurations to hosts
#[derive(Debug, Args)]
pub struct PushArgs {
    /// Hosts to push (one or more)
    pub hosts: Vec<String>,

    /// Dry run - show what would change without activating
    #[arg(long)]
    pub dry_run: bool,

    /// Only set for next boot (don't activate now)
    #[arg(long)]
    pub boot: bool,

    /// Maximum number of hosts to push in parallel
    #[arg(long, default_value = "3")]
    pub concurrency: usize,

    #[cfg(debug_assertions)]
    /// Use mock deployer instead of real SSH (debug builds only)
    #[arg(long, hide = true)]
    pub mock: bool,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock deployer (requires --mock)
    #[arg(long, requires = "mock", hide = true)]
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
    pub fn run(self, ctx: &SnowContext) -> Result<()> {
        // Determine activation mode
        let activation_mode = if self.dry_run {
            ActivationMode::DryActivate
        } else if self.boot {
            ActivationMode::Boot
        } else {
            ActivationMode::Switch
        };

        // Create the appropriate NixOps implementation
        #[cfg(debug_assertions)]
        let nix_ops: Box<dyn snowlib::NixOps> = if self.mock {
            println!("⚠ Using mock deployer");

            if let Some(fixture_path) = &self.mock_fixture {
                let path = std::path::Path::new(fixture_path);
                let fixture = snowlib::DeploymentFixture::from_file(path)?;
                Box::new(snowlib::MockNixOps::from_fixture(fixture))
            } else {
                Box::new(snowlib::MockNixOps::new())
            }
        } else {
            Box::new(snowlib::SystemNixOps::new())
        };

        #[cfg(not(debug_assertions))]
        let nix_ops: Box<dyn snowlib::NixOps> = Box::new(snowlib::SystemNixOps::new());

        // UNIFIED CODE PATH: Build all hosts (same for mock and real)
        println!("→ Building configurations...");

        let mut host_data = Vec::new();
        for host in &self.hosts {
            let metadata = nix_ops.get_host_metadata(ctx, host)?;
            let store_path = nix_ops.build_host(ctx, host)?;

            let target = DeployTarget {
                user: "root".to_string(),
                hostname: metadata.fqdn.clone(),
                port: None,
            };

            host_data.push((metadata, target, store_path));
        }

        println!("✓ Built {} configuration(s)", self.hosts.len());

        // Create deployer (already determined by fixture in nix_ops)
        #[cfg(debug_assertions)]
        let deployer: Box<dyn snowlib::Deployer> = if self.mock {
            if let Some(fixture_path) = &self.mock_fixture {
                let path = std::path::Path::new(fixture_path);
                let fixture = snowlib::DeploymentFixture::from_file(path)?;
                Box::new(FixtureDeployer::from_fixture(fixture))
            } else {
                Box::new(MockDeployer::new())
            }
        } else {
            Box::new(SshDeployer::new())
        };

        #[cfg(not(debug_assertions))]
        let deployer: Box<dyn snowlib::Deployer> = Box::new(SshDeployer::new());

        // UNIFIED CODE PATH: Deploy to all hosts (same for mock and real)
        println!("\n→ Deploying...");

        let opts = PushOptions {
            activation_mode,
            concurrency: self.concurrency,
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
    use snowlib::{
        Deployer, DeploymentFixture, FixtureDeployer, HostFixture, MockDeployer, OperationFixture,
    };

    #[test]
    fn test_mock_deployer_success() {
        let deployer = MockDeployer::new();
        let target = DeployTarget {
            user: "root".to_string(),
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
            user: "root".to_string(),
            hostname: "fail.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_err());
    }

    #[test]
    fn test_fixture_deployer_from_fixture() {
        let fixture = DeploymentFixture {
            name: "Test deployment".to_string(),
            description: Some("Test scenario".to_string()),
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

        let deployer = FixtureDeployer::from_fixture(fixture);
        let target = DeployTarget {
            user: "root".to_string(),
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
    fn test_fixture_deployer_from_file() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/push_scenarios/success.json");

        let deployer = FixtureDeployer::from_file(&fixture_path).unwrap();

        // Test with hydrogen
        let target = DeployTarget {
            user: "root".to_string(),
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
    fn test_fixture_deployer_partial_failure() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/push_scenarios/partial_failure.json");

        let deployer = FixtureDeployer::from_file(&fixture_path).unwrap();

        // Hydrogen should succeed
        let target_hydrogen = DeployTarget {
            user: "root".to_string(),
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
            user: "root".to_string(),
            hostname: "nitrogen.example.com".to_string(),
            port: None,
        };

        assert!(deployer.copy_closure(&target_nitrogen, &store_path).is_ok());
        assert!(deployer
            .activate(&target_nitrogen, &store_path, ActivationMode::Switch)
            .is_err());
    }
}
