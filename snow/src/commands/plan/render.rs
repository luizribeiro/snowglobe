use std::fmt::Write;

use console::style;

use snowlib::plan::{CacheStatus, HostStatus, PlanReport, PlannedBuild};

use crate::error_summary::error_summary;
use crate::marks;
use crate::plural::plural;

pub fn render(report: &PlanReport, show_trivial: bool) -> String {
    let mut out = String::new();
    let host_count = report.hosts.len();
    writeln!(
        out,
        "Plan for {} {}",
        host_count,
        plural(host_count, "host", "hosts")
    )
    .unwrap();

    render_builds(&mut out, report, show_trivial);
    render_fetches(&mut out, report);
    render_hosts(&mut out, report);
    render_warnings(&mut out, report);
    out
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn max_width<'a>(values: impl Iterator<Item = &'a str>) -> usize {
    values.map(str::len).max().unwrap_or(0)
}

fn trivial_note(trivial_builds: usize, show_trivial: bool) -> String {
    let builds = plural(trivial_builds, "build", "builds");
    match (trivial_builds, show_trivial) {
        (0, _) => String::new(),
        (n, true) => format!(" (including {n} trivial local {builds})"),
        (n, false) => format!(" (+{n} trivial local {builds}, --all to list)"),
    }
}

fn render_builds(out: &mut String, report: &PlanReport, show_trivial: bool) {
    let totals = &report.totals;
    let mut builds: Vec<&PlannedBuild> = report
        .builds
        .iter()
        .filter(|build| show_trivial || !build.derivation.trivial)
        .collect();
    builds.sort_by(|a, b| a.derivation.name.cmp(&b.derivation.name));

    let count = if show_trivial {
        totals.builds + totals.trivial_builds
    } else {
        totals.builds
    };
    writeln!(
        out,
        "\nWill build: {count} {}{}",
        plural(count, "derivation", "derivations"),
        trivial_note(totals.trivial_builds, show_trivial)
    )
    .unwrap();

    let name_width = builds
        .iter()
        .map(|b| b.derivation.name.len())
        .max()
        .unwrap_or(0);
    let system_width = builds
        .iter()
        .map(|b| b.derivation.system.len())
        .max()
        .unwrap_or(0);
    for build in builds {
        writeln!(
            out,
            "  {:name_width$}  {:system_width$}  {}",
            build.derivation.name,
            build.derivation.system,
            build.hosts.join(", ")
        )
        .unwrap();
    }
}

fn render_fetches(out: &mut String, report: &PlanReport) {
    let totals = &report.totals;
    writeln!(
        out,
        "\nWill fetch: {} {}, {} download / {} unpacked",
        totals.fetches,
        plural(totals.fetches, "path", "paths"),
        format_size(totals.download_size),
        format_size(totals.nar_size)
    )
    .unwrap();

    let serving: Vec<_> = report
        .substituters
        .iter()
        .filter(|s| s.fetches > 0)
        .collect();
    let url_width = max_width(serving.iter().map(|s| s.url.as_str()));
    for substituter in serving {
        writeln!(
            out,
            "  {:url_width$}  {:>6} {:<5}  {} / {}",
            substituter.url,
            substituter.fetches,
            plural(substituter.fetches, "path", "paths"),
            format_size(substituter.download_size),
            format_size(substituter.nar_size)
        )
        .unwrap();
    }
    if totals.unattributed_fetches > 0 {
        writeln!(
            out,
            "  {} {} not found in any reachable substituter",
            totals.unattributed_fetches,
            plural(totals.unattributed_fetches, "path", "paths")
        )
        .unwrap();
    }
}

fn render_hosts(out: &mut String, report: &PlanReport) {
    writeln!(out, "\nHosts").unwrap();
    let name_width = max_width(report.hosts.iter().map(|h| h.name.as_str()));
    for host in &report.hosts {
        writeln!(
            out,
            "  {:name_width$}  {}",
            host.name,
            host_summary(&host.status)
        )
        .unwrap();
    }
}

fn host_summary(status: &HostStatus) -> String {
    match status {
        HostStatus::Failed { error } => {
            format!(
                "{} evaluation failed: {}",
                marks::failure(),
                error_summary(error)
            )
        }
        HostStatus::Evaluated {
            cache_status: CacheStatus::Local,
            ..
        } => "up to date (in local store)".to_string(),
        HostStatus::Evaluated {
            builds,
            trivial_builds,
            fetches,
            ..
        } => format!(
            "{builds} {}, {trivial_builds} trivial, {fetches} {}",
            plural(*builds, "build", "builds"),
            plural(*fetches, "fetch", "fetches")
        ),
    }
}

fn render_warnings(out: &mut String, report: &PlanReport) {
    let unreachable: Vec<_> = report.unreachable_substituters().collect();
    if unreachable.is_empty() {
        return;
    }
    writeln!(
        out,
        "\n{} {}",
        marks::warning(),
        style("Warnings").yellow().bold()
    )
    .unwrap();
    for substituter in unreachable {
        writeln!(
            out,
            "  {} was unreachable; paths it would serve are counted as builds or not found",
            substituter.url
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowlib::plan::{
        build_report, parse_eval_jobs, DerivationInfo, NarInfo, SubstituterLookup,
    };
    use std::collections::HashMap;

    const OPS_EVAL_JOBS: &str =
        include_str!("../../../../snowlib/tests/fixtures/plan/eval-jobs.jsonl");
    const DIE_HOOK: &str = "/nix/store/cni8h48r0c96xjlndndabai63hz9dmzr-die-hook";
    const WINE: &str = "/nix/store/1f5k2zq8kd0wj3vldmfbgsmlijqz3w0a-wine-wow-staging-10.12.drv";

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

    fn ops_report(s3_reachable: bool) -> PlanReport {
        let mut evals = parse_eval_jobs(OPS_EVAL_JOBS.as_bytes()).unwrap();
        if let snowlib::plan::EvalOutcome::Evaluated(radon) = &mut evals[1].outcome {
            radon.needed_builds.push(WINE.to_string());
        }
        // Every derivation in the real fixture is a trivial NixOS config build.
        let mut derivations: Vec<DerivationInfo> = evals
            .iter()
            .filter_map(|eval| eval.evaluated())
            .flat_map(|host| host.builds())
            .filter(|drv| *drv != WINE)
            .map(|drv| derivation(drv, drv.split_once('-').unwrap().1, true))
            .collect();
        derivations.push(derivation(WINE, "wine-wow-staging-10.12", false));
        let substituters = vec![
            SubstituterLookup {
                url: "s3://nix-store-cache".to_string(),
                priority: 1,
                reachable: s3_reachable,
                paths: HashMap::new(),
            },
            SubstituterLookup {
                url: "https://cache.nixos.org".to_string(),
                priority: 2,
                reachable: true,
                paths: HashMap::from([(
                    DIE_HOOK.to_string(),
                    NarInfo {
                        download_size: Some(3 * 1024 * 1024),
                        nar_size: 12 * 1024 * 1024,
                    },
                )]),
            },
        ];
        build_report(&evals, &derivations, &substituters)
    }

    #[test]
    fn test_render_ops_plan() {
        let rendered = console::strip_ansi_codes(&render(&ops_report(true), false)).to_string();

        assert!(rendered.starts_with(
            "Plan for 4 hosts\n\nWill build: 1 derivation (+11 trivial local builds, --all to list)\n"
        ));
        assert!(rendered.contains("  wine-wow-staging-10.12  x86_64-linux  radon\n"));
        assert!(!rendered.contains("stage-2-init.sh"));
        assert!(rendered.contains("--all to list"));
        assert!(rendered.contains("  https://cache.nixos.org       1 path   3.0 MiB / 12.0 MiB\n"));
        assert!(rendered.contains("  boron    0 builds, 0 trivial, 1 fetch\n"));
        assert!(rendered.contains("  radon    1 build, 5 trivial, 4 fetches\n"));
        assert!(
            rendered.contains("  krypton  ✗ evaluation failed: attribute 'doesNotExist' missing\n")
        );
        assert!(!rendered.contains("Warnings"));
    }

    #[test]
    fn test_render_all_lists_trivial_builds() {
        let rendered = render(&ops_report(true), true);

        assert!(rendered.contains("stage-2-init.sh"));
        assert!(rendered.contains("trivial local builds)"));
        assert!(!rendered.contains("--all to list"));
    }

    #[test]
    fn test_render_warns_about_unreachable_substituter() {
        let rendered = console::strip_ansi_codes(&render(&ops_report(false), false)).to_string();

        assert!(rendered.contains(
            "! Warnings\n  s3://nix-store-cache was unreachable; paths it would serve are counted as builds or not found\n"
        ));
        assert!(rendered.contains("not found in any reachable substituter"));
    }

    #[test]
    fn test_render_empty_plan() {
        let report = build_report(&[], &[], &[]);

        assert_eq!(
            render(&report, false),
            "Plan for 0 hosts\n\nWill build: 0 derivations\n\nWill fetch: 0 paths, 0 B download / 0 B unpacked\n\nHosts\n"
        );
    }

    #[test]
    fn test_local_host_is_up_to_date() {
        let status = HostStatus::Evaluated {
            drv_path: "/nix/store/aaa-nixos-system-oxygen.drv".to_string(),
            system: "x86_64-linux".to_string(),
            cache_status: CacheStatus::Local,
            builds: 0,
            trivial_builds: 0,
            fetches: 0,
        };

        assert_eq!(host_summary(&status), "up to date (in local store)");
    }

    #[test]
    fn test_trivial_note_wording() {
        assert_eq!(trivial_note(0, false), "");
        assert_eq!(
            trivial_note(1, false),
            " (+1 trivial local build, --all to list)"
        );
        assert_eq!(
            trivial_note(187, true),
            " (including 187 trivial local builds)"
        );
    }

    #[test]
    fn test_render_json_round_trips_key_fields() {
        let json = serde_json::to_value(ops_report(true)).unwrap();

        assert_eq!(json["totals"]["builds"], 1);
        assert_eq!(json["hosts"][1]["status"], "failed");
        assert_eq!(json["substituters"][1]["fetches"], 1);
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(2_254_857_830), "2.1 GiB");
    }
}
