use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use super::{CacheStatus, DerivationInfo, EvalOutcome, EvaluatedHost, HostEval, NarInfo};

/// What one substituter reported for the paths that will be fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct SubstituterLookup {
    pub url: String,
    /// Lower is preferred, as in nix.
    pub priority: u32,
    pub reachable: bool,
    pub paths: HashMap<String, NarInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReport {
    pub hosts: Vec<HostReport>,
    pub builds: Vec<PlannedBuild>,
    pub fetches: Vec<PlannedFetch>,
    pub substituters: Vec<SubstituterSummary>,
    pub totals: Totals,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostReport {
    pub name: String,
    #[serde(flatten)]
    pub status: HostStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum HostStatus {
    #[serde(rename_all = "camelCase")]
    Evaluated {
        drv_path: String,
        system: String,
        cache_status: CacheStatus,
        builds: usize,
        trivial_builds: usize,
        fetches: usize,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedBuild {
    #[serde(flatten)]
    pub derivation: DerivationInfo,
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFetch {
    pub path: String,
    /// None when no queried substituter has the path, though nix expects to fetch it.
    pub substituter: Option<String>,
    pub nar_info: Option<NarInfo>,
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubstituterSummary {
    pub url: String,
    pub priority: u32,
    pub reachable: bool,
    pub fetches: usize,
    pub download_size: u64,
    pub nar_size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub builds: usize,
    pub trivial_builds: usize,
    pub fetches: usize,
    /// Fetches no queried substituter reported, so their sizes are unknown.
    pub unattributed_fetches: usize,
    /// Excludes paths whose narinfo has no `FileSize`.
    pub download_size: u64,
    pub nar_size: u64,
}

impl PlanReport {
    pub fn has_nontrivial_builds(&self) -> bool {
        self.totals.builds > 0
    }

    pub fn has_eval_failures(&self) -> bool {
        self.hosts
            .iter()
            .any(|host| matches!(host.status, HostStatus::Failed { .. }))
    }

    pub fn unreachable_substituters(&self) -> impl Iterator<Item = &SubstituterSummary> {
        self.substituters.iter().filter(|s| !s.reachable)
    }
}

/// Combine per-host evaluations into a fleet-wide, deduplicated report.
///
/// Each fetch is attributed to the highest-priority substituter that has it,
/// the one nix would fetch it from.
pub fn build_report(
    evals: &[HostEval],
    derivations: &[DerivationInfo],
    substituters: &[SubstituterLookup],
) -> PlanReport {
    let mut substituters: Vec<&SubstituterLookup> = substituters.iter().collect();
    substituters.sort_by_key(|sub| sub.priority);

    let derivations: HashMap<&str, &DerivationInfo> = derivations
        .iter()
        .map(|info| (info.drv_path.as_str(), info))
        .collect();

    let builds: Vec<PlannedBuild> = hosts_by_path(evals, |host| host.builds())
        .into_iter()
        .map(|(drv_path, hosts)| PlannedBuild {
            derivation: match derivations.get(drv_path) {
                Some(info) => (*info).clone(),
                None => unknown_derivation(drv_path, system_of(evals, &hosts[0])),
            },
            hosts,
        })
        .collect();

    let fetches: Vec<PlannedFetch> = hosts_by_path(evals, |host| {
        host.needed_substitutes.iter().map(String::as_str)
    })
    .into_iter()
    .map(|(path, hosts)| attribute_fetch(path, hosts, &substituters))
    .collect();

    PlanReport {
        hosts: host_reports(evals, &builds, &fetches),
        substituters: substituter_summaries(&substituters, &fetches),
        totals: totals(&builds, &fetches),
        builds,
        fetches,
    }
}

fn hosts_by_path<'a, I>(
    evals: &'a [HostEval],
    paths_of: impl Fn(&'a EvaluatedHost) -> I,
) -> BTreeMap<&'a str, Vec<String>>
where
    I: Iterator<Item = &'a str>,
{
    let mut by_path: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for eval in evals {
        if let Some(host) = eval.evaluated() {
            for path in paths_of(host) {
                by_path.entry(path).or_default().insert(&eval.host);
            }
        }
    }
    by_path
        .into_iter()
        .map(|(path, hosts)| (path, hosts.into_iter().map(str::to_string).collect()))
        .collect()
}

fn system_of(evals: &[HostEval], host: &str) -> String {
    evals
        .iter()
        .find(|eval| eval.host == host)
        .and_then(HostEval::evaluated)
        .map(|host| host.system.clone())
        .unwrap_or_default()
}

// nix says it will build this and nothing says it is cheap, so it is not trivial.
fn unknown_derivation(drv_path: &str, system: String) -> DerivationInfo {
    let name = drv_path
        .rsplit('/')
        .next()
        .and_then(|base| base.split_once('-'))
        .map_or(drv_path, |(_, name)| name.trim_end_matches(".drv"));
    DerivationInfo {
        drv_path: drv_path.to_string(),
        name: name.to_string(),
        pname: None,
        version: None,
        system,
        trivial: false,
    }
}

fn attribute_fetch(
    path: &str,
    hosts: Vec<String>,
    substituters: &[&SubstituterLookup],
) -> PlannedFetch {
    let source = substituters
        .iter()
        .find_map(|sub| sub.paths.get(path).map(|info| (sub.url.clone(), *info)));
    PlannedFetch {
        path: path.to_string(),
        substituter: source.as_ref().map(|(url, _)| url.clone()),
        nar_info: source.map(|(_, info)| info),
        hosts,
    }
}

fn host_reports(
    evals: &[HostEval],
    builds: &[PlannedBuild],
    fetches: &[PlannedFetch],
) -> Vec<HostReport> {
    let mut build_counts: HashMap<&str, (usize, usize)> = HashMap::new();
    for build in builds {
        for host in &build.hosts {
            let (nontrivial, trivial) = build_counts.entry(host).or_default();
            if build.derivation.trivial {
                *trivial += 1;
            } else {
                *nontrivial += 1;
            }
        }
    }
    let mut fetch_counts: HashMap<&str, usize> = HashMap::new();
    for host in fetches.iter().flat_map(|fetch| &fetch.hosts) {
        *fetch_counts.entry(host).or_default() += 1;
    }

    let mut reports: Vec<HostReport> = evals
        .iter()
        .map(|eval| {
            let name = eval.host.as_str();
            let status = match &eval.outcome {
                EvalOutcome::Evaluated(host) => {
                    let (builds, trivial_builds) =
                        build_counts.get(name).copied().unwrap_or_default();
                    HostStatus::Evaluated {
                        drv_path: host.drv_path.clone(),
                        system: host.system.clone(),
                        cache_status: host.cache_status,
                        builds,
                        trivial_builds,
                        fetches: fetch_counts.get(name).copied().unwrap_or_default(),
                    }
                }
                EvalOutcome::Failed { error } => HostStatus::Failed {
                    error: error.clone(),
                },
            };
            HostReport {
                name: name.to_string(),
                status,
            }
        })
        .collect();
    reports.sort_by(|a, b| a.name.cmp(&b.name));
    reports
}

fn sum_sizes<'a>(infos: impl Iterator<Item = &'a NarInfo>) -> (u64, u64) {
    infos.fold((0, 0), |(download, nar), info| {
        (
            download + info.download_size.unwrap_or(0),
            nar + info.nar_size,
        )
    })
}

fn substituter_summaries(
    substituters: &[&SubstituterLookup],
    fetches: &[PlannedFetch],
) -> Vec<SubstituterSummary> {
    substituters
        .iter()
        .map(|sub| {
            let served: Vec<&NarInfo> = fetches
                .iter()
                .filter(|fetch| fetch.substituter.as_deref() == Some(sub.url.as_str()))
                .filter_map(|fetch| fetch.nar_info.as_ref())
                .collect();
            let (download_size, nar_size) = sum_sizes(served.iter().copied());
            SubstituterSummary {
                url: sub.url.clone(),
                priority: sub.priority,
                reachable: sub.reachable,
                fetches: served.len(),
                download_size,
                nar_size,
            }
        })
        .collect()
}

fn totals(builds: &[PlannedBuild], fetches: &[PlannedFetch]) -> Totals {
    let trivial_builds = builds.iter().filter(|b| b.derivation.trivial).count();
    let (download_size, nar_size) = sum_sizes(fetches.iter().filter_map(|f| f.nar_info.as_ref()));
    Totals {
        builds: builds.len() - trivial_builds,
        trivial_builds,
        fetches: fetches.len(),
        unattributed_fetches: fetches.iter().filter(|f| f.substituter.is_none()).count(),
        download_size,
        nar_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::EvaluatedHost;

    const S3: &str = "s3://nix-store-cache";
    const CACHE_NIXOS_ORG: &str = "https://cache.nixos.org";

    const WINE: &str = "/nix/store/1f5k2zq8kd0wj3vldmfbgsmlijqz3w0a-wine-wow-staging-10.12.drv";
    const ETC_SUDO: &str = "/nix/store/2hd9sbbx2q1k4l3ql8zf5a0n5fqxh3rr-sudo.pam.drv";
    const GLIBC: &str = "/nix/store/3n1ckxgm4yz6r9w3kh2gqd8l0w0f5c1v-glibc-2.40-66";
    const FFMPEG: &str = "/nix/store/2smwf1lipxngcsimlhd077yzx5zd7vvi-ffmpeg-8.0";

    fn system_drv(host: &str) -> String {
        format!("/nix/store/0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-nixos-system-{host}-26.05.drv")
    }

    fn evaluated(
        host: &str,
        cache_status: CacheStatus,
        builds: &[&str],
        substitutes: &[&str],
    ) -> HostEval {
        HostEval {
            host: host.to_string(),
            outcome: EvalOutcome::Evaluated(EvaluatedHost {
                drv_path: system_drv(host),
                system: "x86_64-linux".to_string(),
                cache_status,
                needed_builds: builds.iter().map(|s| s.to_string()).collect(),
                needed_substitutes: substitutes.iter().map(|s| s.to_string()).collect(),
            }),
        }
    }

    fn derivation(drv_path: &str, name: &str, trivial: bool) -> DerivationInfo {
        DerivationInfo {
            drv_path: drv_path.to_string(),
            name: name.to_string(),
            pname: None,
            version: None,
            system: "x86_64-linux".to_string(),
            trivial,
        }
    }

    fn known_derivations() -> Vec<DerivationInfo> {
        vec![
            derivation(WINE, "wine-wow-staging-10.12", false),
            derivation(ETC_SUDO, "sudo.pam", true),
            derivation(&system_drv("radon"), "nixos-system-radon-26.05", false),
            derivation(&system_drv("argon"), "nixos-system-argon-26.05", false),
        ]
    }

    fn lookup(
        url: &str,
        priority: u32,
        reachable: bool,
        paths: &[(&str, u64, u64)],
    ) -> SubstituterLookup {
        SubstituterLookup {
            url: url.to_string(),
            priority,
            reachable,
            paths: paths
                .iter()
                .map(|(path, download, nar)| {
                    (
                        path.to_string(),
                        NarInfo {
                            download_size: Some(*download),
                            nar_size: *nar,
                        },
                    )
                })
                .collect(),
        }
    }

    fn fleet_report() -> PlanReport {
        let evals = vec![
            evaluated(
                "radon",
                CacheStatus::NotBuilt,
                &[WINE, ETC_SUDO],
                &[GLIBC, FFMPEG],
            ),
            evaluated("argon", CacheStatus::NotBuilt, &[ETC_SUDO], &[GLIBC]),
            evaluated("boron", CacheStatus::Cached, &[], &[GLIBC]),
        ];
        let substituters = vec![
            lookup(
                CACHE_NIXOS_ORG,
                40,
                true,
                &[(GLIBC, 8_000, 30_000), (FFMPEG, 1, 1)],
            ),
            lookup(S3, 1, true, &[(FFMPEG, 300, 900)]),
        ];
        build_report(&evals, &known_derivations(), &substituters)
    }

    fn build<'a>(report: &'a PlanReport, drv_path: &str) -> &'a PlannedBuild {
        report
            .builds
            .iter()
            .find(|b| b.derivation.drv_path == drv_path)
            .unwrap()
    }

    fn fetch<'a>(report: &'a PlanReport, path: &str) -> &'a PlannedFetch {
        report.fetches.iter().find(|f| f.path == path).unwrap()
    }

    #[test]
    fn test_shared_paths_are_deduplicated_and_attributed_to_hosts() {
        let report = fleet_report();

        assert_eq!(build(&report, ETC_SUDO).hosts, vec!["argon", "radon"]);
        assert_eq!(fetch(&report, GLIBC).hosts, vec!["argon", "boron", "radon"]);
        assert_eq!(report.fetches.len(), 2);
    }

    #[test]
    fn test_not_built_roots_are_builds() {
        let report = fleet_report();

        assert_eq!(build(&report, &system_drv("radon")).hosts, vec!["radon"]);
        assert!(!report
            .builds
            .iter()
            .any(|b| b.derivation.drv_path == system_drv("boron")));
    }

    #[test]
    fn test_totals_count_shared_paths_once_and_split_trivial_builds() {
        let report = fleet_report();

        assert_eq!(
            report.totals,
            Totals {
                builds: 3,
                trivial_builds: 1,
                fetches: 2,
                unattributed_fetches: 0,
                download_size: 8_300,
                nar_size: 30_900,
            }
        );
        assert!(report.has_nontrivial_builds());
    }

    #[test]
    fn test_fetch_is_attributed_to_first_substituter_in_priority_order() {
        let report = fleet_report();

        assert_eq!(fetch(&report, FFMPEG).substituter.as_deref(), Some(S3));
        assert_eq!(fetch(&report, FFMPEG).nar_info.unwrap().nar_size, 900);

        let summaries: Vec<(&str, usize, u64)> = report
            .substituters
            .iter()
            .map(|s| (s.url.as_str(), s.fetches, s.download_size))
            .collect();
        assert_eq!(summaries, vec![(S3, 1, 300), (CACHE_NIXOS_ORG, 1, 8_000)]);
    }

    #[test]
    fn test_host_reports_are_sorted_with_per_host_counts() {
        let report = fleet_report();

        let names: Vec<&str> = report.hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["argon", "boron", "radon"]);
        match &report.hosts[2].status {
            HostStatus::Evaluated {
                builds,
                trivial_builds,
                fetches,
                ..
            } => assert_eq!((*builds, *trivial_builds, *fetches), (2, 1, 2)),
            other => panic!("expected radon to be evaluated, got {other:?}"),
        }
    }

    #[test]
    fn test_everything_cached() {
        let evals = vec![evaluated("boron", CacheStatus::Local, &[], &[])];

        let report = build_report(&evals, &[], &[lookup(CACHE_NIXOS_ORG, 40, true, &[])]);

        assert!(report.builds.is_empty());
        assert!(report.fetches.is_empty());
        assert!(!report.has_nontrivial_builds());
        assert_eq!(report.totals, Totals::default());
    }

    #[test]
    fn test_only_trivial_builds_are_not_nontrivial() {
        let evals = vec![evaluated("argon", CacheStatus::Cached, &[ETC_SUDO], &[])];

        let report = build_report(&evals, &known_derivations(), &[]);

        assert_eq!(report.totals.trivial_builds, 1);
        assert!(!report.has_nontrivial_builds());
    }

    #[test]
    fn test_eval_failure_is_reported_alongside_other_hosts() {
        let evals = vec![
            evaluated("radon", CacheStatus::NotBuilt, &[WINE], &[]),
            HostEval {
                host: "krypton".to_string(),
                outcome: EvalOutcome::Failed {
                    error: "error: attribute 'doesNotExist' missing".to_string(),
                },
            },
        ];

        let report = build_report(&evals, &known_derivations(), &[]);

        assert!(report.has_eval_failures());
        assert_eq!(report.hosts[0].name, "krypton");
        assert!(matches!(report.hosts[0].status, HostStatus::Failed { .. }));
        assert_eq!(build(&report, WINE).hosts, vec!["radon"]);
    }

    #[test]
    fn test_fetch_missing_from_every_substituter_is_unattributed() {
        let evals = vec![evaluated("boron", CacheStatus::Cached, &[], &[GLIBC])];

        let report = build_report(&evals, &[], &[lookup(S3, 1, false, &[])]);

        assert_eq!(fetch(&report, GLIBC).substituter, None);
        assert_eq!(report.totals.fetches, 1);
        assert_eq!(report.totals.unattributed_fetches, 1);
        assert_eq!(report.totals.download_size, 0);
        assert_eq!(report.substituters[0].fetches, 0);
        let unreachable: Vec<&str> = report
            .unreachable_substituters()
            .map(|s| s.url.as_str())
            .collect();
        assert_eq!(unreachable, vec![S3]);
    }

    #[test]
    fn test_undescribed_derivation_is_named_from_its_path_and_not_trivial() {
        let evals = vec![evaluated("radon", CacheStatus::Cached, &[WINE], &[])];

        let report = build_report(&evals, &[], &[]);

        assert_eq!(report.builds[0].derivation.name, "wine-wow-staging-10.12");
        assert_eq!(report.builds[0].derivation.system, "x86_64-linux");
        assert!(!report.builds[0].derivation.trivial);
        assert!(matches!(
            report.hosts[0].status,
            HostStatus::Evaluated {
                builds: 1,
                trivial_builds: 0,
                ..
            }
        ));
    }

    #[test]
    fn test_json_shape() {
        let json = serde_json::to_value(fleet_report()).unwrap();

        assert_eq!(json["hosts"][0]["name"], "argon");
        assert_eq!(json["hosts"][0]["status"], "evaluated");
        assert_eq!(json["hosts"][0]["cacheStatus"], "notBuilt");
        assert_eq!(json["hosts"][0]["trivialBuilds"], 1);
        let wine = json["builds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["drvPath"] == WINE)
            .unwrap();
        assert_eq!(wine["name"], "wine-wow-staging-10.12");
        assert_eq!(wine["hosts"], serde_json::json!(["radon"]));
        let ffmpeg = json["fetches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == FFMPEG)
            .unwrap();
        assert_eq!(ffmpeg["substituter"], S3);
        assert_eq!(ffmpeg["narInfo"]["downloadSize"], 300);
        assert_eq!(json["substituters"][0]["url"], S3);
        assert_eq!(json["substituters"][0]["priority"], 1);
        assert_eq!(json["totals"]["downloadSize"], 8_300);
    }

    #[test]
    fn test_duplicate_path_within_one_host_counts_once() {
        let evals = vec![evaluated(
            "radon",
            CacheStatus::Cached,
            &[WINE, WINE],
            &[GLIBC, GLIBC],
        )];

        let report = build_report(&evals, &known_derivations(), &[]);

        assert_eq!(report.totals.builds, 1);
        assert_eq!(report.totals.fetches, 1);
        assert!(matches!(
            report.hosts[0].status,
            HostStatus::Evaluated {
                builds: 1,
                fetches: 1,
                ..
            }
        ));
    }

    #[test]
    fn test_missing_download_size_is_excluded_from_download_total() {
        let evals = vec![evaluated(
            "boron",
            CacheStatus::Cached,
            &[],
            &[GLIBC, FFMPEG],
        )];
        let mut s3 = lookup(S3, 1, true, &[(FFMPEG, 300, 900)]);
        s3.paths.insert(
            GLIBC.to_string(),
            NarInfo {
                download_size: None,
                nar_size: 30_000,
            },
        );

        let report = build_report(&evals, &[], &[s3]);

        assert_eq!(report.totals.download_size, 300);
        assert_eq!(report.totals.nar_size, 30_900);
        assert_eq!(report.substituters[0].fetches, 2);
    }

    #[test]
    fn test_failed_and_unattributed_json_shape() {
        let evals = vec![
            evaluated("boron", CacheStatus::Cached, &[], &[GLIBC]),
            HostEval {
                host: "krypton".to_string(),
                outcome: EvalOutcome::Failed {
                    error: "error: attribute 'doesNotExist' missing".to_string(),
                },
            },
        ];

        let json = serde_json::to_value(build_report(&evals, &[], &[])).unwrap();

        assert_eq!(
            json["hosts"][1],
            serde_json::json!({
                "name": "krypton",
                "status": "failed",
                "error": "error: attribute 'doesNotExist' missing",
            })
        );
        assert_eq!(
            json["fetches"][0],
            serde_json::json!({
                "path": GLIBC,
                "substituter": null,
                "narInfo": null,
                "hosts": ["boron"],
            })
        );
    }
}
