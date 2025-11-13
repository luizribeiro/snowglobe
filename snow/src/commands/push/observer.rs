//! CLI observer implementation for push deployment progress reporting
//!
//! Manages dynamic step span lifecycle - step spans appear when the first host
//! enters that step and disappear when the last host leaves.

use parking_lot::Mutex;
use snowlib::{ActivationMode, DeployResult, PushObserver, Result};
use tracing::info_span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

/// CLI observer that manages step span lifecycle
///
/// Tracks how many hosts are in each step and shows/hides step spans accordingly.
/// Uses a mutex to protect shared state since observer methods are called from
/// Rayon worker threads (inside execute_host_pipeline) in the parallel execution model.
///
/// NOTE: EnteredSpan is NOT Send, but this is safe because:
/// - The spans are protected inside the mutex
/// - Only one thread can access them at a time (when mutex is locked)
/// - Spans are created and dropped within the same mutex guard
pub struct CliObserver {
    state: Mutex<ObserverState>,
}

struct ObserverState {
    // Phase span - used as parent for step spans
    phase_span: Option<tracing::Span>,

    // Step spans - stored but not entered (Span is Clone and can be shared)
    // All step spans are entered in execute_phase to keep them always visible
    // Entered in reverse order (reboot, activate, copy, build) to display correctly
    build_span: Option<tracing::Span>,
    copy_span: Option<tracing::Span>,
    activate_span: Option<tracing::Span>,
    reboot_span: Option<tracing::Span>,

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
                build_span: None,
                copy_span: None,
                activate_span: None,
                reboot_span: None,
                build_count: 0,
                copy_count: 0,
                activate_count: 0,
                reboot_count: 0,
            }),
        }
    }
}

impl PushObserver for CliObserver {
    fn on_phase_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        // Phase span is created in execute_phase
        Ok(())
    }

    fn on_phase_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        // Clean up any remaining spans
        let mut state = self.state.lock();
        state.build_span = None;
        state.copy_span = None;
        state.activate_span = None;
        state.reboot_span = None;
        Ok(())
    }

    // Build step
    fn on_build_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_build_start(&self, _hostname: &str) -> Result<()> {
        let mut state = self.state.lock();
        if state.build_count == 0 && state.build_span.is_none() {
            // First host entering build - create span if not already pre-created
            state.build_span = if let Some(ref parent) = state.phase_span {
                Some(info_span!(parent: parent, "build"))
            } else {
                Some(info_span!("build"))
            };
        }
        state.build_count += 1;
        Ok(())
    }

    fn on_build_complete(&self, _hostname: &str, _success: bool) -> Result<()> {
        let mut state = self.state.lock();
        state.build_count = state.build_count.saturating_sub(1);
        if state.build_count == 0 {
            // Last host left build - drop span
            state.build_span = None;
        }
        Ok(())
    }

    /// Get the current build span for use as an explicit parent
    fn build_span(&self) -> Option<tracing::Span> {
        self.state.lock().build_span.clone()
    }

    fn on_build_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Copy step
    fn on_copy_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_copy_start(&self, _hostname: &str) -> Result<()> {
        let mut state = self.state.lock();
        if state.copy_count == 0 && state.copy_span.is_none() {
            // First host entering copy - create span if not already pre-created
            state.copy_span = if let Some(ref parent) = state.phase_span {
                Some(info_span!(parent: parent, "copy"))
            } else {
                Some(info_span!("copy"))
            };
        }
        state.copy_count += 1;
        Ok(())
    }

    fn on_copy_complete(&self, _hostname: &str, _success: bool) -> Result<()> {
        let mut state = self.state.lock();
        state.copy_count = state.copy_count.saturating_sub(1);
        if state.copy_count == 0 {
            state.copy_span = None;
        }
        Ok(())
    }

    /// Get the current copy span for use as an explicit parent
    fn copy_span(&self) -> Option<tracing::Span> {
        self.state.lock().copy_span.clone()
    }

    fn on_copy_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Deploy/Activate step
    fn on_deploy_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_deploy_start(&self, _hostname: &str, _mode: ActivationMode) -> Result<()> {
        let mut state = self.state.lock();
        if state.activate_count == 0 && state.activate_span.is_none() {
            // First host entering activate - create span if not already pre-created
            state.activate_span = if let Some(ref parent) = state.phase_span {
                Some(info_span!(parent: parent, "activate"))
            } else {
                Some(info_span!("activate"))
            };
        }
        state.activate_count += 1;
        Ok(())
    }

    fn on_deploy_complete(&self, _hostname: &str, _result: &DeployResult) -> Result<()> {
        let mut state = self.state.lock();
        state.activate_count = state.activate_count.saturating_sub(1);
        if state.activate_count == 0 {
            state.activate_span = None;
        }
        Ok(())
    }

    /// Get the current activate span for use as an explicit parent
    fn activate_span(&self) -> Option<tracing::Span> {
        self.state.lock().activate_span.clone()
    }

    fn on_deploy_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Reboot step
    fn on_reboot_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_reboot_start(&self, _hostname: &str) -> Result<()> {
        let mut state = self.state.lock();
        if state.reboot_count == 0 && state.reboot_span.is_none() {
            // First host entering reboot - create span if not already pre-created
            state.reboot_span = if let Some(ref parent) = state.phase_span {
                Some(info_span!(parent: parent, "reboot"))
            } else {
                Some(info_span!("reboot"))
            };
        }
        state.reboot_count += 1;
        Ok(())
    }

    fn on_reboot_complete(&self, _hostname: &str, _success: bool) -> Result<()> {
        let mut state = self.state.lock();
        state.reboot_count = state.reboot_count.saturating_sub(1);
        if state.reboot_count == 0 {
            state.reboot_span = None;
        }
        Ok(())
    }

    /// Get the current reboot span for use as an explicit parent
    fn reboot_span(&self) -> Option<tracing::Span> {
        self.state.lock().reboot_span.clone()
    }

    fn on_reboot_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // --- Span management ---

    fn set_phase_span(&self, span: tracing::Span) {
        self.state.lock().phase_span = Some(span);
    }

    fn prepare_step_spans(&self) {
        use tracing_indicatif::style::ProgressStyle;

        let mut state = self.state.lock();

        // Pre-create all step spans (build, copy, activate, reboot)
        // These will be entered in REVERSE order in execute_phase to display correctly
        // (tracing-indicatif's insert_after causes reverse ordering)
        let parent = state.phase_span.clone();

        // Create a simple style without spinner for step spans
        let step_style = ProgressStyle::default_bar()
            .template("{span_child_prefix}{span_name}{span_fields}")
            .expect("Failed to create step span style");

        if let Some(parent) = parent {
            let build = info_span!(parent: &parent, "build");
            let copy = info_span!(parent: &parent, "copy");
            let activate = info_span!(parent: &parent, "activate");
            let reboot = info_span!(parent: &parent, "reboot");

            // Set style without spinner on each step span
            build.pb_set_style(&step_style);
            copy.pb_set_style(&step_style);
            activate.pb_set_style(&step_style);
            reboot.pb_set_style(&step_style);

            state.build_span = Some(build);
            state.copy_span = Some(copy);
            state.activate_span = Some(activate);
            state.reboot_span = Some(reboot);
        } else {
            let build = info_span!("build");
            let copy = info_span!("copy");
            let activate = info_span!("activate");
            let reboot = info_span!("reboot");

            build.pb_set_style(&step_style);
            copy.pb_set_style(&step_style);
            activate.pb_set_style(&step_style);
            reboot.pb_set_style(&step_style);

            state.build_span = Some(build);
            state.copy_span = Some(copy);
            state.activate_span = Some(activate);
            state.reboot_span = Some(reboot);
        }
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

        // First host enters build - span appears
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

        // Last host leaves - span disappears
        observer.on_build_complete("host2", true).unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
            assert_eq!(state.build_count, 0);
        }
    }

    #[test]
    fn test_multiple_step_spans() {
        let observer = CliObserver::new(false);

        // Host enters build
        observer.on_build_start("host1").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(state.copy_span.is_none());
        }

        // Same host finishes build, enters copy
        observer.on_build_complete("host1", true).unwrap();
        observer.on_copy_start("host1").unwrap();
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none()); // Build span gone
            assert!(state.copy_span.is_some()); // Copy span appeared
        }

        // Cleanup
        observer.on_copy_complete("host1", true).unwrap();
    }

    #[test]
    fn test_error_handling_maintains_counter() {
        let observer = CliObserver::new(false);

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
            assert!(state.build_span.is_none());
        }
    }

    #[test]
    fn test_phase_complete_cleanup() {
        let observer = CliObserver::new(false);

        // Create some spans
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
