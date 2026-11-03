//! Writes each host's progress to the log

use super::{HostEvent, PushObserver};
use crate::push::{HostDeploymentResult, PipelineStep};
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
    match (result.failed_at_step, &result.error_message) {
        (Some(PipelineStep::SerialGroup), Some(reason)) => Some(format!("{hostname}: {reason}")),
        _ => None,
    }
}

impl PushObserver for LogObserver {
    fn on_host_event(&self, hostname: &str, event: &HostEvent) {
        if let Some(line) = self.line(hostname, event, Instant::now()) {
            tracing::info!("{line}");
        }
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
            reason: "Failed to deploy to host 'cerium': Copy closure failed".to_string(),
        }
    }

    #[test]
    fn test_finished_step_logs_how_long_it_took() {
        let log = LogObserver::default();
        let t0 = Instant::now();

        let started = log.line("cerium", &HostEvent::Started(PipelineStep::Copy), t0);
        let finished = log.line(
            "cerium",
            &HostEvent::Finished(PipelineStep::Copy),
            t0 + Duration::from_millis(12_340),
        );

        assert_eq!(started.as_deref(), Some("cerium: copy started"));
        assert_eq!(finished.as_deref(), Some("cerium: copy finished in 12.3s"));
    }

    #[test]
    fn test_steps_of_different_hosts_are_timed_separately() {
        let log = LogObserver::default();
        let t0 = Instant::now();

        log.line("cerium", &HostEvent::Started(PipelineStep::Build), t0);
        log.line(
            "radon",
            &HostEvent::Started(PipelineStep::Build),
            t0 + Duration::from_secs(30),
        );
        let radon = log.line(
            "radon",
            &HostEvent::Finished(PipelineStep::Build),
            t0 + Duration::from_secs(40),
        );
        let cerium = log.line(
            "cerium",
            &HostEvent::Finished(PipelineStep::Build),
            t0 + Duration::from_secs(45),
        );

        assert_eq!(radon.as_deref(), Some("radon: build finished in 10.0s"));
        assert_eq!(cerium.as_deref(), Some("cerium: build finished in 45.0s"));
    }

    #[test]
    fn test_failures_are_left_to_the_pipeline_and_forget_their_start() {
        let log = LogObserver::default();
        let t0 = Instant::now();

        log.line("cerium", &HostEvent::Started(PipelineStep::Copy), t0);
        log.line("radon", &HostEvent::Started(PipelineStep::Copy), t0);
        let failure = log.line("cerium", &failed(PipelineStep::Copy), t0);
        let after_failure = log.line("cerium", &HostEvent::Finished(PipelineStep::Copy), t0);
        let radon = log.line(
            "radon",
            &HostEvent::Finished(PipelineStep::Copy),
            t0 + Duration::from_secs(3),
        );

        assert_eq!(failure, None);
        assert_eq!(after_failure.as_deref(), Some("cerium: copy finished"));
        assert_eq!(radon.as_deref(), Some("radon: copy finished in 3.0s"));
    }

    #[test]
    fn test_done_logs_deploys_and_serial_group_skips_only() {
        let log = LogObserver::default();
        let now = Instant::now();
        let deployed = HostDeploymentResult {
            hostname: "radon".to_string(),
            success: true,
            failed_at_step: None,
            activation_mode: Some(ActivationMode::Boot),
            rebooted: true,
            unchanged: false,
            error_message: None,
        };
        let skipped = HostDeploymentResult::skipped_by_serial_group("samarium", "k8s", "cerium");
        let failed = HostDeploymentResult {
            hostname: "cerium".to_string(),
            success: false,
            failed_at_step: Some(PipelineStep::Copy),
            error_message: Some("Copy closure failed".to_string()),
            ..deployed.clone()
        };
        let unchanged = HostDeploymentResult::unchanged("sodium");

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
                Some("radon: deployed".to_string()),
                Some(
                    "samarium: skipped: serial group 'k8s' halted after cerium failed".to_string()
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
            ("sodium", HostEvent::Unchanged),
            (
                "neodymium",
                HostEvent::WaitingForSerialGroup {
                    group: "k8s".to_string(),
                    holder: Some("cerium".to_string()),
                },
            ),
            (
                "samarium",
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
                "sodium: already running the built system, skipping",
                "neodymium: waiting for serial group 'k8s' (held by cerium)",
                "samarium: waiting for serial group 'k8s' (held by another host)",
            ]
        );
    }
}
