//! Terminal output formatting for deployment results
//!
//! Handles each phase's report and the final summary.
//! Keeps ANSI terminal formatting out of the library layer
//! (library should be reusable by TUI, agents, etc.).

use super::board::{format_elapsed, plural};
use super::colors::style;
use crate::marks;

/// Maximum error lines to display before truncating.
/// Limited to keep terminal output scannable - full errors are in logs.
const MAX_ERROR_LINES: usize = 10;

/// The phase hosts without a push phase share, which has no name to show
pub(super) fn is_unnamed_phase(phase: &str) -> bool {
    matches!(phase, "default" | "_all")
}

/// "Phase p2", or `unnamed` for the phase without a name
pub(super) fn phase_title(phase: &str, unnamed: &str) -> String {
    if is_unnamed_phase(phase) {
        unnamed.to_string()
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
pub fn print_summary(results: &snowlib::PushResults) {
    print_skipped_phases(&results.skipped_phases);
    print_overall_summary(results);
    print_error_details(results);
    println!();
}

/// How a phase went, shown as soon as it ends:
///
/// ```text
/// ✗ Phase p2 (1/3 failed in 0:52)
///   ✓ boron (switch)
///   ✗ cerium at activate
/// ```
pub fn phase_report(phase: &snowlib::PhaseResult) -> String {
    let mut lines = vec![phase_heading(phase)];
    lines.extend(phase.hosts.iter().map(host_line));
    lines.join("\n")
}

fn phase_heading(phase: &snowlib::PhaseResult) -> String {
    let title = phase_title(&phase.phase, "Deploy");
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

fn host_line(host: &snowlib::HostDeploymentResult) -> String {
    if host.success {
        let labels = success_labels(host);
        let status = if labels.is_empty() {
            String::new()
        } else {
            format!(" {}", style(format!("({})", labels.join(", "))).dim())
        };
        format!("  {} {}{status}", marks::success(), host.hostname)
    } else {
        let step = host
            .failed_at_step
            .map(|step| format!(" at {step}"))
            .unwrap_or_default();
        format!(
            "  {} {}{}",
            marks::failure(),
            host.hostname,
            style(step).red()
        )
    }
}

fn print_skipped_phases(skipped_phases: &[snowlib::SkippedPhase]) {
    for skipped in skipped_phases {
        println!(
            "{} {}",
            style(format!("⊘ Phase {}", skipped.phase)).yellow(),
            style(format!("({} hosts skipped)", skipped.hostnames.len())).dim()
        );
        for hostname in &skipped.hostnames {
            println!("  {}", style(format!("⊘ {}", hostname)).dim());
        }
        println!();
    }
}

fn print_overall_summary(results: &snowlib::PushResults) {
    let total = results.total_count();
    let succeeded = results.success_count();
    let failed = results.failure_count();
    let skipped = results.skipped_count();

    if failed == 0 && skipped == 0 {
        println!(
            "{}",
            style(format!("✨ All {} hosts deployed successfully", succeeded))
                .green()
                .bold()
        );
    } else if skipped > 0 {
        println!(
            "{}",
            style(format!(
                "⚠ {}/{} deployed, {} skipped",
                succeeded, total, skipped
            ))
            .yellow()
            .bold()
        );
    } else {
        println!(
            "{}",
            style(format!(
                "✗ {}/{} deployed, {} failed",
                succeeded, total, failed
            ))
            .red()
            .bold()
        );
    }
}

fn print_error_details(results: &snowlib::PushResults) {
    let errors = collect_failed_hosts(results);
    if errors.is_empty() {
        return;
    }

    println!();
    println!("{}", style("Errors:").red().bold());

    for host in errors {
        if let Some(error_msg) = &host.error_message {
            print_error_header(host);
            print_error_lines(error_msg);
        }
    }
}

fn collect_failed_hosts(results: &snowlib::PushResults) -> Vec<&snowlib::HostDeploymentResult> {
    results
        .phases
        .iter()
        .flat_map(|p| &p.hosts)
        .filter(|h| !h.success && h.error_message.is_some())
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

fn print_error_lines(error_msg: &str) {
    for line in error_lines(error_msg, MAX_ERROR_LINES) {
        println!("    {}", style(line).dim());
    }
}

/// The first line says what failed and the last ones usually why, so long
/// errors keep both ends and leave out the middle
fn error_lines(error_msg: &str, max: usize) -> Vec<String> {
    let lines: Vec<&str> = error_msg.lines().collect();
    if lines.len() <= max {
        return lines.iter().map(|line| line.to_string()).collect();
    }
    let tail = max.saturating_sub(1);
    let hidden = lines.len() - 1 - tail;
    std::iter::once(lines[0].to_string())
        .chain(std::iter::once(format!(
            "… {hidden} more {}, see logs",
            plural(hidden, "line")
        )))
        .chain(
            lines[lines.len() - tail..]
                .iter()
                .map(|line| line.to_string()),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::phase_overview;
    use snowlib::{
        ActivationMode, HostDeploymentResult, PhaseResult, PipelineStep, PushResults, SkippedPhase,
    };
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
            ],
        };

        assert_eq!(
            super::phase_report(&phase),
            "✗ Phase p2 (2/3 failed in 0:52)\n  ✓ boron (switch)\n  ✗ cerium at activate\n  ✗ radon at pre-deploy"
        );
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
                super::phase_report(&phase),
                "✓ Deploy (1 host in 1:02:07)\n  ✓ sodium (switch)"
            );
        }
    }

    #[test]
    fn test_long_errors_keep_their_first_and_last_lines() {
        let error = (1..=12)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            super::error_lines(&error, 5),
            vec![
                "line 1",
                "… 7 more lines, see logs",
                "line 9",
                "line 10",
                "line 11",
                "line 12"
            ]
        );
        assert_eq!(
            super::error_lines("line 1\nline 2", 5),
            vec!["line 1", "line 2"]
        );
        assert_eq!(super::error_lines(&error, 12).len(), 12);
        let one_hidden = super::error_lines(&error, 11);
        assert_eq!(one_hidden[1], "… 1 more line, see logs");
        assert_eq!(one_hidden[2], "line 3");
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
