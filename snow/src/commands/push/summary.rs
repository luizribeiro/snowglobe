//! Terminal output formatting for deployment results
//!
//! Handles each phase's report and the final summary.
//! Keeps ANSI terminal formatting out of the library layer
//! (library should be reusable by TUI, agents, etc.).

use super::board::{format_elapsed, plural};
use super::colors::style;
use crate::marks;
use snowlib::error::summary::{error_context, error_details, error_summary, truncate};
use snowlib::push::{ALL_HOSTS, UNPHASED};
use std::path::Path;

/// Keeps the terminal output scannable; the log has every error in full
const MAX_CONTEXT_LINES: usize = 10;

/// The phase hosts without a push phase share, which has no name to show
fn is_unnamed_phase(phase: &str) -> bool {
    matches!(phase, ALL_HOSTS | UNPHASED)
}

/// "Phase p2", or "Push" for a push's only phase when it has no name
pub(super) fn phase_title(phase: &str, among_other_phases: bool) -> String {
    match phase {
        UNPHASED if among_other_phases => "Hosts without a phase".to_string(),
        ALL_HOSTS | UNPHASED => "Push".to_string(),
        _ => format!("Phase {phase}"),
    }
}

/// How the phase overview lists a phase
fn overview_label(phase: &str) -> &str {
    if phase == UNPHASED {
        "(no phase)"
    } else {
        phase
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
                .map(|(phase, _)| overview_label(phase).len())
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
                    style(format!("{:<width$}", overview_label(phase)))
                        .green()
                        .bold(),
                    hosts.join(", ")
                ));
            }
            overview
        }
    }
}

/// What is left once every phase reported itself: skipped phases and the
/// totals
pub fn print_summary(results: &snowlib::PushResults) {
    print_skipped_phases(&results.skipped_phases);
    print_overall_summary(results);
    println!();
}

/// How a phase went, shown as soon as it ends. Each failure's cause is cut
/// to fit `width` columns, and the lines leading up to it follow:
///
/// ```text
/// ✗ Phase p2 (1/3 failed in 0:52)
///   ✓ boron (switch)
///   ✗ cerium at activate: warning: the following units failed: storage-agent.service
///     activating the configuration...
///     warning: the following units failed: storage-agent.service
///     … 9 more lines in /tmp/snow-1762255380.log
/// ```
pub fn phase_report(
    phase: &snowlib::PhaseResult,
    among_other_phases: bool,
    width: usize,
    log_file: Option<&Path>,
) -> String {
    let mut lines = vec![phase_heading(phase, among_other_phases)];
    for host in &phase.hosts {
        lines.extend(host_lines(host, width, log_file));
    }
    lines.join("\n")
}

fn phase_heading(phase: &snowlib::PhaseResult, among_other_phases: bool) -> String {
    let title = phase_title(&phase.phase, among_other_phases);
    let took = format_elapsed(phase.duration);
    let total = phase.hosts.len();
    if phase.all_succeeded() {
        format!(
            "{} {}",
            success_headline(&title),
            style(format!("({total} {} in {took})", plural(total, "host"))).dim()
        )
    } else {
        let headline = if phase.has_blocking_failure() {
            failure_headline(&title)
        } else {
            warning_headline(&title)
        };
        format!(
            "{headline} {}",
            style(format!(
                "({}/{total} failed in {took})",
                phase.failure_count()
            ))
            .dim()
        )
    }
}

fn success_headline(text: &str) -> String {
    format!("{} {}", marks::success().bold(), style(text).green().bold())
}

fn failure_headline(text: &str) -> String {
    format!("{} {}", marks::failure().bold(), style(text).red().bold())
}

/// For failures that don't fail the push: those of optional hosts
fn warning_headline(text: &str) -> String {
    format!("{} {}", marks::warning(), style(text).yellow().bold())
}

fn host_label(host: &snowlib::HostDeploymentResult) -> String {
    if host.optional {
        format!("{} (optional)", host.hostname)
    } else {
        host.hostname.clone()
    }
}

pub(super) fn success_labels(host: &snowlib::HostDeploymentResult) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if let Some(mode) = host.activation_mode {
        labels.push(mode_label(mode));
    }
    if host.rebooted {
        labels.push("rebooted");
    }
    if host.unchanged {
        labels.push("unchanged");
    }
    labels
}

fn host_lines(
    host: &snowlib::HostDeploymentResult,
    width: usize,
    log_file: Option<&Path>,
) -> Vec<String> {
    if host.success {
        vec![format!(
            "{}{}",
            " ".repeat(HOST_INDENT),
            success_line(&host.hostname, &success_labels(host))
        )]
    } else if host.is_skipped() {
        let reason = host.error_message.as_deref().unwrap_or_default();
        vec![format!(
            "  {}",
            marks::skipped_line(&format!("{} skipped, {reason}", host_label(host)))
        )]
    } else {
        let details = host
            .error_message
            .as_deref()
            .map(|error| failure_details(error, log_file))
            .unwrap_or_default();
        std::iter::once(format!(
            "{}{}",
            " ".repeat(HOST_INDENT),
            failure_line(host, width.saturating_sub(HOST_INDENT))
        ))
        .chain(details.into_iter().map(|line| format!("    {line}")))
        .collect()
    }
}

pub(in crate::commands) fn mode_label(mode: snowlib::ActivationMode) -> &'static str {
    use snowlib::ActivationMode;
    match mode {
        ActivationMode::Switch => "switch",
        ActivationMode::Boot => "boot",
        ActivationMode::Test => "test",
        ActivationMode::DryActivate => "dry-run",
    }
}

/// "✓ boron (switch, rebooted)"
pub(in crate::commands) fn success_line(hostname: &str, labels: &[&str]) -> String {
    let status = if labels.is_empty() {
        String::new()
    } else {
        format!(" {}", style(format!("({})", labels.join(", "))).dim())
    };
    format!("{} {hostname}{status}", marks::success())
}

/// What a phase's report indents its hosts by
const HOST_INDENT: usize = 2;

/// "✗ cerium at activate: <cause>", the cause cut to fit `width` columns
fn failure_line(host: &snowlib::HostDeploymentResult, width: usize) -> String {
    let line = failed_host(&host_label(host), host.failed_at_step);
    match host.error_message.as_deref().map(error_summary) {
        Some(cause) if !cause.is_empty() => {
            // Some terminals wrap a line that reaches the last column
            let room = width.saturating_sub(console::measure_text_width(&line) + ": ".len() + 1);
            format!("{line}: {}", truncate(cause, room))
        }
        _ => line,
    }
}

/// "✗ cerium at activate"
pub(in crate::commands) fn failed_host(
    hostname: &str,
    step: Option<snowlib::PipelineStep>,
) -> String {
    let step = step.map(|step| format!(" at {step}")).unwrap_or_default();
    format!("{} {hostname}{}", marks::failure(), style(step).red())
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
        // Only a phase after another can be skipped
        phase_title(&skipped.phase, true),
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
        return success_headline(&format!(
            "{succeeded} {} deployed",
            plural(succeeded, "host")
        ));
    }

    let optional = results.optional_failure_count();
    let failed_label = match optional {
        0 => "failed".to_string(),
        n if n == failed => "failed (optional)".to_string(),
        n => format!("failed ({n} optional)"),
    };
    let mut text = format!("{succeeded}/{} deployed", results.total_count());
    for (count, label) in [(failed, failed_label.as_str()), (skipped, "skipped")] {
        if count > 0 {
            text.push_str(&format!(", {count} {label}"));
        }
    }
    if failed > optional {
        failure_headline(&text)
    } else {
        warning_headline(&text)
    }
}

/// The error's own details, or else the lines leading up to the cause,
/// dim but for the cause itself, then where to read the rest. A cause on
/// its own is left out, since the line above already ends with it.
fn failure_details(error: &str, log_file: Option<&Path>) -> Vec<String> {
    let dim = |line: &&str| style(line).dim().to_string();
    let mut details = Vec::new();
    let context = match error_details(error, MAX_CONTEXT_LINES) {
        Some(own) => {
            details.extend(own.lines.iter().map(dim));
            own
        }
        None => {
            let context = error_context(error, MAX_CONTEXT_LINES);
            if let [leading @ .., cause] = context.lines.as_slice() {
                if !leading.is_empty() {
                    details.extend(leading.iter().map(dim));
                    details.push(cause.to_string());
                }
            }
            context
        }
    };
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

    #[test]
    fn test_overview_lists_hosts_without_a_phase_last_as_no_phase() {
        console::set_colors_enabled(false);
        assert_eq!(
            phase_overview(&[
                phase("p2", &["ferrum"]),
                phase("default", &["boron", "radon"])
            ]),
            "Pushing 3 hosts in 2 phases:\n  p2          ferrum\n  (no phase)  boron, radon"
        );
    }

    #[test]
    fn test_hosts_without_a_phase_after_named_phases_are_titled_so() {
        assert_eq!(super::phase_title("default", false), "Push");
        assert_eq!(super::phase_title("default", true), "Hosts without a phase");
        assert_eq!(super::phase_title("_all", false), "Push");
        assert_eq!(super::phase_title("p2", true), "Phase p2");
        assert_eq!(super::phase_title("p2", false), "Phase p2");
    }

    #[test]
    fn test_report_of_hosts_without_a_phase_after_named_phases_is_titled_so() {
        console::set_colors_enabled(false);
        let phase = PhaseResult {
            phase: "default".to_string(),
            duration: Duration::from_secs(41),
            hosts: vec![make_successful_host(
                "sodium",
                ActivationMode::Switch,
                false,
            )],
        };

        assert_eq!(
            super::phase_report(&phase, true, 100, None),
            "✓ Hosts without a phase (1 host in 0:41)\n  ✓ sodium (switch)"
        );
    }

    #[test]
    fn test_skipped_hosts_without_a_phase_are_titled_so() {
        console::set_colors_enabled(false);
        let skipped = SkippedPhase {
            phase: "default".to_string(),
            hostnames: vec!["sodium".to_string()],
        };

        assert_eq!(
            super::skipped_phase_report(&skipped),
            "⊘ Hosts without a phase (1 host skipped)\n  ⊘ sodium"
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
            optional: false,
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
            optional: false,
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
            super::phase_report(&phase, false, 100, None),
            "✗ Phase p2 (3/4 failed in 0:52)\n  ✓ boron (switch)\n  ✗ cerium at activate: activation failed\n  ✗ radon at pre-deploy: storage degraded\n  ⊘ lanthanum skipped, serial group 'k8s' halted after cerium failed"
        );
    }

    #[test]
    fn test_failure_line_cuts_the_cause_to_the_terminal() {
        console::set_colors_enabled(false);
        let error = "Command failed\n\n\
                     Command: ssh cerium sudo snowglobe-activate /nix/store/yzxw-cerium switch\n\
                     Exit code: 4\n\n\
                     Stderr:\n\
                     activating the configuration...\n\
                     warning: the following units failed: storage-agent.service";
        let cerium = make_failed_host("cerium", PipelineStep::Activate, error);

        assert_eq!(
            super::failure_line(&cerium, 100),
            "✗ cerium at activate: warning: the following units failed: storage-agent.service"
        );
        assert_eq!(
            super::failure_line(&cerium, 48),
            "✗ cerium at activate: warning: the following u…"
        );
    }

    #[test]
    fn test_phase_report_shows_what_led_to_a_failure_under_its_host() {
        console::set_colors_enabled(false);
        let error = "Command failed\n\n\
                     Command: ssh sudo snowglobe-activate /nix/store/yzxw-cerium switch\n\
                     Exit code: 4\n\n\
                     Stderr:\n\
                     activating the configuration...\n\
                     warning: the following units failed: storage-agent.service\n\n\
                     Stdout:\n\
                     Nov 04 10:12:04 cerium storage-agent[812]: disk sdb missing";
        let phase = PhaseResult {
            phase: "p2".to_string(),
            duration: Duration::from_secs(52),
            hosts: vec![
                make_successful_host("boron", ActivationMode::Switch, false),
                make_failed_host("cerium", PipelineStep::Activate, error),
                make_failed_host("radon", PipelineStep::PreDeploy, "storage degraded"),
                HostDeploymentResult::skipped_by_serial_group("lanthanum", "k8s", "cerium"),
            ],
        };

        assert_eq!(
            super::phase_report(&phase, false, 100, Some(Path::new("/tmp/snow-1762255380.log"))),
            "✗ Phase p2 (3/4 failed in 0:52)\n\
             \x20 ✓ boron (switch)\n\
             \x20 ✗ cerium at activate: warning: the following units failed: storage-agent.service\n\
             \x20   Command: ssh sudo snowglobe-activate /nix/store/yzxw-cerium switch\n\
             \x20   activating the configuration...\n\
             \x20   warning: the following units failed: storage-agent.service\n\
             \x20   … 5 more lines in /tmp/snow-1762255380.log\n\
             \x20 ✗ radon at pre-deploy: storage degraded\n\
             \x20 ⊘ lanthanum skipped, serial group 'k8s' halted after cerium failed"
        );
    }

    #[test]
    fn test_phase_report_lists_an_errors_own_details_under_its_cause() {
        console::set_colors_enabled(false);
        let error = "activated, but 2 units are failing\n\
                     Details:\n\
                     nginx.service\n\
                     unifi.service (already failing before this push)";
        let phase = PhaseResult {
            phase: "p1".to_string(),
            duration: Duration::from_secs(41),
            hosts: vec![make_failed_host("cerium", PipelineStep::Activate, error)],
        };

        assert_eq!(
            super::phase_report(&phase, false, 100, None),
            "✗ Phase p1 (1/1 failed in 0:41)\n\
             \x20 ✗ cerium at activate: activated, but 2 units are failing\n\
             \x20   nginx.service\n\
             \x20   unifi.service (already failing before this push)"
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

        assert_eq!(super::failure_line(&cerium, 100), "✗ cerium at copy");
        assert_eq!(super::failure_line(&radon, 100), "✗ radon at build");
        cerium.failed_at_step = None;
        assert_eq!(super::failure_line(&cerium, 100), "✗ cerium");
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
                super::phase_report(&phase, false, 100, None),
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
                "Command: ssh sudo snowglobe-activate /nix/store/yzxw-cerium switch",
                "activating the configuration...",
                "warning: the following units failed: storage-agent.service",
                "… 3 more lines in /tmp/snow-1762255380.log",
            ]
        );
        assert!(super::failure_details("worker crashed", None).is_empty());
        assert_eq!(
            super::failure_details(
                "Command failed\n\nCommand: true\nExit code: 1\n\nStderr:\n",
                None
            )
            .iter()
            .map(|line| console::strip_ansi_codes(line))
            .collect::<Vec<_>>(),
            vec!["… 3 more lines in the log on stderr"]
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

    fn failed_optional_host(name: &str) -> HostDeploymentResult {
        HostDeploymentResult {
            optional: true,
            ..make_failed_host(
                name,
                PipelineStep::Check,
                "ssh: connect to host curium port 22: Connection timed out",
            )
        }
    }

    #[test]
    fn test_phase_report_marks_a_failed_optional_host_as_a_warning() {
        console::set_colors_enabled(false);
        let phase = PhaseResult {
            phase: "p1".to_string(),
            duration: Duration::from_secs(41),
            hosts: vec![
                make_successful_host("boron", ActivationMode::Switch, false),
                failed_optional_host("curium"),
            ],
        };

        assert_eq!(
            super::phase_report(&phase, true, 100, None),
            "! Phase p1 (1/2 failed in 0:41)\n\
             \x20 ✓ boron (switch)\n\
             \x20 ✗ curium (optional) at check: ssh: connect to host curium port 22: Connection timed out"
        );
    }

    #[test]
    fn test_phase_report_with_a_failed_host_beside_an_optional_one_is_a_failure() {
        console::set_colors_enabled(false);
        let phase = PhaseResult {
            phase: "p1".to_string(),
            duration: Duration::from_secs(41),
            hosts: vec![
                failed_optional_host("curium"),
                make_failed_host("radon", PipelineStep::Build, "build failed"),
            ],
        };

        assert!(super::phase_report(&phase, true, 100, None).starts_with("✗ Phase p1 (2/2 failed"));
    }

    #[test]
    fn test_overall_summary_says_which_failures_were_optional() {
        console::set_colors_enabled(false);
        let pushed = |hosts: Vec<HostDeploymentResult>| PushResults {
            phases: vec![PhaseResult {
                hosts,
                ..deployed_phase(&[])
            }],
            skipped_phases: vec![],
        };
        let boron = || make_successful_host("boron", ActivationMode::Switch, false);
        let radon = make_failed_host("radon", PipelineStep::Activate, "activation failed");

        assert_eq!(
            super::overall_summary(&pushed(vec![boron(), failed_optional_host("curium")])),
            "! 1/2 deployed, 1 failed (optional)"
        );
        assert_eq!(
            super::overall_summary(&pushed(vec![
                boron(),
                failed_optional_host("curium"),
                radon
            ])),
            "✗ 1/3 deployed, 2 failed (1 optional)"
        );
    }

    #[test]
    fn test_skipped_optional_host_is_marked_optional() {
        console::set_colors_enabled(false);
        let curium = HostDeploymentResult {
            optional: true,
            ..HostDeploymentResult::skipped_by_serial_group("curium", "k8s", "radon")
        };

        assert_eq!(
            super::host_lines(&curium, 100, None),
            vec!["  ⊘ curium (optional) skipped, serial group 'k8s' halted after radon failed"]
        );
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
