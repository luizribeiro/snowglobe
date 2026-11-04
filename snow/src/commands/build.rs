use crate::error_summary::error_summary;
use crate::marks;
use crate::plural::plural;
use clap::Args;
use console::style;
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

/// Each host's outcome and the totals, worded like push's report.
/// A failure shows only its cause: the build's output was already shown
/// unless `--quiet` asked to hide it.
fn report(results: &[BuildResult]) -> String {
    let mut lines: Vec<String> = results.iter().map(host_line).collect();
    lines.push(String::new());
    lines.push(totals(results));
    lines.join("\n")
}

fn host_line(result: &BuildResult) -> String {
    match &result.result {
        Ok(store_path) => format!(
            "  {} {} {}",
            marks::success(),
            result.hostname,
            style(store_path.as_str()).dim()
        ),
        Err(error) => format!(
            "  {} {}: {}",
            marks::failure(),
            result.hostname,
            error_summary(&error.to_string())
        ),
    }
}

fn totals(results: &[BuildResult]) -> String {
    let total = results.len();
    let failed = results.iter().filter(|r| r.result.is_err()).count();
    match (failed, total) {
        (0, 1) => style("✓ 1 host built").green().bold().to_string(),
        (0, _) => style(format!("✓ All {total} hosts built"))
            .green()
            .bold()
            .to_string(),
        _ => style(format!(
            "✗ {}/{total} built, {failed} failed",
            total - failed
        ))
        .red()
        .bold()
        .to_string(),
    }
}

impl BuildArgs {
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
            plural(host_metadata.len(), "host", "hosts"),
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

        if !results.is_empty() {
            println!("\n{}", report(&results));
        }

        match failure_error(&results) {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
}

fn failure_error(results: &[BuildResult]) -> Option<snowlib::error::DeployError> {
    let failed: Vec<String> = results
        .iter()
        .filter(|r| r.result.is_err())
        .map(|r| format!("{} ({})", r.hostname, snowlib::PipelineStep::Build))
        .collect();
    if failed.is_empty() {
        return None;
    }
    Some(snowlib::error::DeployError::HostsFailed {
        total: results.len(),
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::snow_cmd;
    use predicates::prelude::*;

    fn build_result(hostname: &str, ok: bool) -> BuildResult {
        BuildResult {
            hostname: hostname.to_string(),
            result: if ok {
                Ok(StorePath::new(format!(
                    "/nix/store/3xv9d1x5qzl1-nixos-system-{hostname}-26.05"
                )))
            } else {
                Err(snowlib::error::NixError::BuildFailed {
                    host: hostname.to_string(),
                    stderr: "error: builder for wine-wow-staging-10.12 failed".to_string(),
                }
                .into())
            },
        }
    }

    fn plain_report(results: &[BuildResult]) -> String {
        console::strip_ansi_codes(&report(results)).to_string()
    }

    #[test]
    fn test_report_shows_each_failure_by_its_cause() {
        let results = vec![
            build_result("boron", true),
            build_result("radon", false),
            build_result("sodium", true),
        ];

        assert_eq!(
            plain_report(&results),
            "  ✓ boron /nix/store/3xv9d1x5qzl1-nixos-system-boron-26.05\n\
             \x20 ✗ radon: builder for wine-wow-staging-10.12 failed\n\
             \x20 ✓ sodium /nix/store/3xv9d1x5qzl1-nixos-system-sodium-26.05\n\
             \n\
             ✗ 2/3 built, 1 failed"
        );
    }

    #[test]
    fn test_report_totals_when_every_build_succeeds() {
        let one = vec![build_result("boron", true)];
        let two = vec![build_result("boron", true), build_result("sodium", true)];

        assert!(plain_report(&one).ends_with("\n\n✓ 1 host built"));
        assert!(plain_report(&two).ends_with("\n\n✓ All 2 hosts built"));
    }

    #[test]
    fn test_failure_error_names_failed_builds() {
        let results = vec![
            build_result("boron", true),
            build_result("radon", false),
            build_result("sodium", true),
        ];

        assert_eq!(
            failure_error(&results).unwrap().to_string(),
            "1 of 3 hosts failed: radon (build)"
        );
    }

    #[test]
    fn test_failure_error_lists_every_failed_build() {
        let results = vec![
            build_result("boron", false),
            build_result("radon", false),
            build_result("sodium", true),
        ];

        assert_eq!(
            failure_error(&results).unwrap().to_string(),
            "2 of 3 hosts failed: boron (build), radon (build)"
        );
    }

    #[test]
    fn test_no_failure_error_when_all_builds_succeed() {
        let results = vec![build_result("boron", true), build_result("sodium", true)];

        assert!(failure_error(&results).is_none());
    }

    #[test]
    fn test_build_invalid_host() {
        snow_cmd()
            .arg("build")
            .arg("nonexistent")
            .assert()
            .failure()
            .stderr(predicate::str::contains("Unknown host: nonexistent"))
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
            .stderr(predicate::str::contains("Unknown host: nonexistent"))
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
            .stdout(predicate::str::contains("Building 0 host"))
            .stdout(predicate::str::contains("built").not());
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
