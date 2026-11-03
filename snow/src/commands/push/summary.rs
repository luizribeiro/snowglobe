//! Terminal output formatting for deployment results
//!
//! Handles the final summary printed after deployment completes.
//! Keeps ANSI terminal formatting out of the library layer
//! (library should be reusable by TUI, agents, etc.).

use super::colors::style;

/// Maximum error lines to display before truncating.
/// Limited to keep terminal output scannable - full errors are in logs.
const MAX_ERROR_LINES: usize = 10;

/// Which hosts each phase will push, so a host nobody expected shows up
/// before anything happens
pub fn phase_overview(phases: &[(String, Vec<String>)]) -> String {
    let total: usize = phases.iter().map(|(_, hosts)| hosts.len()).sum();
    let hosts_word = if total == 1 { "host" } else { "hosts" };
    match phases {
        [(phase, hosts)] if phase == "default" || phase == "_all" => {
            format!("Pushing {total} {hosts_word}: {}", hosts.join(", "))
        }
        _ => {
            let width = phases
                .iter()
                .map(|(phase, _)| phase.len())
                .max()
                .unwrap_or(0);
            let phases_word = if phases.len() == 1 { "phase" } else { "phases" };
            let mut overview = format!(
                "Pushing {total} {hosts_word} in {} {phases_word}:",
                phases.len()
            );
            for (phase, hosts) in phases {
                overview.push_str(&format!("\n  {phase:<width$}  {}", hosts.join(", ")));
            }
            overview
        }
    }
}

pub fn print_summary(results: &snowlib::PushResults) {
    println!();
    print_phase_results(&results.phases);
    print_skipped_phases(&results.skipped_phases);
    print_overall_summary(results);
    print_error_details(results);
    println!();
}

fn print_phase_results(phases: &[snowlib::PhaseResult]) {
    for phase_result in phases {
        print_phase_header(phase_result);
        print_host_results(&phase_result.hosts);
        println!();
    }
}

fn print_phase_header(phase: &snowlib::PhaseResult) {
    let total = phase.hosts.len();
    if phase.all_succeeded() {
        println!(
            "{} {}",
            style(format!("✓ Phase {}", phase.phase)).green().bold(),
            style(format!("({} hosts)", total)).dim()
        );
    } else {
        let failed = phase.failure_count();
        println!(
            "{} {}",
            style(format!("✗ Phase {}", phase.phase)).red().bold(),
            style(format!("({}/{} failed)", failed, total)).dim()
        );
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

fn print_host_results(hosts: &[snowlib::HostDeploymentResult]) {
    for host in hosts {
        if host.success {
            let parts = success_labels(host);

            let status = if parts.is_empty() {
                String::new()
            } else {
                format!(" {}", style(format!("({})", parts.join(", "))).dim())
            };

            println!("  {} {}{}", style("✓").green(), host.hostname, status);
        } else {
            let step_msg = host
                .failed_at_step
                .map(|step| format!(" at {}", step))
                .unwrap_or_default();
            println!(
                "  {} {}{}",
                style("✗").red(),
                host.hostname,
                style(step_msg).red()
            );
        }
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
    let lines: Vec<&str> = error_msg.lines().collect();
    let (lines_to_show, remaining) = truncate_lines(&lines, MAX_ERROR_LINES);

    for line in lines_to_show {
        println!("    {}", style(line).dim());
    }

    if remaining > 0 {
        println!(
            "    {}",
            style(format!("... ({} more lines, see logs)", remaining)).dim()
        );
    }
}

fn truncate_lines<'a>(lines: &'a [&'a str], max: usize) -> (&'a [&'a str], usize) {
    if lines.len() > max {
        (&lines[..max], lines.len() - max)
    } else {
        (lines, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::phase_overview;
    use snowlib::{
        ActivationMode, HostDeploymentResult, PhaseResult, PipelineStep, PushResults, SkippedPhase,
    };

    fn phase(name: &str, hosts: &[&str]) -> (String, Vec<String>) {
        (
            name.to_string(),
            hosts.iter().map(|h| h.to_string()).collect(),
        )
    }

    #[test]
    fn test_overview_lists_each_phase_with_its_hosts() {
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
                    hosts: vec![make_successful_host("stg01", ActivationMode::Switch, false)],
                },
                PhaseResult {
                    phase: "production".to_string(),
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
