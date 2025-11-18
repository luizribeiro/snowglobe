use clap::Args;
use rayon::prelude::*;
use snowlib::{Flake, NixBackend, OutputMode, Result, StorePath};

/// Build NixOS configurations for one or more hosts
#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Hosts to build (defaults to all hosts if not specified)
    pub hosts: Vec<String>,

    /// Quiet mode - suppress subprocess output
    #[arg(short, long)]
    pub quiet: bool,

    /// Maximum number of hosts to build in parallel (must be at least 1)
    #[arg(long, default_value = "3", value_parser = clap::value_parser!(u32).range(1..))]
    pub concurrency: u32,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock backend (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

#[derive(Debug)]
pub struct BuildResult {
    pub hostname: String,
    pub result: Result<StorePath>,
}

impl BuildArgs {
    fn resolve_hosts(&self, flake: &Flake, nix_backend: &dyn NixBackend) -> Result<Vec<String>> {
        let all_hosts = nix_backend.list_all_hosts(flake, None)?;

        if self.hosts.is_empty() {
            Ok(all_hosts.iter().map(|h| h.name.clone()).collect())
        } else {
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

    fn print_results(&self, results: &[BuildResult]) {
        println!();

        let success_count = results.iter().filter(|r| r.result.is_ok()).count();
        let total = results.len();

        for result in results {
            match &result.result {
                Ok(store_path) => {
                    println!("✓ {}: {}", result.hostname, store_path.as_str());
                }
                Err(e) => {
                    println!("✗ {}: {}", result.hostname, e);
                }
            }
        }

        println!();
        if success_count == total {
            println!("Built {}/{} hosts successfully", success_count, total);
        } else {
            println!(
                "Built {}/{} hosts successfully ({} failed)",
                success_count,
                total,
                total - success_count
            );
        }
    }

    pub fn run(self, flake: &Flake) -> Result<()> {
        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        let output_mode = if self.quiet {
            OutputMode::Capture
        } else {
            OutputMode::Inherit
        };

        let hostnames = self.resolve_hosts(flake, nix_backend.as_ref())?;

        // Get metadata for each host to determine system type
        let host_metadata: Vec<_> = hostnames
            .iter()
            .filter_map(|hostname| {
                // Try NixOS first
                if let Ok(metadata) =
                    nix_backend.get_host_metadata(flake, hostname, snowlib::SystemType::NixOS)
                {
                    return Some(metadata);
                }
                // Then try Darwin
                if let Ok(metadata) =
                    nix_backend.get_host_metadata(flake, hostname, snowlib::SystemType::Darwin)
                {
                    return Some(metadata);
                }
                None
            })
            .collect();

        println!(
            "Building {} {} with concurrency {}...",
            host_metadata.len(),
            if host_metadata.len() == 1 {
                "host"
            } else {
                "hosts"
            },
            self.concurrency
        );

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.concurrency as usize)
            .build()
            .map_err(|e| snowlib::error::DeployError::Failed {
                host: "build".to_string(),
                source: std::io::Error::other(format!("Failed to create thread pool: {}", e)),
            })?;

        let results: Vec<BuildResult> = pool.install(|| {
            host_metadata
                .par_iter()
                .map(|metadata| {
                    let result = nix_backend.build_host(
                        flake,
                        &metadata.name,
                        metadata.system_type,
                        output_mode,
                    );
                    BuildResult {
                        hostname: metadata.name.clone(),
                        result,
                    }
                })
                .collect()
        });

        self.print_results(&results);

        let any_failed = results.iter().any(|r| r.result.is_err());
        if any_failed {
            return Err(snowlib::error::DeployError::Failed {
                host: "multiple".to_string(),
                source: std::io::Error::other("One or more builds failed"),
            }
            .into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::snow_cmd;
    use predicates::prelude::*;

    #[test]
    fn test_build_args_defaults() {
        let args = BuildArgs {
            hosts: vec![],
            quiet: false,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.hosts.len(), 0);
        assert!(!args.quiet);
        assert_eq!(args.concurrency, 3);
    }

    #[test]
    fn test_build_args_with_hosts() {
        let args = BuildArgs {
            hosts: vec!["host1".to_string(), "host2".to_string()],
            quiet: true,
            concurrency: 5,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.hosts.len(), 2);
        assert_eq!(args.hosts[0], "host1");
        assert_eq!(args.hosts[1], "host2");
        assert!(args.quiet);
        assert_eq!(args.concurrency, 5);
    }

    #[test]
    fn test_build_invalid_host() {
        snow_cmd()
            .arg("build")
            .arg("nonexistent")
            .assert()
            .failure()
            .stderr(predicate::str::contains("Invalid host(s)"))
            .stderr(predicate::str::contains("nonexistent"));
    }
}
