//! Terminal output formatting for deployment results
//!
//! Handles each phase's report and the final summary.
//! Keeps ANSI terminal formatting out of the library layer
//! (library should be reusable by TUI, agents, etc.).

use super::board::{format_elapsed, plural};
use super::colors::style;
use crate::error_summary::{error_context, error_summary, truncate};
use crate::marks;
use std::path::Path;

/// Keeps the terminal output scannable; the log has every error in full
const MAX_CONTEXT_LINES: usize = 10;

/// The phase hosts without a push phase share, which has no name to show
pub(super) fn is_unnamed_phase(phase: &str) -> bool {
    matches!(phase, "default" | "_all")
}

/// "Phase p2", or "Push" for the phase without a name
pub(super) fn phase_title(phase: &str) -> String {
    if is_unnamed_phase(phase) {
        "Push".to_string()
    } else {
        format!("Phase {phase}")
    }
}

/// Which hosts each phase will push, so a host nobody expected shows up
/// before anything happens
pub fn phase_overview(phases: &[(String, Vec<String>)]) -> String {
    let total: usize = phases.iter().map(|(_, hosts)| hosts.len()).sum();
    let pushing = format!("Pushing {total} {}", plural(total, "host"));
    match phases {
        [(phase, hosts)] if is_unnamed_phase(phase) => {
            format!(
                "{} {}",
                style(format!("{pushing}:")).bold(),
                hosts.join(", ")
            )
        }
        _ => {
            let width = phases
                .iter()
                .map(|(phase, _)| phase.len())
                .max()
                .unwrap_or(0);
            let heading = format!(
                "{pushing} in {} {}:",
                phases.len(),
                plural(phases.len(), "phase")
            );
            let mut overview = style(heading).bold().to_string();
            for (phase, hosts) in phases {
                overview.push_str(&format!(
                    "\n  {}  {}",
                    style(format!("{phase:<width$}")).green().bold(),
                    hosts.join(", ")
                ));
            }
            overview
        }
    }
}

/// What is left once every phase reported itself: skipped phases, the
/// totals and the errors in full
pub fn print_summary(results: &snowlib::PushResults, log_file: Option<&Path>) {
    print_skipped_phases(&results.skipped_phases);
    print_overall_summary(results);
    print_error_details(results, log_file);
    println!();
}

/// How a phase went, shown as soon as it ends, with each failure's cause cut
/// to fit `width` columns:
///
/// ```text
/// ✗ Phase p2 (1/3 failed in 0:52)
///   ✓ boron (switch)
///   ✗ cerium at activate: warning: the following units failed: storage-agent.service
/// ```
pub fn phase_report(phase: &snowlib::PhaseResult, width: usize) -> String {
    let mut lines = vec![phase_heading(phase)];
    lines.extend(phase.hosts.iter().map(|host| host_line(host, width)));
    lines.join("\n")
}

fn phase_heading(phase: &snowlib::PhaseResult) -> String {
    let title = phase_title(&phase.phase);
    let took = format_elapsed(phase.duration);
    let total = phase.hosts.len();
    if phase.all_succeeded() {
        format!(
            "{} {}",
            style(format!("✓ {title}")).green().bold(),
            style(format!("({total} {} in {took})", plural(total, "host"))).dim()
        )
    } else {
        format!(
            "{} {}",
            style(format!("✗ {title}")).red().bold(),
            style(format!(
                "({}/{total} failed in {took})",
                phase.failure_count()
            ))
            .dim()
        )
    }
}

pub(super) fn success_labels(host: &snowlib::HostDeploymentResult) -> Vec<&'static str> {
    use snowlib::ActivationMode;

    let mut labels = Vec::new();
    if let Some(mode) = host.activation_mode {
        labels.push(match mode {
            ActivationMode::Switch => "switch",
            ActivationMode::Boot => "boot",
            ActivationMode::Test => "test",
            ActivationMode::DryActivate => "dry-run",
        });
    }
    if host.rebooted {
        labels.push("rebooted");
    }
    if host.unchanged {
        labels.push("unchanged");
    }
    labels
}

fn host_line(host: &snowlib::HostDeploymentResult, width: usize) -> String {
    if host.success {
        let labels = success_labels(host);
        let status = if labels.is_empty() {
            String::new()
        } else {
            format!(" {}", style(format!("({})", labels.join(", "))).dim())
        };
        format!("  {} {}{status}", marks::success(), host.hostname)
    } else if host.is_skipped() {
        let reason = host.error_message.as_deref().unwrap_or_default();
        format!(
            "  {}",
            marks::skipped_line(&format!("{} skipped, {reason}", host.hostname))
        )
    } else {
        failure_line(host, width)
    }
}

fn failure_line(host: &snowlib::HostDeploymentResult, width: usize) -> String {
    let step = host
        .failed_at_step
        .map(|step| format!(" at {step}"))
        .unwrap_or_default();
    let line = format!(
        "  {} {}{}",
        marks::failure(),
        host.hostname,
        style(step).red()
    );
    match host.error_message.as_deref().map(error_summary) {
        Some(cause) if !cause.is_empty() => {
            // Some terminals wrap a line that reaches the last column
            let room = width.saturating_sub(console::measure_text_width(&line) + ": ".len() + 1);
            format!("{line}: {}", truncate(cause, room))
        }
        _ => line,
    }
}

fn print_skipped_phases(skipped_phases: &[snowlib::SkippedPhase]) {
    for skipped in skipped_phases {
        println!("{}\n", skipped_phase_report(skipped));
    }
}

fn skipped_phase_report(skipped: &snowlib::SkippedPhase) -> String {
    let count = skipped.hostnames.len();
    let heading = marks::skipped_line(&format!(
        "{} ({count} {} skipped)",
        phase_title(&skipped.phase),
        plural(count, "host")
    ));
    std::iter::once(heading)
        .chain(
            skipped
                .hostnames
                .iter()
                .map(|hostname| format!("  {}", marks::skipped_line(hostname))),
        )
        .collect::<Vec<_>>()
        .join("\n")
}

fn print_overall_summary(results: &snowlib::PushResults) {
    println!("{}", overall_summary(results));
}

fn overall_summary(results: &snowlib::PushResults) -> String {
    let succeeded = results.success_count();
    let failed = results.failure_count();
    let skipped = results.skipped_count();
    if failed == 0 && skipped == 0 {
        return format!(
            "{} {}",
            marks::success().bold(),
            style(format!(
                "{succeeded} {} deployed",
                plural(succeeded, "host")
            ))
            .green()
            .bold()
        );
    }

    let mut text = format!("{succeeded}/{} deployed", results.total_count());
    for (count, label) in [(failed, "failed"), (skipped, "skipped")] {
        if count > 0 {
            text.push_str(&format!(", {count} {label}"));
        }
    }
    if failed > 0 {
        format!("{} {}", marks::failure().bold(), style(text).red().bold())
    } else {
        format!("{} {}", marks::warning(), style(text).yellow().bold())
    }
}

fn print_error_details(results: &snowlib::PushResults, log_file: Option<&Path>) {
    let errors = collect_failed_hosts(results);
    if errors.is_empty() {
        return;
    }

    println!();
    println!("{}", style("Errors:").red().bold());

    for host in errors {
        if let Some(error_msg) = &host.error_message {
            print_error_header(host);
            print_error_lines(error_msg, log_file);
        }
    }
}

fn collect_failed_hosts(results: &snowlib::PushResults) -> Vec<&snowlib::HostDeploymentResult> {
    results
        .phases
        .iter()
        .flat_map(|p| &p.hosts)
        .filter(|h| !h.success && !h.is_skipped() && h.error_message.is_some())
        .collect()
}

fn print_error_header(host: &snowlib::HostDeploymentResult) {
    let step_label = host
        .failed_at_step
        .map(|s| format!(" ({})", s))
        .unwrap_or_default();
    println!(
        "  {} {}{}",
        style("▸").red(),
        host.hostname,
        style(step_label).bold()
    );
}

fn print_error_lines(error_msg: &str, log_file: Option<&Path>) {
    for line in failure_details(error_msg, log_file) {
        println!("    {line}");
    }
}

/// The lines leading up to the cause, dim but for the cause itself, then
/// where to read the rest
fn failure_details(error: &str, log_file: Option<&Path>) -> Vec<String> {
    let context = error_context(error, MAX_CONTEXT_LINES);
    let mut details = Vec::new();
    if let [leading @ .., cause] = context.lines.as_slice() {
        details.extend(leading.iter().map(|line| style(line).dim().to_string()));
        details.push(cause.to_string());
    }
    if context.hidden > 0 {
        details.push(
            style(more_lines_hint(context.hidden, log_file))
                .dim()
                .to_string(),
        );
    }
    details
}

/// Where to read the lines an error left out. Without a log file, logs go
/// to stderr, which has every failure in full.
fn more_lines_hint(hidden: usize, log_file: Option<&Path>) -> String {
    let lines = format!("… {hidden} more {}", plural(hidden, "line"));
    match log_file {
        Some(path) => format!("{lines} in {}", path.display()),
        None => format!("{lines} in the log on stderr"),
    }
}

#[cfg(test)]
mod tests {
    use super::phase_overview;
    use snowlib::{
        ActivationMode, HostDeploymentResult, PhaseResult, PipelineStep, PushResults, SkippedPhase,
    };
    use std::path::Path;
    use std::time::Duration;

    fn phase(name: &str, hosts: &[&str]) -> (String, Vec<String>) {
        (
            name.to_string(),
            hosts.iter().map(|h| h.to_string()).collect(),
        )
    }

    #[test]
    fn test_overview_lists_each_phase_with_its_hosts() {
        console::set_colors_enabled(false);
        let overview = phase_overview(&[
            phase("p2", &["argon", "curium", "ferrum"]),
            phase("p3-k8s", &["cerium", "lanthanum"]),
        ]);

        assert_eq!(
            overview,
            "Pushing 5 hosts in 2 phases:\n  p2      argon, curium, ferrum\n  p3-k8s  cerium, lanthanum"
        );
    }

    #[test]
    fn test_overview_of_a_push_without_phases_is_one_line() {
        console::set_colors_enabled(false);
        assert_eq!(
            phase_overview(&[phase("default", &["boron", "radon"])]),
            "Pushing 2 hosts: boron, radon"
        );
        assert_eq!(
            phase_overview(&[phase("_all", &["sodium"])]),
            "Pushing 1 host: sodium"
        );
    }

    #[test]
    fn test_overview_names_a_single_real_phase() {
        console::set_colors_enabled(false);
        assert_eq!(
            phase_overview(&[phase("p2", &["ferrum"])]),
            "Pushing 1 host in 1 phase:\n  p2  ferrum"
        );
    }

    fn make_successful_host(
        name: &str,
        mode: ActivationMode,
        rebooted: bool,
    ) -> HostDeploymentResult {
        HostDeploymentResult {
            hostname: name.to_string(),
            success: true,
            activation_mode: Some(mode),
            rebooted,
            unchanged: false,
            failed_at_step: None,
            error_message: None,
        }
    }

    fn make_failed_host(name: &str, step: PipelineStep, error: &str) -> HostDeploymentResult {
        HostDeploymentResult {
            hostname: name.to_string(),
            success: false,
            activation_mode: None,
            rebooted: false,
            unchanged: false,
            failed_at_step: Some(step),
            error_message: Some(error.to_string()),
        }
    }

    #[test]
    fn test_all_hosts_successful() {
        let results = PushResults {
            phases: vec![PhaseResult {
                phase: "default".to_string(),
                duration: Duration::ZERO,
                hosts: vec![
                    make_successful_host("hydrogen", ActivationMode::Switch, false),
                    make_successful_host("oxygen", ActivationMode::Switch, true),
                ],
            }],
            skipped_phases: vec![],
        };

        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);
    }

    #[test]
    fn test_partial_failure() {
        let results = PushResults {
            phases: vec![PhaseResult {
                phase: "production".to_string(),
                duration: Duration::ZERO,
                hosts: vec![
                    make_successful_host("hydrogen", ActivationMode::Switch, false),
                    make_failed_host("nitrogen", PipelineStep::Activate, "activation failed"),
                ],
            }],
            skipped_phases: vec![],
        };

        assert!(!results.all_succeeded());
        assert_eq!(results.success_count(), 1);
        assert_eq!(results.failure_count(), 1);
    }

    #[test]
    fn test_skipped_phases() {
        let results = PushResults {
            phases: vec![PhaseResult {
                phase: "staging".to_string(),
                duration: Duration::ZERO,
                hosts: vec![make_successful_host("web01", ActivationMode::Switch, false)],
            }],
            skipped_phases: vec![SkippedPhase {
                phase: "production".to_string(),
                hostnames: vec!["prod01".to_string(), "prod02".to_string()],
            }],
        };

        assert_eq!(results.skipped_count(), 2);
    }

    #[test]
    fn test_multiple_phases() {
        let results = PushResults {
            phases: vec![
                PhaseResult {
                    phase: "staging".to_string(),
                    duration: Duration::ZERO,
                    hosts: vec![make_successful_host("stg01", ActivationMode::Switch, false)],
                },
                PhaseResult {
                    phase: "production".to_string(),
                    duration: Duration::ZERO,
                    hosts: vec![
                        make_successful_host("prod01", ActivationMode::Switch, true),
                        make_successful_host("prod02", ActivationMode::Boot, false),
                    ],
                },
            ],
            skipped_phases: vec![],
        };

        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 3);
    }

    #[test]
    fn test_activation_modes() {
        let results = PushResults {
            phases: vec![PhaseResult {
                phase: "default".to_string(),
                duration: Duration::ZERO,
                hosts: vec![
                    make_successful_host("h1", ActivationMode::Switch, false),
                    make_successful_host("h2", ActivationMode::Boot, false),
                    make_successful_host("h3", ActivationMode::Test, false),
                    make_successful_host("h4", ActivationMode::DryActivate, false),
                ],
            }],
            skipped_phases: vec![],
        };

        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 4);
    }

    #[test]
    fn test_phase_report_says_how_long_the_phase_took() {
        console::set_colors_enabled(false);
        let phase = PhaseResult {
            phase: "p2".to_string(),
            duration: Duration::from_secs(52),
            hosts: vec![
                make_successful_host("boron", ActivationMode::Switch, false),
                make_failed_host("cerium", PipelineStep::Activate, "activation failed"),
                make_failed_host("radon", PipelineStep::PreDeploy, "storage degraded"),
                HostDeploymentResult::skipped_by_serial_group("lanthanum", "k8s", "cerium"),
            ],
        };

        assert_eq!(
            super::phase_report(&phase, 100),
            "✗ Phase p2 (3/4 failed in 0:52)\n  ✓ boron (switch)\n  ✗ cerium at activate: activation failed\n  ✗ radon at pre-deploy: storage degraded\n  ⊘ lanthanum skipped, serial group 'k8s' halted after cerium failed"
        );
    }

    #[test]
    fn test_phase_report_shows_why_a_host_failed_cut_to_the_terminal() {
        console::set_colors_enabled(false);
        let error = "Command failed\n\n\
                     Command: ssh cerium sudo snowglobe-activate /nix/store/yzxw-cerium switch\n\
                     Exit code: 4\n\n\
                     Stderr:\n\
                     activating the configuration...\n\
                     warning: the following units failed: storage-agent.service";
        let phase = PhaseResult {
            phase: "p2".to_string(),
            duration: Duration::from_secs(52),
            hosts: vec![make_failed_host("cerium", PipelineStep::Activate, error)],
        };

        assert_eq!(
            super::phase_report(&phase, 100),
            "✗ Phase p2 (1/1 failed in 0:52)\n  ✗ cerium at activate: warning: the following units failed: storage-agent.service"
        );
        assert_eq!(
            super::phase_report(&phase, 50),
            "✗ Phase p2 (1/1 failed in 0:52)\n  ✗ cerium at activate: warning: the following u…"
        );
    }

    #[test]
    fn test_failure_without_a_reason_shows_only_where_it_failed() {
        console::set_colors_enabled(false);
        let mut cerium = make_failed_host("cerium", PipelineStep::Copy, "");
        let radon = HostDeploymentResult {
            error_message: None,
            ..make_failed_host("radon", PipelineStep::Build, "")
        };

        assert_eq!(super::failure_line(&cerium, 100), "  ✗ cerium at copy");
        assert_eq!(super::failure_line(&radon, 100), "  ✗ radon at build");
        cerium.failed_at_step = None;
        assert_eq!(super::failure_line(&cerium, 100), "  ✗ cerium");
    }

    #[test]
    fn test_phase_report_of_a_push_without_phases() {
        console::set_colors_enabled(false);
        for name in ["default", "_all"] {
            let phase = PhaseResult {
                phase: name.to_string(),
                duration: Duration::from_secs(3727),
                hosts: vec![make_successful_host(
                    "sodium",
                    ActivationMode::Switch,
                    false,
                )],
            };

            assert_eq!(
                super::phase_report(&phase, 100),
                "✓ Push (1 host in 1:02:07)\n  ✓ sodium (switch)"
            );
        }
    }

    #[test]
    fn test_failure_details_lead_up_to_the_cause_then_point_at_the_log() {
        let error = "Command failed\n\n\
                     Command: ssh sudo snowglobe-activate /nix/store/yzxw-cerium switch\n\
                     Exit code: 4\n\n\
                     Stderr:\n\
                     activating the configuration...\n\
                     warning: the following units failed: storage-agent.service";
        let log = Some(Path::new("/tmp/snow-1762255380.log"));

        let details = super::failure_details(error, log);

        assert_eq!(
            details
                .iter()
                .map(|line| console::strip_ansi_codes(line))
                .collect::<Vec<_>>(),
            vec![
                "activating the configuration...",
                "warning: the following units failed: storage-agent.service",
                "… 4 more lines in /tmp/snow-1762255380.log",
            ]
        );
        assert_eq!(
            super::failure_details("worker crashed", None),
            vec!["worker crashed"]
        );
    }

    #[test]
    fn test_hint_without_a_log_file_points_at_stderr() {
        assert_eq!(
            super::more_lines_hint(1, None),
            "… 1 more line in the log on stderr"
        );
        assert_eq!(
            super::more_lines_hint(3, Some(Path::new("/tmp/snow-1762255380.log"))),
            "… 3 more lines in /tmp/snow-1762255380.log"
        );
    }

    fn deployed_phase(hosts: &[&str]) -> PhaseResult {
        PhaseResult {
            phase: "p2".to_string(),
            duration: Duration::ZERO,
            hosts: hosts
                .iter()
                .map(|h| make_successful_host(h, ActivationMode::Switch, false))
                .collect(),
        }
    }

    #[test]
    fn test_overall_summary_pluralizes_deployed_hosts() {
        console::set_colors_enabled(false);
        let one = PushResults {
            phases: vec![deployed_phase(&["sodium"])],
            skipped_phases: vec![],
        };
        let three = PushResults {
            phases: vec![deployed_phase(&["boron", "cerium", "radon"])],
            skipped_phases: vec![],
        };

        assert_eq!(super::overall_summary(&one), "✓ 1 host deployed");
        assert_eq!(super::overall_summary(&three), "✓ 3 hosts deployed");
    }

    #[test]
    fn test_overall_summary_shows_failures_next_to_skipped_hosts() {
        console::set_colors_enabled(false);
        let mut p2 = deployed_phase(&["boron", "radon"]);
        p2.hosts.push(make_failed_host(
            "cerium",
            PipelineStep::Activate,
            "activation failed",
        ));
        let skipped_phases = vec![SkippedPhase {
            phase: "p3".to_string(),
            hostnames: vec!["lanthanum".to_string(), "sodium".to_string()],
        }];
        let failed_and_skipped = PushResults {
            phases: vec![p2.clone()],
            skipped_phases: skipped_phases.clone(),
        };
        let failed_only = PushResults {
            phases: vec![p2],
            skipped_phases: vec![],
        };
        let skipped_only = PushResults {
            phases: vec![deployed_phase(&["boron"])],
            skipped_phases,
        };

        assert_eq!(
            super::overall_summary(&failed_and_skipped),
            "✗ 2/5 deployed, 1 failed, 2 skipped"
        );
        assert_eq!(
            super::overall_summary(&failed_only),
            "✗ 2/3 deployed, 1 failed"
        );
        assert_eq!(
            super::overall_summary(&skipped_only),
            "! 1/3 deployed, 2 skipped"
        );
    }

    #[test]
    fn test_skipped_phase_lists_its_hosts() {
        console::set_colors_enabled(false);
        let one = SkippedPhase {
            phase: "p3".to_string(),
            hostnames: vec!["lanthanum".to_string()],
        };
        let two = SkippedPhase {
            phase: "p3-k8s".to_string(),
            hostnames: vec!["cerium".to_string(), "lanthanum".to_string()],
        };

        assert_eq!(
            super::skipped_phase_report(&one),
            "⊘ Phase p3 (1 host skipped)\n  ⊘ lanthanum"
        );
        assert_eq!(
            super::skipped_phase_report(&two),
            "⊘ Phase p3-k8s (2 hosts skipped)\n  ⊘ cerium\n  ⊘ lanthanum"
        );
    }

    #[test]
    fn test_success_labels() {
        assert_eq!(
            super::success_labels(&make_successful_host("h1", ActivationMode::Boot, true)),
            vec!["boot", "rebooted"]
        );
        assert_eq!(
            super::success_labels(&HostDeploymentResult::unchanged("h2")),
            vec!["unchanged"]
        );
    }

    #[test]
    fn test_error_at_different_steps() {
        let results = PushResults {
            phases: vec![PhaseResult {
                phase: "default".to_string(),
                duration: Duration::ZERO,
                hosts: vec![
                    make_failed_host("h1", PipelineStep::Build, "build error"),
                    make_failed_host("h2", PipelineStep::Copy, "copy error"),
                    make_failed_host("h3", PipelineStep::Activate, "activate error"),
                    make_failed_host("h4", PipelineStep::Reboot, "reboot timeout"),
                ],
            }],
            skipped_phases: vec![],
        };

        assert!(!results.all_succeeded());
        assert_eq!(results.failure_count(), 4);
        assert_eq!(
            results.phases[0].hosts[0].failed_at_step,
            Some(PipelineStep::Build)
        );
        assert_eq!(
            results.phases[0].hosts[1].failed_at_step,
            Some(PipelineStep::Copy)
        );
        assert_eq!(
            results.phases[0].hosts[2].failed_at_step,
            Some(PipelineStep::Activate)
        );
        assert_eq!(
            results.phases[0].hosts[3].failed_at_step,
            Some(PipelineStep::Reboot)
        );
    }
}
