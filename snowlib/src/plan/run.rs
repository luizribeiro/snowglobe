use std::collections::{BTreeSet, HashMap};

use futures::future::join_all;

use super::{
    build_report, ConfiguredSubstituter, EvalOutcome, EvaluatedHost, HostEval, PlanReport,
    SubstituterLookup,
};
use crate::backends::plan::PlanBackend;
use crate::error::Result;
use crate::nix::{Flake, SystemType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTarget {
    /// Attribute name under nixosConfigurations or darwinConfigurations.
    pub host: String,
    pub system_type: SystemType,
}

/// Work out what building `targets` would build and fetch, without building.
pub async fn create_plan(
    flake: &Flake,
    backend: &dyn PlanBackend,
    targets: &[PlanTarget],
) -> Result<PlanReport> {
    let (evals, substituters) = tokio::join!(
        evaluate_all(flake, backend, targets),
        probe_substituters(backend)
    );
    let (evals, substituters) = (evals?, substituters?);

    let builds: Vec<String> = unique_paths(&evals, |host| host.builds().collect());
    let fetches: Vec<String> = unique_paths(&evals, |host| {
        host.needed_substitutes.iter().map(String::as_str).collect()
    });

    let derivations = if builds.is_empty() {
        Vec::new()
    } else {
        tracing::info!("Describing {} derivations to build", builds.len());
        backend.show_derivations(&builds).await?
    };

    tracing::info!(
        "Checking {} substituters for {} paths",
        substituters.len(),
        fetches.len()
    );
    let lookups = join_all(
        substituters
            .iter()
            .map(|(substituter, reachable)| look_up(backend, substituter, *reachable, &fetches)),
    )
    .await;

    Ok(build_report(&evals, &derivations, &lookups))
}

async fn evaluate_all(
    flake: &Flake,
    backend: &dyn PlanBackend,
    targets: &[PlanTarget],
) -> Result<Vec<HostEval>> {
    let mut by_type: HashMap<SystemType, Vec<String>> = HashMap::new();
    for target in targets {
        by_type
            .entry(target.system_type)
            .or_default()
            .push(target.host.clone());
    }

    tracing::info!("Evaluating {} hosts", targets.len());
    let results = join_all(
        by_type
            .iter()
            .map(|(system_type, hosts)| backend.evaluate(flake, *system_type, hosts)),
    )
    .await;

    let mut evals = Vec::new();
    for result in results {
        evals.extend(result?);
    }
    for target in targets {
        if !evals.iter().any(|eval| eval.host == target.host) {
            evals.push(HostEval {
                host: target.host.clone(),
                outcome: EvalOutcome::Failed {
                    error: "evaluation returned no result for this host".to_string(),
                },
            });
        }
    }
    Ok(evals)
}

async fn probe_substituters(
    backend: &dyn PlanBackend,
) -> Result<Vec<(ConfiguredSubstituter, bool)>> {
    let substituters = backend.substituters().await?;
    let reachable = join_all(substituters.iter().map(|sub| backend.is_reachable(sub))).await;
    Ok(substituters.into_iter().zip(reachable).collect())
}

fn unique_paths<'a>(
    evals: &'a [HostEval],
    paths_of: impl Fn(&'a EvaluatedHost) -> Vec<&'a str>,
) -> Vec<String> {
    evals
        .iter()
        .filter_map(HostEval::evaluated)
        .flat_map(paths_of)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// A failed query is reported as an unreachable substituter instead of failing
/// the plan, since the rest of the report is still accurate.
async fn look_up(
    backend: &dyn PlanBackend,
    substituter: &ConfiguredSubstituter,
    reachable: bool,
    paths: &[String],
) -> SubstituterLookup {
    let lookup = |reachable, paths| SubstituterLookup {
        url: substituter.url.clone(),
        priority: substituter.priority,
        reachable,
        paths,
    };

    if !reachable || paths.is_empty() {
        return lookup(reachable, HashMap::new());
    }
    match backend.query_substituter(substituter, paths).await {
        Ok(found) => lookup(true, found),
        Err(e) => {
            tracing::warn!("Querying {} failed: {}", substituter.url, e);
            lookup(false, HashMap::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::plan::{MockPlanBackend, MockPlanCall};
    use crate::plan::{parse_eval_jobs, DerivationInfo, HostStatus, NarInfo};

    const OPS_EVAL_JOBS: &str = include_str!("../../tests/fixtures/plan/eval-jobs.jsonl");
    const S3: &str = "s3://nix-store-cache";
    const CACHE_NIXOS_ORG: &str = "https://cache.nixos.org";
    const SPRUCE_STAGE2: &str = "/nix/store/my8fv7s5hvjgjqfa3j18bajh4hymlzz0-stage-2-init.sh.drv";
    const DIE_HOOK: &str = "/nix/store/cni8h48r0c96xjlndndabai63hz9dmzr-die-hook";

    fn substituter(url: &str, priority: u32) -> ConfiguredSubstituter {
        ConfiguredSubstituter {
            url: url.to_string(),
            store_url: format!("{url}?priority={priority}"),
            priority,
        }
    }

    fn ops_backend() -> MockPlanBackend {
        let nar = NarInfo {
            download_size: Some(1_000),
            nar_size: 4_000,
        };
        MockPlanBackend {
            evals: HashMap::from([(
                SystemType::NixOS,
                parse_eval_jobs(OPS_EVAL_JOBS.as_bytes()).unwrap(),
            )]),
            derivations: vec![DerivationInfo {
                drv_path: SPRUCE_STAGE2.to_string(),
                name: "stage-2-init.sh".to_string(),
                pname: None,
                version: None,
                system: "x86_64-linux".to_string(),
                trivial: true,
            }],
            substituters: vec![substituter(S3, 1), substituter(CACHE_NIXOS_ORG, 2)],
            substituter_paths: HashMap::from([(
                CACHE_NIXOS_ORG.to_string(),
                HashMap::from([(DIE_HOOK.to_string(), nar)]),
            )]),
            ..Default::default()
        }
    }

    fn nixos(hosts: &[&str]) -> Vec<PlanTarget> {
        hosts
            .iter()
            .map(|host| PlanTarget {
                host: host.to_string(),
                system_type: SystemType::NixOS,
            })
            .collect()
    }

    async fn plan(backend: &MockPlanBackend, targets: &[PlanTarget]) -> Result<PlanReport> {
        create_plan(&Flake::test_context(), backend, targets).await
    }

    #[tokio::test]
    async fn test_plan_for_ops_hosts() {
        let report = plan(
            &ops_backend(),
            &nixos(&["bravo", "raven", "spruce", "kestrel"]),
        )
        .await
        .unwrap();

        let names: Vec<&str> = report.hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["bravo", "kestrel", "raven", "spruce"]);
        assert!(report.has_eval_failures());

        let stage2 = report
            .builds
            .iter()
            .find(|b| b.derivation.drv_path == SPRUCE_STAGE2)
            .unwrap();
        assert!(stage2.derivation.trivial);
        assert_eq!(stage2.hosts, vec!["spruce"]);

        let die_hook = report.fetches.iter().find(|f| f.path == DIE_HOOK).unwrap();
        assert_eq!(die_hook.substituter.as_deref(), Some(CACHE_NIXOS_ORG));
        assert_eq!(report.substituters[1].fetches, 1);
    }

    #[tokio::test]
    async fn test_plan_only_evaluates_selected_hosts() {
        let report = plan(&ops_backend(), &nixos(&["bravo"])).await.unwrap();

        assert_eq!(report.hosts.len(), 1);
        assert!(report.builds.is_empty());
        assert!(!report.has_nontrivial_builds());
        assert_eq!(report.totals.fetches, 1);
    }

    #[tokio::test]
    async fn test_plan_with_no_targets() {
        let report = plan(&ops_backend(), &[]).await.unwrap();

        assert!(report.hosts.is_empty());
        assert_eq!(report.totals, Default::default());
    }

    #[tokio::test]
    async fn test_unreachable_substituter_serves_nothing() {
        let mut backend = ops_backend();
        backend.unreachable = vec![CACHE_NIXOS_ORG.to_string()];

        let report = plan(&backend, &nixos(&["spruce"])).await.unwrap();

        let unreachable: Vec<&str> = report
            .unreachable_substituters()
            .map(|s| s.url.as_str())
            .collect();
        assert_eq!(unreachable, vec![CACHE_NIXOS_ORG]);
        assert_eq!(report.totals.unattributed_fetches, report.totals.fetches);
    }

    #[tokio::test]
    async fn test_failed_query_is_reported_as_unreachable() {
        let mut backend = ops_backend();
        backend.failing_queries = vec![CACHE_NIXOS_ORG.to_string()];

        let report = plan(&backend, &nixos(&["spruce"])).await.unwrap();

        assert!(!report.substituters[1].reachable);
        assert_eq!(report.totals.unattributed_fetches, report.totals.fetches);
    }

    #[tokio::test]
    async fn test_hosts_of_each_system_type_are_evaluated() {
        let mut backend = ops_backend();
        let falcon = parse_eval_jobs(
            br#"{"attr":"falcon","drvPath":"/nix/store/ddd-darwin-system-26.05.drv","system":"aarch64-darwin","cacheStatus":"local","neededBuilds":[],"neededSubstitutes":[]}"#,
        )
        .unwrap();
        backend.evals.insert(SystemType::Darwin, falcon);
        let mut targets = nixos(&["bravo"]);
        targets.push(PlanTarget {
            host: "falcon".to_string(),
            system_type: SystemType::Darwin,
        });

        let report = plan(&backend, &targets).await.unwrap();

        let systems: Vec<(&str, &str)> = report
            .hosts
            .iter()
            .filter_map(|h| match &h.status {
                HostStatus::Evaluated { system, .. } => Some((h.name.as_str(), system.as_str())),
                HostStatus::Failed { .. } => None,
            })
            .collect();
        assert_eq!(
            systems,
            vec![("bravo", "x86_64-linux"), ("falcon", "aarch64-darwin")]
        );
    }

    #[tokio::test]
    async fn test_shared_paths_are_requested_once() {
        let mut backend = ops_backend();
        let nixos_evals = backend.evals.get_mut(&SystemType::NixOS).unwrap();
        let mut spruce_twin = nixos_evals
            .iter()
            .find(|e| e.host == "spruce")
            .unwrap()
            .clone();
        spruce_twin.host = "spruce-twin".to_string();
        nixos_evals.push(spruce_twin);

        plan(&backend, &nixos(&["spruce", "spruce-twin"]))
            .await
            .unwrap();

        let calls = backend.calls.lock().clone();
        let MockPlanCall::ShowDerivations(drvs) = &calls[0] else {
            panic!("expected derivations to be described first, got {calls:?}");
        };
        assert_eq!(drvs.len(), 6);
        assert!(drvs.contains(&SPRUCE_STAGE2.to_string()));
        for call in &calls[1..] {
            let MockPlanCall::QuerySubstituter { paths, .. } = call else {
                panic!("unexpected call {call:?}");
            };
            assert_eq!(paths.len(), 5);
        }
    }

    #[tokio::test]
    async fn test_nothing_is_queried_when_everything_is_local() {
        let mut backend = ops_backend();
        backend.evals.insert(
            SystemType::NixOS,
            parse_eval_jobs(
                br#"{"attr":"osprey","drvPath":"/nix/store/eee-nixos-system-osprey.drv","system":"x86_64-linux","cacheStatus":"local","neededBuilds":[],"neededSubstitutes":[]}"#,
            )
            .unwrap(),
        );

        let report = plan(&backend, &nixos(&["osprey"])).await.unwrap();

        assert!(backend.calls.lock().is_empty());
        assert!(report.substituters.iter().all(|s| s.reachable));
    }

    #[tokio::test]
    async fn test_unreachable_substituter_is_not_queried() {
        let mut backend = ops_backend();
        backend.unreachable = vec![S3.to_string()];

        plan(&backend, &nixos(&["spruce"])).await.unwrap();

        let queried: Vec<String> = backend
            .calls
            .lock()
            .iter()
            .filter_map(|call| match call {
                MockPlanCall::QuerySubstituter { url, .. } => Some(url.clone()),
                MockPlanCall::ShowDerivations(_) => None,
            })
            .collect();
        assert_eq!(queried, vec![CACHE_NIXOS_ORG]);
    }

    #[tokio::test]
    async fn test_host_missing_from_evaluation_is_reported_as_failed() {
        let report = plan(&ops_backend(), &nixos(&["bravo", "xenon"]))
            .await
            .unwrap();

        let xenon = report.hosts.iter().find(|h| h.name == "xenon").unwrap();
        assert!(matches!(xenon.status, HostStatus::Failed { .. }));
        assert!(report.has_eval_failures());
    }

    #[tokio::test]
    async fn test_fatal_evaluation_error_fails_the_plan() {
        let mut backend = ops_backend();
        backend.failing_eval = Some(SystemType::NixOS);

        assert!(plan(&backend, &nixos(&["bravo"])).await.is_err());
    }

    #[tokio::test]
    async fn test_eval_failure_of_one_host_keeps_the_others() {
        let report = plan(&ops_backend(), &nixos(&["kestrel", "spruce"]))
            .await
            .unwrap();

        assert!(report.has_eval_failures());
        assert!(report.builds.iter().any(|b| b.hosts == vec!["spruce"]));
    }
}
