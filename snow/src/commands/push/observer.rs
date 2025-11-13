//! CLI observer implementation for push deployment progress reporting
//!
//! Manages the complete span hierarchy for deployment progress tracking,
//! including phase, step, and host-level spans.

use parking_lot::Mutex;
use snowlib::{ActivationMode, DeployResult, PushObserver, Result};
use std::collections::HashMap;
use tracing::info_span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

/// CLI observer that manages the complete span hierarchy
///
/// Creates and manages phase, step, and host spans internally to provide
/// structured progress reporting. All span creation happens inside the observer,
/// keeping the library code free of tracing span details.
///
/// Uses a mutex to protect shared state since observer methods are called from
/// Rayon worker threads in the parallel-per-host execution model.
pub struct CliObserver {
    state: Mutex<ObserverState>,
}

struct ObserverState {
    // Phase span - created in on_phase_start()
    phase_span: Option<tracing::Span>,

    // Step spans - pre-created in on_phase_start() for consistent ordering
    // These are NOT entered - they exist only as parents for host spans
    build_span: Option<tracing::Span>,
    copy_span: Option<tracing::Span>,
    activate_span: Option<tracing::Span>,
    reboot_span: Option<tracing::Span>,

    // Host spans - created when host starts a step, entered temporarily for logging
    // Key is hostname, value is the span (not entered - we enter/exit as needed)
    host_spans: HashMap<String, tracing::Span>,
}

impl CliObserver {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ObserverState {
                phase_span: None,
                build_span: None,
                copy_span: None,
                activate_span: None,
                reboot_span: None,
                host_spans: HashMap::new(),
            }),
        }
    }
}

impl PushObserver for CliObserver {
    fn on_phase_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        use tracing_indicatif::style::ProgressStyle;

        let mut state = self.state.lock();

        // Create phase span
        let phase_span = info_span!("phase", phase = phase, hosts = hosts.len());

        // Log phase start within the span
        let _guard = phase_span.enter();
        if phase == "default" || phase == "_all" {
            tracing::info!(hosts = hosts.len(), "Starting deployment");
        } else {
            tracing::info!(phase = phase, hosts = hosts.len(), "Starting phase");
        }
        drop(_guard);

        // Pre-create all step spans with phase as parent
        // These spans are NOT entered - they exist only to serve as parents for host spans
        // The explicit parent relationships ensure correct hierarchy without needing
        // to keep spans entered (which would require unsafe lifetime manipulation)
        let step_style = ProgressStyle::default_bar()
            .template("{span_child_prefix}{span_name}{span_fields}")
            .expect("Failed to create step span style");

        let build = info_span!(parent: &phase_span, "build");
        let copy = info_span!(parent: &phase_span, "copy");
        let activate = info_span!(parent: &phase_span, "activate");
        let reboot = info_span!(parent: &phase_span, "reboot");

        // Set style without spinner on each step span
        build.pb_set_style(&step_style);
        copy.pb_set_style(&step_style);
        activate.pb_set_style(&step_style);
        reboot.pb_set_style(&step_style);

        // Store all spans
        state.phase_span = Some(phase_span);
        state.build_span = Some(build);
        state.copy_span = Some(copy);
        state.activate_span = Some(activate);
        state.reboot_span = Some(reboot);

        Ok(())
    }

    fn on_phase_complete(&self, phase: &str, success: bool) -> Result<()> {
        // Log phase completion
        if phase == "default" || phase == "_all" {
            if success {
                tracing::info!("✓ Deployment completed successfully");
            } else {
                tracing::error!("✗ Deployment failed");
            }
        } else if success {
            tracing::info!(phase = phase, "✓ Phase completed successfully");
        } else {
            tracing::error!(phase = phase, "✗ Phase failed");
        }

        // Clean up all spans
        let mut state = self.state.lock();
        state.host_spans.clear();
        state.build_span = None;
        state.copy_span = None;
        state.activate_span = None;
        state.reboot_span = None;
        state.phase_span = None;
        Ok(())
    }

    // Build step
    fn on_build_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_build_start(&self, hostname: &str) -> Result<()> {
        let mut state = self.state.lock();

        // Create host span with build step as explicit parent
        let host_span = if let Some(ref build_span) = state.build_span {
            info_span!(parent: build_span, "host", host = %hostname)
        } else {
            info_span!("host", host = %hostname)
        };

        // Log within the host span
        let _guard = host_span.enter();
        tracing::info!("Building configuration");
        drop(_guard);

        // Store span (not entered) - we'll enter it temporarily in on_build_complete for logging
        state.host_spans.insert(hostname.to_string(), host_span);
        Ok(())
    }

    fn on_build_complete(&self, hostname: &str, success: bool) -> Result<()> {
        let mut state = self.state.lock();

        // Log completion within the host span (if it exists)
        if let Some(span) = state.host_spans.get(hostname) {
            let _guard = span.enter();
            if success {
                tracing::info!("✓ Build completed");
            } else {
                tracing::error!("✗ Build failed");
            }
        }

        state.host_spans.remove(hostname);
        Ok(())
    }

    fn on_build_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Copy step
    fn on_copy_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_copy_start(&self, hostname: &str) -> Result<()> {
        let mut state = self.state.lock();

        // Create host span with copy step as explicit parent
        let host_span = if let Some(ref copy_span) = state.copy_span {
            info_span!(parent: copy_span, "host", host = %hostname)
        } else {
            info_span!("host", host = %hostname)
        };

        // Log within the host span
        let _guard = host_span.enter();
        tracing::info!("Copying closure");
        drop(_guard);

        // Store span (not entered)
        state.host_spans.insert(hostname.to_string(), host_span);
        Ok(())
    }

    fn on_copy_complete(&self, hostname: &str, success: bool) -> Result<()> {
        let mut state = self.state.lock();

        // Log completion within the host span (if it exists)
        if let Some(span) = state.host_spans.get(hostname) {
            let _guard = span.enter();
            if success {
                tracing::info!("✓ Copy completed");
            } else {
                tracing::error!("✗ Copy failed");
            }
        }

        state.host_spans.remove(hostname);
        Ok(())
    }

    fn on_copy_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Deploy/Activate step
    fn on_deploy_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_deploy_start(&self, hostname: &str, _mode: ActivationMode) -> Result<()> {
        let mut state = self.state.lock();

        // Create host span with activate step as explicit parent
        let host_span = if let Some(ref activate_span) = state.activate_span {
            info_span!(parent: activate_span, "host", host = %hostname)
        } else {
            info_span!("host", host = %hostname)
        };

        // Log within the host span
        let _guard = host_span.enter();
        tracing::info!("Activating configuration");
        drop(_guard);

        // Store span (not entered)
        state.host_spans.insert(hostname.to_string(), host_span);
        Ok(())
    }

    fn on_deploy_complete(&self, hostname: &str, result: &DeployResult) -> Result<()> {
        let mut state = self.state.lock();

        // Log completion within the host span (if it exists)
        if let Some(span) = state.host_spans.get(hostname) {
            let _guard = span.enter();
            if result.success {
                tracing::info!("✓ Activation completed");
            } else {
                tracing::error!(message = %result.message, "✗ Activation failed");
            }
        }

        state.host_spans.remove(hostname);
        Ok(())
    }

    fn on_deploy_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Reboot step
    fn on_reboot_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_reboot_start(&self, hostname: &str) -> Result<()> {
        let mut state = self.state.lock();

        // Create host span with reboot step as explicit parent
        let host_span = if let Some(ref reboot_span) = state.reboot_span {
            info_span!(parent: reboot_span, "host", host = %hostname)
        } else {
            info_span!("host", host = %hostname)
        };

        // Log within the host span
        let _guard = host_span.enter();
        tracing::info!("Rebooting host");
        drop(_guard);

        // Store span (not entered)
        state.host_spans.insert(hostname.to_string(), host_span);
        Ok(())
    }

    fn on_reboot_complete(&self, hostname: &str, success: bool) -> Result<()> {
        let mut state = self.state.lock();

        // Log completion within the host span (if it exists)
        if let Some(span) = state.host_spans.get(hostname) {
            let _guard = span.enter();
            if success {
                tracing::info!("✓ Back online");
            } else {
                tracing::error!("✗ Reboot failed");
            }
        }

        state.host_spans.remove(hostname);
        Ok(())
    }

    fn on_reboot_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_observer_creation() {
        let _observer = CliObserver::new();
    }

    #[test]
    fn test_build_span_lifecycle() {
        let observer = CliObserver::new();

        // Initially no build span
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
        }

        // Start phase - creates step spans
        observer
            .on_phase_start("default", &["host1".to_string(), "host2".to_string()])
            .unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
        }

        // First host enters build
        observer.on_build_start("host1").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(state.host_spans.contains_key("host1"));
        }

        // Second host enters build
        observer.on_build_start("host2").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(state.host_spans.contains_key("host1"));
            assert!(state.host_spans.contains_key("host2"));
        }

        // First host leaves - span still there
        observer.on_build_complete("host1", true).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(!state.host_spans.contains_key("host1"));
            assert!(state.host_spans.contains_key("host2"));
        }

        // Last host leaves - span still there (until phase completes)
        observer.on_build_complete("host2", true).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some()); // Span persists until phase ends
            assert!(state.host_spans.is_empty());
        }

        // Phase complete - cleans up all spans
        observer.on_phase_complete("default", true).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
        }
    }

    #[test]
    fn test_multiple_step_spans() {
        let observer = CliObserver::new();

        // Start phase - creates all step spans
        observer
            .on_phase_start("default", &["host1".to_string()])
            .unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(state.copy_span.is_some());
            assert!(state.activate_span.is_some());
            assert!(state.reboot_span.is_some());
        }

        // Host enters build
        observer.on_build_start("host1").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(state.host_spans.contains_key("host1"));
        }

        // Same host finishes build, enters copy
        observer.on_build_complete("host1", true).unwrap();
        observer.on_copy_start("host1").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some()); // Build span still there
            assert!(state.copy_span.is_some()); // Copy span exists
            assert!(state.host_spans.contains_key("host1"));
        }

        // Cleanup
        observer.on_copy_complete("host1", true).unwrap();
        observer.on_phase_complete("default", true).unwrap();
    }

    #[test]
    fn test_error_handling_with_multiple_hosts() {
        let observer = CliObserver::new();

        // Start phase
        observer
            .on_phase_start("default", &["host1".to_string(), "host2".to_string()])
            .unwrap();

        observer.on_build_start("host1").unwrap();
        observer.on_build_start("host2").unwrap();

        // Host 1 fails
        observer.on_build_complete("host1", false).unwrap();
        {
            let state = observer.state.lock();
            assert!(!state.host_spans.contains_key("host1"));
            assert!(state.host_spans.contains_key("host2"));
            assert!(state.build_span.is_some()); // Still active for host2
        }

        // Host 2 completes
        observer.on_build_complete("host2", true).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.host_spans.is_empty());
            assert!(state.build_span.is_some()); // Persists until phase ends
        }

        // Phase complete
        observer.on_phase_complete("default", false).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
        }
    }

    #[test]
    fn test_phase_complete_cleanup() {
        let observer = CliObserver::new();

        // Start phase
        observer
            .on_phase_start("default", &["host1".to_string(), "host2".to_string()])
            .unwrap();

        // Create some host spans
        observer.on_build_start("host1").unwrap();
        observer.on_copy_start("host2").unwrap();

        // Phase complete should clean up everything
        observer.on_phase_complete("default", true).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
            assert!(state.copy_span.is_none());
            assert!(state.activate_span.is_none());
            assert!(state.reboot_span.is_none());
            assert!(state.host_spans.is_empty());
        }
    }
}
