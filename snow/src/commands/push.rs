use clap::Args;
use snowlib::{
    build_host, get_host_metadata, push_parallel, ActivationMode, DeployTarget, PushOptions,
    Result, SnowContext, SshDeployer, StorePath,
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

        // Load fixture if using mock mode with fixture file
        #[cfg(debug_assertions)]
        let fixture = if self.mock && self.mock_fixture.is_some() {
            let path = std::path::Path::new(self.mock_fixture.as_ref().unwrap());
            Some(snowlib::DeploymentFixture::from_file(path)?)
        } else {
            None
        };

        // Skip building when using mock mode
        #[cfg(debug_assertions)]
        let host_data = if self.mock {
            println!("⚠ Using mock deployer");

            // Process each host
            let mut results = Vec::new();
            for host in &self.hosts {
                // Get metadata from fixture or create fake
                let metadata = if let Some(ref fix) = fixture {
                    if let Some(meta_fix) = fix.get_metadata(host) {
                        snowlib::HostMetadata {
                            name: host.clone(),
                            phase: meta_fix.phase.clone(),
                            tags: meta_fix.tags.clone(),
                            fqdn: meta_fix.fqdn.clone(),
                        }
                    } else {
                        snowlib::HostMetadata {
                            name: host.clone(),
                            phase: Some("production".to_string()),
                            tags: vec![],
                            fqdn: format!("{}.example.com", host),
                        }
                    }
                } else {
                    snowlib::HostMetadata {
                        name: host.clone(),
                        phase: Some("production".to_string()),
                        tags: vec![],
                        fqdn: format!("{}.example.com", host),
                    }
                };

                // Simulate build phase
                if let Some(ref fix) = fixture {
                    if let Some(build_fix) = fix.get_build(host) {
                        use std::thread;
                        use std::time::Duration;

                        print!("Building {}... ", host);
                        thread::sleep(Duration::from_millis(build_fix.duration_ms));

                        if !build_fix.stdout.is_empty() {
                            print!("{}", build_fix.stdout);
                        }
                        if !build_fix.stderr.is_empty() {
                            eprint!("{}", build_fix.stderr);
                        }

                        if !build_fix.success {
                            println!("✗ Build failed");
                            return Err(snowlib::error::NixError::BuildFailed {
                                host: host.clone(),
                                stderr: build_fix.stderr.clone(),
                                source: std::io::Error::other("Build failed (from fixture)"),
                            }
                            .into());
                        }
                        println!("✓");
                    }
                }

                // Get store path from fixture or create fake
                let store_path = if let Some(ref fix) = fixture {
                    if let Some(build_fix) = fix.get_build(host) {
                        if let Some(ref sp) = build_fix.store_path {
                            StorePath::new(sp.clone())
                        } else {
                            StorePath::new(format!("/nix/store/mock-{}", host))
                        }
                    } else {
                        StorePath::new(format!("/nix/store/mock-{}", host))
                    }
                } else {
                    StorePath::new(format!("/nix/store/mock-{}", host))
                };

                let target = DeployTarget {
                    user: "root".to_string(),
                    hostname: metadata.fqdn.clone(),
                    port: None,
                };

                results.push((metadata, target, store_path));
            }
            results
        } else {
            println!("→ Building configurations...");

            // Build all hosts and gather metadata
            // TODO: Build all hosts in parallel with single `nix build` command for better performance
            let mut host_data = Vec::new();

            for host in &self.hosts {
                let metadata = get_host_metadata(ctx, host)?;

                let store_path = build_host(ctx, host)?;

                let target = DeployTarget {
                    user: "root".to_string(),
                    hostname: metadata.fqdn.clone(),
                    port: None,
                };

                host_data.push((metadata, target, store_path));
            }

            println!("✓ Built {} configuration(s)", self.hosts.len());
            host_data
        };

        #[cfg(not(debug_assertions))]
        let host_data = {
            println!("→ Building configurations...");

            // Build all hosts and gather metadata
            // TODO: Build all hosts in parallel with single `nix build` command for better performance
            let mut host_data = Vec::new();

            for host in &self.hosts {
                let metadata = get_host_metadata(ctx, host)?;

                let store_path = build_host(ctx, host)?;

                let target = DeployTarget {
                    user: "root".to_string(),
                    hostname: metadata.fqdn.clone(),
                    port: None,
                };

                host_data.push((metadata, target, store_path));
            }

            println!("✓ Built {} configuration(s)", self.hosts.len());
            host_data
        };

        // Deploy
        println!("\n→ Deploying...");

        let opts = PushOptions {
            activation_mode,
            concurrency: self.concurrency,
        };

        #[cfg(debug_assertions)]
        let results = if self.mock {
            // Create appropriate mock deployer
            let deployer: Box<dyn snowlib::Deployer> = if let Some(fix) = fixture {
                Box::new(FixtureDeployer::from_fixture(fix))
            } else {
                Box::new(MockDeployer::new())
            };
            push_parallel(deployer.as_ref(), host_data, &opts)?
        } else {
            let deployer = SshDeployer::new();
            push_parallel(&deployer, host_data, &opts)?
        };

        #[cfg(not(debug_assertions))]
        let results = {
            let deployer = SshDeployer::new();
            push_parallel(&deployer, host_data, &opts)?
        };

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
