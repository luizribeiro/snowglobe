//! Writes each host's progress to the log

use super::{DeployObserver, HostEvent};
use crate::deploy::{HostDeploymentResult, PhaseResult, PipelineStep};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::Instant;

/// Logs every step a host starts and finishes, with how long it took
///
/// Failures are not logged here: the pipeline logs them with their reason,
/// including hook failures, which are not steps.
#[derive(Default)]
pub struct LogObserver {
    started: Mutex<HashMap<(String, PipelineStep), Instant>>,
}

impl LogObserver {
    fn line(&self, hostname: &str, event: &HostEvent, now: Instant) -> Option<String> {
        let mut started = self.started.lock();
        match event {
            HostEvent::Started(step) => {
                started.insert((hostname.to_string(), *step), now);
                Some(format!("{hostname}: {step} started"))
            }
            HostEvent::Finished(step) => {
                Some(match started.remove(&(hostname.to_string(), *step)) {
                    Some(at) => format!("{hostname}: {step} finished in {:.1?}", now - at),
                    None => format!("{hostname}: {step} finished"),
                })
            }
            HostEvent::Failed { step, .. } => {
                started.remove(&(hostname.to_string(), *step));
                None
            }
            HostEvent::Unchanged => Some(format!(
                "{hostname}: already running the built system, skipping"
            )),
            HostEvent::WaitingForSerialGroup { group, holder } => Some(format!(
                "{hostname}: waiting for serial group '{group}' (held by {})",
                holder.as_deref().unwrap_or("another host")
            )),
            HostEvent::Done(result) => done_line(result),
        }
    }
}

/// Failures are logged by the pipeline and unchanged hosts by `Unchanged`
fn done_line(result: &HostDeploymentResult) -> Option<String> {
    let hostname = &result.hostname;
    if result.success && !result.unchanged {
        return Some(format!("{hostname}: deployed"));
    }
    match &result.error_message {
        Some(reason) if result.is_skipped() => Some(format!("{hostname}: skipped, {reason}")),
        _ => None,
    }
}

fn phase_line(phase: &str, result: Option<&PhaseResult>) -> String {
    match result {
        Some(result) => format!(
            "phase {phase} finished in {:.1?}: {} succeeded, {} failed",
            result.duration,
            result.success_count(),
            result.failure_count()
        ),
        None => format!("phase {phase} stopped on an error"),
    }
}

impl DeployObserver for LogObserver {
    fn on_host_event(&self, hostname: &str, event: &HostEvent) {
        if let Some(line) = self.line(hostname, event, Instant::now()) {
            tracing::info!("{line}");
        }
    }

    fn on_phase_complete(&self, phase: &str, result: Option<&PhaseResult>) {
        tracing::info!("{}", phase_line(phase, result));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nix::ActivationMode;
    use std::time::Duration;

    fn failed(step: PipelineStep) -> HostEvent {
        HostEvent::Failed {
            step,
            reason: "Copy closure failed".to_string(),
        }
    }

    #[test]
    fn test_finished_phase_logs_how_long_it_took_and_how_it_went() {
        let mut failed = HostDeploymentResult::unchanged("raven");
        failed.success = false;
        let result = PhaseResult {
            phase: "p2".to_string(),
            hosts: vec![HostDeploymentResult::unchanged("cactus"), failed],
            duration: Duration::from_millis(83_460),
        };

        assert_eq!(
            phase_line("p2", Some(&result)),
            "phase p2 finished in 83.5s: 1 succeeded, 1 failed"
        );
        assert_eq!(phase_line("p3", None), "phase p3 stopped on an error");
    }

    #[test]
    fn test_finished_step_logs_how_long_it_took() {
        let log = LogObserver::default();
        let t0 = Instant::now();

        let started = log.line("cactus", &HostEvent::Started(PipelineStep::Copy), t0);
        let finished = log.line(
            "cactus",
            &HostEvent::Finished(PipelineStep::Copy),
            t0 + Duration::from_millis(12_340),
        );

        assert_eq!(started.as_deref(), Some("cactus: copy started"));
        assert_eq!(finished.as_deref(), Some("cactus: copy finished in 12.3s"));
    }

    #[test]
    fn test_steps_of_different_hosts_are_timed_separately() {
        let log = LogObserver::default();
        let t0 = Instant::now();

        log.line("cactus", &HostEvent::Started(PipelineStep::Build), t0);
        log.line(
            "raven",
            &HostEvent::Started(PipelineStep::Build),
            t0 + Duration::from_secs(30),
        );
        let raven = log.line(
            "raven",
            &HostEvent::Finished(PipelineStep::Build),
            t0 + Duration::from_secs(40),
        );
        let cactus = log.line(
            "cactus",
            &HostEvent::Finished(PipelineStep::Build),
            t0 + Duration::from_secs(45),
        );

        assert_eq!(raven.as_deref(), Some("raven: build finished in 10.0s"));
        assert_eq!(cactus.as_deref(), Some("cactus: build finished in 45.0s"));
    }

    #[test]
    fn test_failures_are_left_to_the_pipeline_and_forget_their_start() {
        let log = LogObserver::default();
        let t0 = Instant::now();

        log.line("cactus", &HostEvent::Started(PipelineStep::Copy), t0);
        log.line("raven", &HostEvent::Started(PipelineStep::Copy), t0);
        let failure = log.line("cactus", &failed(PipelineStep::Copy), t0);
        let after_failure = log.line("cactus", &HostEvent::Finished(PipelineStep::Copy), t0);
        let raven = log.line(
            "raven",
            &HostEvent::Finished(PipelineStep::Copy),
            t0 + Duration::from_secs(3),
        );

        assert_eq!(failure, None);
        assert_eq!(after_failure.as_deref(), Some("cactus: copy finished"));
        assert_eq!(raven.as_deref(), Some("raven: copy finished in 3.0s"));
    }

    #[test]
    fn test_done_logs_deploys_and_serial_group_skips_only() {
        let log = LogObserver::default();
        let now = Instant::now();
        let deployed = HostDeploymentResult {
            hostname: "raven".to_string(),
            success: true,
            failed_at_step: None,
            activation_mode: Some(ActivationMode::Boot),
            rebooted: true,
            unchanged: false,
            optional: false,
            ignored_failing: Vec::new(),
            error_message: None,
        };
        let skipped = HostDeploymentResult::skipped_by_serial_group("seahorse", "k8s", "cactus");
        let failed = HostDeploymentResult {
            hostname: "cactus".to_string(),
            success: false,
            failed_at_step: Some(PipelineStep::Copy),
            error_message: Some("Copy closure failed".to_string()),
            ..deployed.clone()
        };
        let unchanged = HostDeploymentResult::unchanged("spruce");

        let lines: Vec<_> = [deployed, skipped, failed, unchanged]
            .into_iter()
            .map(|result| {
                let hostname = result.hostname.clone();
                log.line(&hostname, &HostEvent::Done(result), now)
            })
            .collect();

        assert_eq!(
            lines,
            vec![
                Some("raven: deployed".to_string()),
                Some(
                    "seahorse: skipped, serial group 'k8s' halted after cactus failed".to_string()
                ),
                None,
                None,
            ]
        );
    }

    #[test]
    fn test_unchanged_hosts_and_serial_waits_are_logged() {
        let log = LogObserver::default();
        let now = Instant::now();

        let lines: Vec<_> = [
            ("spruce", HostEvent::Unchanged),
            (
                "nightfall",
                HostEvent::WaitingForSerialGroup {
                    group: "k8s".to_string(),
                    holder: Some("cactus".to_string()),
                },
            ),
            (
                "seahorse",
                HostEvent::WaitingForSerialGroup {
                    group: "k8s".to_string(),
                    holder: None,
                },
            ),
        ]
        .iter()
        .map(|(host, event)| log.line(host, event, now).unwrap())
        .collect();

        assert_eq!(
            lines,
            vec![
                "spruce: already running the built system, skipping",
                "nightfall: waiting for serial group 'k8s' (held by cactus)",
                "seahorse: waiting for serial group 'k8s' (held by another host)",
            ]
        );
    }
}
