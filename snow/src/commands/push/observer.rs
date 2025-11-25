//! CLI observer implementation for push deployment progress reporting
//!
//! Manages the complete span hierarchy for deployment progress tracking,
//! including phase, step, and host-level spans.

use parking_lot::Mutex;
use snowlib::{HookStage, PushObserver};
use std::collections::HashMap;
use tracing::info_span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

/// Lazily creates a step span if it doesn't exist, with the phase span as parent
macro_rules! get_or_create_step_span {
    ($state:expr, $span_field:ident, $span_name:expr) => {{
        if $state.$span_field.is_none() {
            let phase_span = $state
                .phase_span
                .as_ref()
                .expect("phase_span must exist - on_phase_start() should be called first");
            let span = info_span!(parent: phase_span, $span_name);

            // Set style without spinner
            use tracing_indicatif::style::ProgressStyle;
            let step_style = ProgressStyle::default_bar()
                .template("{span_child_prefix}{span_name}{span_fields}")
                .expect("Failed to create step span style");
            span.pb_set_style(&step_style);

            $state.$span_field = Some(span);
        }
        $state
            .$span_field
            .as_ref()
            .expect("span was just created")
    }};
}

/// CLI observer that manages the complete span hierarchy
///
/// Creates and manages phase, step, and host spans internally to provide
/// structured progress reporting. Phase spans are created in `on_phase_start()`,
/// step spans are created lazily when first needed, and host spans are created
/// per-host as they enter each step. All span creation happens inside the observer,
/// keeping the library code free of tracing span details.
///
/// Uses a mutex to protect shared state since observer methods are called from
/// Rayon worker threads in the parallel-per-host execution model.
pub struct CliObserver {
    state: Mutex<ObserverState>,
}

/// Key for tracking individual hook spans (hostname + stage + hook_name)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HookKey {
    hostname: String,
    stage: HookStage,
    hook_name: String,
}

struct ObserverState {
    // Phase span - created in on_phase_start()
    phase_span: Option<tracing::Span>,

    // Step spans - created lazily when first host enters each step
    // These are NOT entered - they exist only as parents for host spans
    build_span: Option<tracing::Span>,
    copy_span: Option<tracing::Span>,
    activate_span: Option<tracing::Span>,
    reboot_span: Option<tracing::Span>,

    // Hook stage spans - keyed by (hostname, stage)
    // Maps stage to the hooks step span for that stage
    hooks_stage_spans: HashMap<HookStage, tracing::Span>,

    // Host spans per step - created when host starts a step, entered temporarily for logging
    // Key is hostname, value is the span (not entered - we enter/exit as needed)
    build_host_spans: HashMap<String, tracing::Span>,
    copy_host_spans: HashMap<String, tracing::Span>,
    activate_host_spans: HashMap<String, tracing::Span>,
    reboot_host_spans: HashMap<String, tracing::Span>,

    // Hook host spans - keyed by (hostname, stage, hook_name)
    hook_host_spans: HashMap<HookKey, tracing::Span>,
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
                hooks_stage_spans: HashMap::new(),
                build_host_spans: HashMap::new(),
                copy_host_spans: HashMap::new(),
                activate_host_spans: HashMap::new(),
                reboot_host_spans: HashMap::new(),
                hook_host_spans: HashMap::new(),
            }),
        }
    }

    fn get_or_create_hooks_stage_span(
        state: &mut ObserverState,
        stage: HookStage,
    ) -> &tracing::Span {
        if !state.hooks_stage_spans.contains_key(&stage) {
            let phase_span = state
                .phase_span
                .as_ref()
                .expect("phase_span must exist - on_phase_start() should be called first");

            // Use stage name directly as span name for clean rendering
            let span = match stage {
                HookStage::PreActivate => info_span!(parent: phase_span, "hooks:PreActivate"),
                HookStage::PostActivate => info_span!(parent: phase_span, "hooks:PostActivate"),
                HookStage::PostReboot => info_span!(parent: phase_span, "hooks:PostReboot"),
                HookStage::OnFailure => info_span!(parent: phase_span, "hooks:OnFailure"),
            };

            use tracing_indicatif::style::ProgressStyle;
            let step_style = ProgressStyle::default_bar()
                .template("{span_child_prefix}{span_name}")
                .expect("Failed to create step span style");
            span.pb_set_style(&step_style);

            state.hooks_stage_spans.insert(stage, span);
        }

        state
            .hooks_stage_spans
            .get(&stage)
            .expect("hooks span was just created")
    }

    fn cleanup_hooks_stage_if_empty(state: &mut ObserverState, stage: HookStage) {
        let has_remaining_hooks = state.hook_host_spans.keys().any(|k| k.stage == stage);

        if !has_remaining_hooks {
            state.hooks_stage_spans.remove(&stage);
        }
    }
}

impl PushObserver for CliObserver {
    fn on_phase_start(&self, phase: &str, hosts: &[String]) {
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

        // Store phase span - step spans will be created lazily when first needed
        state.phase_span = Some(phase_span);
    }

    fn on_phase_complete(&self, phase: &str, success: bool) {
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
        state.build_host_spans.clear();
        state.copy_host_spans.clear();
        state.activate_host_spans.clear();
        state.reboot_host_spans.clear();
        state.hook_host_spans.clear();
        state.hooks_stage_spans.clear();
        state.build_span = None;
        state.copy_span = None;
        state.activate_span = None;
        state.reboot_span = None;
        state.phase_span = None;
    }

    // Build step
    fn on_build_step_start(&self, _phase: &str, _hosts: &[String]) {}

    fn on_build_start(&self, hostname: &str) {
        let mut state = self.state.lock();

        // Get or create the build step span, then create host span as child
        let build_span = get_or_create_step_span!(state, build_span, "build");
        let host_span = info_span!(parent: build_span, "host", host = %hostname);

        // Log within the host span
        let _guard = host_span.enter();
        tracing::info!("Building configuration");
        drop(_guard);

        // Store span (not entered) - we'll enter it temporarily in on_build_complete for logging
        state
            .build_host_spans
            .insert(hostname.to_string(), host_span);
    }

    fn on_build_complete(&self, hostname: &str, success: bool) {
        let mut state = self.state.lock();

        // Log completion within the host span
        let span = state
            .build_host_spans
            .get(hostname)
            .expect("host span must exist - on_build_start() should be called first");
        let _guard = span.enter();
        if success {
            tracing::info!("✓ Build completed");
        } else {
            tracing::error!("✗ Build failed");
        }
        drop(_guard);

        // Remove host span
        state.build_host_spans.remove(hostname);

        // If no hosts remain in this step, clean up the step span
        if state.build_host_spans.is_empty() {
            state.build_span = None;
        }
    }

    fn on_build_step_complete(&self, _phase: &str, _success: bool) {}

    // Copy step
    fn on_copy_step_start(&self, _phase: &str, _hosts: &[String]) {}

    fn on_copy_start(&self, hostname: &str) {
        let mut state = self.state.lock();

        // Get or create the copy step span, then create host span as child
        let copy_span = get_or_create_step_span!(state, copy_span, "copy");
        let host_span = info_span!(parent: copy_span, "host", host = %hostname);

        // Log within the host span
        let _guard = host_span.enter();
        tracing::info!("Copying closure");
        drop(_guard);

        // Store span (not entered)
        state
            .copy_host_spans
            .insert(hostname.to_string(), host_span);
    }

    fn on_copy_complete(&self, hostname: &str, success: bool) {
        let mut state = self.state.lock();

        // Log completion within the host span
        let span = state
            .copy_host_spans
            .get(hostname)
            .expect("host span must exist - on_copy_start() should be called first");
        let _guard = span.enter();
        if success {
            tracing::info!("✓ Copy completed");
        } else {
            tracing::error!("✗ Copy failed");
        }
        drop(_guard);

        // Remove host span
        state.copy_host_spans.remove(hostname);

        // If no hosts remain in this step, clean up the step span
        if state.copy_host_spans.is_empty() {
            state.copy_span = None;
        }
    }

    fn on_copy_step_complete(&self, _phase: &str, _success: bool) {}

    // Deploy/Activate step
    fn on_deploy_step_start(&self, _phase: &str, _hosts: &[String]) {}

    fn on_activation_start(&self, hostname: &str) {
        let mut state = self.state.lock();

        // Get or create the activate step span, then create host span as child
        let activate_span = get_or_create_step_span!(state, activate_span, "activate");
        let host_span = info_span!(parent: activate_span, "host", host = %hostname);

        // Log within the host span
        let _guard = host_span.enter();
        tracing::info!("Activating configuration");
        drop(_guard);

        // Store span (not entered)
        state
            .activate_host_spans
            .insert(hostname.to_string(), host_span);
    }

    fn on_activation_complete(&self, hostname: &str, success: bool) {
        let mut state = self.state.lock();

        // Log completion within the host span
        let span = state
            .activate_host_spans
            .get(hostname)
            .expect("host span must exist - on_activation_start() should be called first");
        let _guard = span.enter();
        if success {
            tracing::info!("✓ Activation completed");
        } else {
            tracing::error!("✗ Activation failed");
        }
        drop(_guard);

        // Remove host span
        state.activate_host_spans.remove(hostname);

        // If no hosts remain in this step, clean up the step span
        if state.activate_host_spans.is_empty() {
            state.activate_span = None;
        }
    }

    fn on_deploy_step_complete(&self, _phase: &str, _success: bool) {}

    // Reboot step
    fn on_reboot_step_start(&self, _phase: &str, _hosts: &[String]) {}

    fn on_reboot_start(&self, hostname: &str) {
        let mut state = self.state.lock();

        // Get or create the reboot step span, then create host span as child
        let reboot_span = get_or_create_step_span!(state, reboot_span, "reboot");
        let host_span = info_span!(parent: reboot_span, "host", host = %hostname);

        // Log within the host span
        let _guard = host_span.enter();
        tracing::info!("Rebooting host");
        drop(_guard);

        // Store span (not entered)
        state
            .reboot_host_spans
            .insert(hostname.to_string(), host_span);
    }

    fn on_reboot_complete(&self, hostname: &str, success: bool) {
        let mut state = self.state.lock();

        // Log completion within the host span
        let span = state
            .reboot_host_spans
            .get(hostname)
            .expect("host span must exist - on_reboot_start() should be called first");
        let _guard = span.enter();
        if success {
            tracing::info!("✓ Back online");
        } else {
            tracing::error!("✗ Reboot failed");
        }
        drop(_guard);

        // Remove host span
        state.reboot_host_spans.remove(hostname);

        // If no hosts remain in this step, clean up the step span
        if state.reboot_host_spans.is_empty() {
            state.reboot_span = None;
        }
    }

    fn on_reboot_step_complete(&self, _phase: &str, _success: bool) {}

    // Hook step
    fn on_hooks_start(&self, hostname: &str, stage: HookStage) {
        let mut state = self.state.lock();
        let hooks_span = Self::get_or_create_hooks_stage_span(&mut state, stage);
        let _guard = hooks_span.enter();
        tracing::info!(host = %hostname, "Running {} hooks", stage);
    }

    fn on_hooks_complete(&self, hostname: &str, stage: HookStage, success: bool) {
        let mut state = self.state.lock();

        if let Some(hooks_span) = state.hooks_stage_spans.get(&stage) {
            let _guard = hooks_span.enter();
            if success {
                tracing::info!(host = %hostname, "✓ {} hooks completed", stage);
            } else {
                tracing::error!(host = %hostname, "✗ {} hooks failed", stage);
            }
        }

        Self::cleanup_hooks_stage_if_empty(&mut state, stage);
    }

    fn on_hook_start(&self, hostname: &str, stage: HookStage, hook_name: &str) {
        let mut state = self.state.lock();
        let hooks_span = Self::get_or_create_hooks_stage_span(&mut state, stage);
        let host_span = info_span!(parent: hooks_span, "host", host = %hostname, hook = %hook_name);

        let _guard = host_span.enter();
        tracing::info!("Running hook '{}'", hook_name);
        drop(_guard);

        let key = HookKey {
            hostname: hostname.to_string(),
            stage,
            hook_name: hook_name.to_string(),
        };
        state.hook_host_spans.insert(key, host_span);
    }

    fn on_hook_complete(&self, hostname: &str, stage: HookStage, hook_name: &str, success: bool) {
        let mut state = self.state.lock();

        let key = HookKey {
            hostname: hostname.to_string(),
            stage,
            hook_name: hook_name.to_string(),
        };

        if let Some(span) = state.hook_host_spans.get(&key) {
            let _guard = span.enter();
            if success {
                tracing::info!("✓ Hook '{}' completed", hook_name);
            } else {
                tracing::error!("✗ Hook '{}' failed", hook_name);
            }
        }

        state.hook_host_spans.remove(&key);
        Self::cleanup_hooks_stage_if_empty(&mut state, stage);
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

        // Start phase - only creates phase span, step spans are lazy
        observer.on_phase_start("default", &["host1".to_string(), "host2".to_string()]);
        {
            let state = observer.state.lock();
            assert!(state.phase_span.is_some());
            assert!(state.build_span.is_none()); // Not created yet - lazy!
        }

        // First host enters build - NOW the build span is created
        observer.on_build_start("host1");
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(state.build_host_spans.contains_key("host1"));
        }

        // Second host enters build
        observer.on_build_start("host2");
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(state.build_host_spans.contains_key("host1"));
            assert!(state.build_host_spans.contains_key("host2"));
        }

        // First host leaves - span still there because host2 is still in build
        observer.on_build_complete("host1", true);
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(!state.build_host_spans.contains_key("host1"));
            assert!(state.build_host_spans.contains_key("host2"));
        }

        // Last host leaves - step span is cleaned up immediately!
        observer.on_build_complete("host2", true);
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none()); // Cleaned up when last host leaves
            assert!(state.build_host_spans.is_empty());
        }

        // Phase complete - cleans up all spans
        observer.on_phase_complete("default", true);
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
        }
    }

    #[test]
    fn test_multiple_step_spans() {
        let observer = CliObserver::new();

        // Start phase - only creates phase span, step spans are lazy
        observer.on_phase_start("default", &["host1".to_string()]);
        {
            let state = observer.state.lock();
            assert!(state.phase_span.is_some());
            assert!(state.build_span.is_none());
            assert!(state.copy_span.is_none());
            assert!(state.activate_span.is_none());
            assert!(state.reboot_span.is_none());
        }

        // Host enters build - creates build span lazily
        observer.on_build_start("host1");
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_some());
            assert!(state.copy_span.is_none()); // Copy span not created yet
            assert!(state.build_host_spans.contains_key("host1"));
        }

        // Same host finishes build, enters copy - build span cleaned up, copy span created
        observer.on_build_complete("host1", true);
        observer.on_copy_start("host1");
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none()); // Cleaned up when last host left
            assert!(state.copy_span.is_some()); // Copy span NOW created
            assert!(state.copy_host_spans.contains_key("host1"));
        }

        // Cleanup - copy span cleaned up when host leaves
        observer.on_copy_complete("host1", true);
        {
            let state = observer.state.lock();
            assert!(state.copy_span.is_none()); // Cleaned up immediately
        }
        observer.on_phase_complete("default", true);
    }

    #[test]
    fn test_error_handling_with_multiple_hosts() {
        let observer = CliObserver::new();

        // Start phase
        observer.on_phase_start("default", &["host1".to_string(), "host2".to_string()]);

        observer.on_build_start("host1");
        observer.on_build_start("host2");

        // Host 1 fails
        observer.on_build_complete("host1", false);
        {
            let state = observer.state.lock();
            assert!(!state.build_host_spans.contains_key("host1"));
            assert!(state.build_host_spans.contains_key("host2"));
            assert!(state.build_span.is_some()); // Still active for host2
        }

        // Host 2 completes - last host, so step span is cleaned up
        observer.on_build_complete("host2", true);
        {
            let state = observer.state.lock();
            assert!(state.build_host_spans.is_empty());
            assert!(state.build_span.is_none()); // Cleaned up when last host left
        }

        // Phase complete
        observer.on_phase_complete("default", false);
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
        }
    }

    #[test]
    fn test_phase_complete_cleanup() {
        let observer = CliObserver::new();

        // Start phase
        observer.on_phase_start("default", &["host1".to_string(), "host2".to_string()]);

        // Create some host spans
        observer.on_build_start("host1");
        observer.on_copy_start("host2");

        // Phase complete should clean up everything
        observer.on_phase_complete("default", true);
        {
            let state = observer.state.lock();
            assert!(state.build_span.is_none());
            assert!(state.copy_span.is_none());
            assert!(state.activate_span.is_none());
            assert!(state.reboot_span.is_none());
            assert!(state.build_host_spans.is_empty());
            assert!(state.copy_host_spans.is_empty());
            assert!(state.activate_host_spans.is_empty());
            assert!(state.reboot_host_spans.is_empty());
        }
    }
}
