use clap::Args;
use rayon::prelude::*;
use snowlib::{Flake, NixBackend, OutputMode, Result, StorePath};

/// Build NixOS configurations for one or more hosts
#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Hosts to build (defaults to all hosts if not specified)
    pub hosts: Vec<String>,

    /// Filter hosts using a Nix expression (e.g., "config: config.services.nginx.enable or false")
    #[arg(short = 'e', long, conflicts_with_all = &["phase", "phases", "tag", "tags"])]
    pub filter: Option<String>,

    /// Filter by deployment phase (e.g., "production")
    #[arg(long, conflicts_with_all = &["filter", "phases", "tag", "tags"])]
    pub phase: Option<String>,

    /// Filter by multiple phases (comma-separated, OR logic)
    #[arg(long, conflicts_with_all = &["filter", "phase", "tag", "tags"], value_delimiter = ',')]
    pub phases: Vec<String>,

    /// Filter by tag (e.g., "web")
    #[arg(long, conflicts_with_all = &["filter", "phase", "phases", "tags"])]
    pub tag: Option<String>,

    /// Filter by multiple tags (comma-separated, OR logic)
    #[arg(long, conflicts_with_all = &["filter", "phase", "phases", "tag"], value_delimiter = ',')]
    pub tags: Vec<String>,

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
    fn get_filter_expression(&self) -> Result<Option<String>> {
        crate::filter_helpers::build_filter_expression(
            self.filter.as_deref(),
            self.phase.as_deref(),
            &self.phases,
            self.tag.as_deref(),
            &self.tags,
        )
    }

    fn resolve_hosts(&self, flake: &Flake, nix_backend: &dyn NixBackend) -> Result<Vec<String>> {
        crate::host_selection::resolve_hosts_from_args(
            flake,
            nix_backend,
            self.get_filter_expression()?.as_deref(),
            &self.hosts,
        )
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
            filter: None,
            phase: None,
            phases: vec![],
            tag: None,
            tags: vec![],
            quiet: false,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.hosts.len(), 0);
        assert!(args.filter.is_none());
        assert!(args.phase.is_none());
        assert!(args.phases.is_empty());
        assert!(args.tag.is_none());
        assert!(args.tags.is_empty());
        assert!(!args.quiet);
        assert_eq!(args.concurrency, 3);
    }

    #[test]
    fn test_build_args_with_hosts() {
        let args = BuildArgs {
            hosts: vec!["host1".to_string(), "host2".to_string()],
            filter: None,
            phase: None,
            phases: vec![],
            tag: None,
            tags: vec![],
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

    #[test]
    fn test_build_with_filter_and_nonexistent_host() {
        snow_cmd()
            .arg("build")
            .arg("-e")
            .arg("config: config.services.postgresql.enable or false")
            .arg("nonexistent")
            .assert()
            .failure()
            .stderr(predicate::str::contains("Invalid host(s)"))
            .stderr(predicate::str::contains("nonexistent"));
    }

    #[test]
    fn test_build_with_invalid_filter() {
        snow_cmd()
            .arg("build")
            .arg("-e")
            .arg("config: { invalid syntax")
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "Failed to evaluate Nix expression",
            ));
    }

    #[test]
    fn test_build_intersection_filter_and_explicit_hosts() {
        // Test that combining filter with explicit hosts returns intersection
        // hydrogen and oxygen have postgresql, nitrogen does not
        snow_cmd()
            .arg("build")
            .arg("-e")
            .arg("config: config.services.postgresql.enable or false")
            .arg("hydrogen")
            .arg("nitrogen")
            .assert()
            .failure() // Build will fail (unbuildable test flake), but we can check the host selection
            .stdout(predicate::str::contains("Building 1 host")); // Only hydrogen matches both filter AND explicit list
    }

    #[test]
    fn test_build_empty_intersection() {
        // Test that when no hosts match both filter and explicit list, we get 0 hosts
        // helium doesn't have postgresql
        snow_cmd()
            .arg("build")
            .arg("-e")
            .arg("config: config.services.postgresql.enable or false")
            .arg("helium")
            .assert()
            .success() // 0 hosts to build = success
            .stdout(predicate::str::contains("Building 0 host"));
    }

    #[test]
    fn test_build_with_phase_production() {
        // hydrogen and nitrogen have phase=production
        snow_cmd()
            .arg("build")
            .arg("--phase")
            .arg("production")
            .assert()
            .failure() // Build will fail (unbuildable flake)
            .stdout(predicate::str::contains("Building 2 host"));
    }

    #[test]
    fn test_build_with_phase_staging() {
        // Only oxygen has phase=staging
        snow_cmd()
            .arg("build")
            .arg("--phase")
            .arg("staging")
            .assert()
            .failure()
            .stdout(predicate::str::contains("Building 1 host"));
    }

    #[test]
    fn test_build_with_multiple_phases() {
        // production (hydrogen, nitrogen) + staging (oxygen) = 3 hosts
        snow_cmd()
            .arg("build")
            .arg("--phases")
            .arg("production,staging")
            .assert()
            .failure()
            .stdout(predicate::str::contains("Building 3 host"));
    }

    #[test]
    fn test_build_with_tag_web() {
        // hydrogen and oxygen have tag=web
        snow_cmd()
            .arg("build")
            .arg("--tag")
            .arg("web")
            .assert()
            .failure()
            .stdout(predicate::str::contains("Building 2 host"));
    }

    #[test]
    fn test_build_with_multiple_tags() {
        // web (hydrogen, oxygen) + database (nitrogen) = 3 hosts
        snow_cmd()
            .arg("build")
            .arg("--tags")
            .arg("web,database")
            .assert()
            .failure()
            .stdout(predicate::str::contains("Building 3 host"));
    }

    #[test]
    fn test_build_phase_with_explicit_hosts() {
        // Test intersection: --phase production + explicit hosts (hydrogen, helium)
        // Only hydrogen matches both (helium is dev phase)
        snow_cmd()
            .arg("build")
            .arg("--phase")
            .arg("production")
            .arg("hydrogen")
            .arg("helium")
            .assert()
            .failure()
            .stdout(predicate::str::contains("Building 1 host"));
    }

    #[test]
    fn test_build_tag_with_explicit_hosts() {
        // Test intersection: --tag web + explicit hosts (hydrogen, nitrogen, helium)
        // Only hydrogen matches both (nitrogen has database tag, helium has test tag)
        snow_cmd()
            .arg("build")
            .arg("--tag")
            .arg("web")
            .arg("hydrogen")
            .arg("nitrogen")
            .arg("helium")
            .assert()
            .failure()
            .stdout(predicate::str::contains("Building 1 host"));
    }
}
