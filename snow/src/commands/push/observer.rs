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
    _phase_entered: Option<tracing::span::Entered<'static>>,

    // Step spans - pre-created in on_phase_start() for consistent ordering
    build_span: Option<tracing::Span>,
    copy_span: Option<tracing::Span>,
    activate_span: Option<tracing::Span>,
    reboot_span: Option<tracing::Span>,

    // Step span entered guards - stored in reverse order for correct display
    _step_guards: Vec<tracing::span::Entered<'static>>,

    // Host spans - created when host starts a step, entered temporarily for logging
    // Key is hostname, value is the span (not entered - we enter/exit as needed)
    host_spans: HashMap<String, tracing::Span>,

    // Counters - track how many hosts are in each step
    build_count: usize,
    copy_count: usize,
    activate_count: usize,
    reboot_count: usize,
}

impl CliObserver {
    pub fn new(_quiet: bool) -> Self {
        Self {
            state: Mutex::new(ObserverState {
                phase_span: None,
                _phase_entered: None,
                build_span: None,
                copy_span: None,
                activate_span: None,
                reboot_span: None,
                _step_guards: Vec::new(),
                host_spans: HashMap::new(),
                build_count: 0,
                copy_count: 0,
                activate_count: 0,
                reboot_count: 0,
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

        // Pre-create all step spans for consistent ordering
        // These will be entered in reverse order to display correctly
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

        // Store spans FIRST (before creating entered guards)
        state.build_span = Some(build);
        state.copy_span = Some(copy);
        state.activate_span = Some(activate);
        state.reboot_span = Some(reboot);
        state.phase_span = Some(phase_span);

        // Now enter spans in reverse order for correct display
        // (tracing-indicatif's insert_after causes reverse ordering)
        // SAFETY: We must enter spans AFTER storing them, because Entered<'_>
        // contains a reference to the Span. We transmute to 'static to store
        // in the state, which is safe because:
        // 1. The Spans are stored in the same struct
        // 2. Entered guards are dropped before Spans in on_phase_complete
        // 3. All guards are dropped before the spans they reference
        let mut guards = Vec::new();
        guards.push(unsafe {
            std::mem::transmute::<tracing::span::Entered<'_>, tracing::span::Entered<'static>>(
                state.reboot_span.as_ref().unwrap().enter(),
            )
        });
        guards.push(unsafe {
            std::mem::transmute::<tracing::span::Entered<'_>, tracing::span::Entered<'static>>(
                state.activate_span.as_ref().unwrap().enter(),
            )
        });
        guards.push(unsafe {
            std::mem::transmute::<tracing::span::Entered<'_>, tracing::span::Entered<'static>>(
                state.copy_span.as_ref().unwrap().enter(),
            )
        });
        guards.push(unsafe {
            std::mem::transmute::<tracing::span::Entered<'_>, tracing::span::Entered<'static>>(
                state.build_span.as_ref().unwrap().enter(),
            )
        });
        state._step_guards = guards;

        // Enter phase span last
        let phase_entered = unsafe {
            std::mem::transmute::<tracing::span::Entered<'_>, tracing::span::Entered<'static>>(
                state.phase_span.as_ref().unwrap().enter(),
            )
        };
        state._phase_entered = Some(phase_entered);

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

        // Clean up all spans and guards
        let mut state = self.state.lock();
        state.host_spans.clear();
        state._step_guards.clear();
        state.build_span = None;
        state.copy_span = None;
        state.activate_span = None;
        state.reboot_span = None;
        state._phase_entered = None;
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
        state.build_count += 1;
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
        state.build_count = state.build_count.saturating_sub(1);
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
        state.copy_count += 1;
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
        state.copy_count = state.copy_count.saturating_sub(1);
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
        state.activate_count += 1;
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
        state.activate_count = state.activate_count.saturating_sub(1);
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
        state.reboot_count += 1;
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
        state.reboot_count = state.reboot_count.saturating_sub(1);
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
        let _observer = CliObserver::new(false);
        let _quiet_observer = CliObserver::new(true);
    }

    #[test]
    fn test_build_span_lifecycle() {
        let observer = CliObserver::new(false);

        // Initially no build span
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
            assert_eq!(state.build_count, 0);
        }

        // Start phase - creates step spans
        observer
            .on_phase_start("default", &["host1".to_string(), "host2".to_string()])
            .unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert_eq!(state.build_count, 0);
        }

        // First host enters build
        observer.on_build_start("host1").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert_eq!(state.build_count, 1);
        }

        // Second host enters build - span still there, count increases
        observer.on_build_start("host2").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert_eq!(state.build_count, 2);
        }

        // First host leaves - span still there
        observer.on_build_complete("host1", true).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert_eq!(state.build_count, 1);
        }

        // Last host leaves - span still there (until phase completes)
        observer.on_build_complete("host2", true).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some()); // Span persists until phase ends
            assert_eq!(state.build_count, 0);
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
        let observer = CliObserver::new(false);

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
            assert_eq!(state.build_count, 1);
        }

        // Same host finishes build, enters copy
        observer.on_build_complete("host1", true).unwrap();
        observer.on_copy_start("host1").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some()); // Build span still there
            assert!(state.copy_span.is_some()); // Copy span exists
            assert_eq!(state.build_count, 0);
            assert_eq!(state.copy_count, 1);
        }

        // Cleanup
        observer.on_copy_complete("host1", true).unwrap();
        observer.on_phase_complete("default", true).unwrap();
    }

    #[test]
    fn test_error_handling_maintains_counter() {
        let observer = CliObserver::new(false);

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
            assert_eq!(state.build_count, 1);
            assert!(state.build_span.is_some()); // Still active for host2
        }

        // Host 2 completes
        observer.on_build_complete("host2", true).unwrap();
        {
            let state = observer.state.lock();
            assert_eq!(state.build_count, 0);
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
        let observer = CliObserver::new(false);

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
        }
    }
}
