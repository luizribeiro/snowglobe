use clap::Args;
use futures::stream::{FuturesUnordered, StreamExt};
use snowlib::{Flake, NixBackend, OutputMode, Result, StorePath};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Build NixOS configurations for one or more hosts
#[derive(Debug, Args)]
pub struct BuildArgs {
    #[command(flatten)]
    pub selection: crate::host_selection::HostSelectionArgs,

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

    pub async fn run(self, flake: &Flake) -> Result<()> {
        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &self.mock_fixture,
        )?;

        let output_mode = if self.quiet {
            OutputMode::Capture
        } else {
            OutputMode::Inherit
        };

        let hostnames = self.selection.resolve(flake, nix_backend.as_ref()).await?;

        let host_metadata = nix_backend.load_host_metadata(flake, &hostnames).await?;

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

        // Use semaphore for concurrency control
        let semaphore = Arc::new(Semaphore::new(self.concurrency as usize));
        let nix_backend: Arc<dyn NixBackend> = nix_backend.into();
        let mut futures = FuturesUnordered::new();

        for metadata in host_metadata {
            let sem = Arc::clone(&semaphore);
            let nix_backend = Arc::clone(&nix_backend);
            let hostname = metadata.name.clone();
            let system_type = metadata.system_type;
            let flake = flake.clone();

            futures.push(async move {
                let _permit = sem.acquire().await.expect("semaphore closed");
                let result = nix_backend
                    .build_host(&flake, &hostname, system_type, output_mode)
                    .await;
                BuildResult { hostname, result }
            });
        }

        let mut results = Vec::new();
        while let Some(result) = futures.next().await {
            results.push(result);
        }

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
    use crate::test_utils::snow_cmd;
    use predicates::prelude::*;

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
