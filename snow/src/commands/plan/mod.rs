mod render;

use std::collections::HashSet;

use clap::Args;
use snowlib::plan::{create_plan, PlanReport, PlanTarget};
use snowlib::{Flake, NixBackend, Result, SystemPlanBackend, SystemType};

use crate::host_selection::HostSelectionArgs;

const EXIT_EVAL_FAILED: i32 = 1;
const EXIT_WOULD_BUILD: i32 = 2;

/// Show what building hosts would build from source versus fetch, without building
#[derive(Debug, Args)]
pub struct PlanArgs {
    #[command(flatten)]
    pub selection: HostSelectionArgs,

    /// Print the plan as JSON
    #[arg(long)]
    pub json: bool,

    /// Also list trivial local builds (config files, units, wrappers)
    #[arg(long)]
    pub all: bool,

    /// Exit with status 2 if any non-trivial derivation would be built
    #[arg(long)]
    pub fail_on_build: bool,

    /// Parallel nix-eval-jobs workers (defaults to the number of hosts, at most 4)
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub eval_workers: Option<u32>,

    /// Memory limit per evaluation worker, in MiB
    #[arg(long, default_value = "4096", value_parser = clap::value_parser!(u32).range(256..))]
    pub eval_max_memory: u32,
}

impl PlanArgs {
    pub async fn run(self, flake: &Flake) -> Result<()> {
        let nix_backend = crate::backend_factory::create_nix_backend(
            #[cfg(debug_assertions)]
            &None,
        )?;
        let hosts = self.selection.resolve(flake, nix_backend.as_ref()).await?;
        let targets = plan_targets(flake, nix_backend.as_ref(), hosts).await;

        let workers = self
            .eval_workers
            .unwrap_or_else(|| (targets.len() as u32).clamp(1, 4));
        let backend = SystemPlanBackend::new(workers, self.eval_max_memory);
        let report = create_plan(flake, &backend, &targets).await?;

        self.print(&report);
        // Commands only signal success or failure through Result; --fail-on-build needs exit code 2.
        crate::cursor::exit(self.exit_code(&report));
    }

    fn print(&self, report: &PlanReport) {
        if self.json {
            let json = serde_json::to_string_pretty(report).expect("plan report serializes");
            println!("{json}");
        } else {
            print!("{}", render::render(report, self.all));
        }
    }

    fn exit_code(&self, report: &PlanReport) -> i32 {
        if report.has_eval_failures() {
            EXIT_EVAL_FAILED
        } else if self.fail_on_build && report.has_nontrivial_builds() {
            EXIT_WOULD_BUILD
        } else {
            0
        }
    }
}

async fn plan_targets(
    flake: &Flake,
    nix_backend: &dyn NixBackend,
    hosts: Vec<String>,
) -> Vec<PlanTarget> {
    let darwin: HashSet<String> = match nix_backend
        .list_hosts(flake, SystemType::Darwin, None)
        .await
    {
        Ok(hosts) => hosts.into_iter().map(|h| h.name).collect(),
        Err(e) => {
            if !e.to_string().contains("does not provide attribute") {
                tracing::warn!("Could not list darwin hosts, planning all as NixOS: {e}");
            }
            HashSet::new()
        }
    };

    hosts
        .into_iter()
        .map(|host| PlanTarget {
            system_type: if darwin.contains(&host) {
                SystemType::Darwin
            } else {
                SystemType::NixOS
            },
            host,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::snow_cmd;
    use predicates::prelude::*;
    use snowlib::plan::build_report;

    fn args(fail_on_build: bool) -> PlanArgs {
        PlanArgs {
            selection: Default::default(),
            json: false,
            all: false,
            fail_on_build,
            eval_workers: None,
            eval_max_memory: 4096,
        }
    }

    fn report(lines: &str) -> PlanReport {
        let evals = snowlib::plan::parse_eval_jobs(lines.as_bytes()).unwrap();
        build_report(&evals, &[], &[])
    }

    const RAVEN_BUILDS_WINE: &str = r#"{"attr":"raven","drvPath":"/nix/store/aaa-nixos-system-raven.drv","system":"x86_64-linux","cacheStatus":"cached","neededBuilds":["/nix/store/bbb-wine-wow-staging-10.12.drv"],"neededSubstitutes":[]}"#;
    const KESTREL_FAILS: &str =
        r#"{"attr":"kestrel","error":"error: attribute 'doesNotExist' missing"}"#;

    #[test]
    fn test_exit_code_fails_on_build_only_when_asked() {
        let report = report(RAVEN_BUILDS_WINE);

        assert_eq!(args(false).exit_code(&report), 0);
        assert_eq!(args(true).exit_code(&report), EXIT_WOULD_BUILD);
    }

    #[test]
    fn test_exit_code_for_eval_failure_wins() {
        let report = report(&format!("{RAVEN_BUILDS_WINE}\n{KESTREL_FAILS}"));

        assert_eq!(args(true).exit_code(&report), EXIT_EVAL_FAILED);
    }

    #[test]
    fn test_exit_code_when_nothing_to_build() {
        let report = report(
            r#"{"attr":"bravo","drvPath":"/nix/store/ccc-nixos-system-bravo.drv","system":"x86_64-linux","cacheStatus":"cached","neededBuilds":[],"neededSubstitutes":["/nix/store/ddd-glibc"]}"#,
        );

        assert_eq!(args(true).exit_code(&report), 0);
    }

    #[test]
    fn test_plan_invalid_host() {
        snow_cmd()
            .arg("plan")
            .arg("nonexistent")
            .assert()
            .failure()
            .stderr(predicate::str::contains("Unknown host: nonexistent"))
            .stderr(predicate::str::contains("nonexistent"));
    }

    #[test]
    fn test_plan_rejects_conflicting_filters() {
        snow_cmd()
            .args(["plan", "--phase", "production", "--tag", "web"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("cannot be used with"));
    }

    #[test]
    fn test_plan_rejects_zero_eval_workers() {
        snow_cmd()
            .args(["plan", "--eval-workers", "0"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("--eval-workers"));
    }
}
