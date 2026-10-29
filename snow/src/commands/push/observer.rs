//! CLI observer that bridges snowlib events to tracing-indicatif progress UI
//!
//! Manages span lifecycle (creation, activation, completion) while delegating
//! styling to styles.rs.

use super::styles::*;
use parking_lot::Mutex;
use snowlib::{HookStage, PushObserver};
use std::collections::{HashMap, HashSet};
use tracing::info_span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Macros for DRY step span creation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

macro_rules! activate_step_span {
    ($state:expr, $step_field:ident, $verb:expr, $active_style:expr) => {{
        let step = $state
            .$step_field
            .as_mut()
            .expect("step must be pre-created in on_phase_start");

        // Switch to active style on first host entry
        if !step.has_active_hosts {
            step.span.pb_set_style(&$active_style);
            step.span.pb_set_message($verb);
            step.has_active_hosts = true;
        }

        step.span.clone()
    }};
}

macro_rules! complete_host_in_step {
    ($state:expr, $step_field:ident, $host_spans:ident, $success:expr, $empty_style:expr) => {{
        let is_empty = $state.$host_spans.is_empty();
        if let Some(ref mut s) = $state.$step_field {
            if $success {
                s.completed_hosts += 1;
                s.span.pb_inc(1);
            }
            // Switch back to empty style when no more active hosts
            if is_empty && s.has_active_hosts {
                s.span.pb_set_style(&$empty_style);
                s.span.pb_set_message("");
                s.has_active_hosts = false;
            }
        }
    }};
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Observer State
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HookKey {
    hostname: String,
    stage: HookStage,
    hook_name: String,
}

struct StepState {
    span: tracing::Span,
    has_active_hosts: bool,
    completed_hosts: usize,
}

struct ObserverState {
    total_hosts: usize,
    failed_hosts: HashSet<String>,

    // Per-host completion tracking
    hosts_activated: HashSet<String>, // Hosts that completed activation successfully
    hosts_will_reboot: HashSet<String>, // Hosts that will reboot (from on_reboot_step_start)
    hosts_done: HashSet<String>,      // Hosts that are fully done

    phase_span: Option<tracing::Span>,

    build_step: Option<StepState>,
    copy_step: Option<StepState>,
    activate_step: Option<StepState>,
    reboot_step: Option<StepState>,
    failed_step: Option<tracing::Span>,

    build_host_spans: HashMap<String, tracing::Span>,
    copy_host_spans: HashMap<String, tracing::Span>,
    activate_host_spans: HashMap<String, tracing::Span>,
    reboot_host_spans: HashMap<String, tracing::Span>,
    hook_host_spans: HashMap<HookKey, tracing::Span>,
    failed_host_spans: HashMap<String, tracing::Span>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// CLI Observer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// CLI observer that provides gorgeous progress reporting with emojis,
/// spinners, progress bars, and hierarchical span rendering.
pub struct CliObserver {
    state: Mutex<ObserverState>,
}

impl CliObserver {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ObserverState {
                total_hosts: 0,
                failed_hosts: HashSet::new(),
                hosts_activated: HashSet::new(),
                hosts_will_reboot: HashSet::new(),
                hosts_done: HashSet::new(),
                phase_span: None,
                build_step: None,
                copy_step: None,
                activate_step: None,
                reboot_step: None,
                failed_step: None,
                build_host_spans: HashMap::new(),
                copy_host_spans: HashMap::new(),
                activate_host_spans: HashMap::new(),
                reboot_host_spans: HashMap::new(),
                hook_host_spans: HashMap::new(),
                failed_host_spans: HashMap::new(),
            }),
        }
    }

    fn mark_host_failed(state: &mut ObserverState, hostname: &str, step: &str) {
        if state.failed_hosts.contains(hostname) {
            return;
        }
        state.failed_hosts.insert(hostname.to_string());

        // Create the failed step span if it doesn't exist (first failure)
        if state.failed_step.is_none() {
            let phase_span = state.phase_span.as_ref().expect("phase_span must exist");

            // Switch phase to failure style (red spinner and progress bar)
            let show_progress_bar = state.total_hosts > 1;
            phase_span.pb_set_style(&phase_style(false, show_progress_bar));

            let span = info_span!(parent: phase_span, "✗ Failed");
            span.pb_set_style(&failed_step_style());
            span.pb_set_message(&format!("{} host(s)", state.failed_hosts.len()));
            span.pb_start();

            state.failed_step = Some(span);
        } else if let Some(ref span) = state.failed_step {
            span.pb_set_message(&format!("{} host(s)", state.failed_hosts.len()));
        }

        // Create a span for this failed host
        if let Some(ref failed_span) = state.failed_step {
            let host_span = info_span!(parent: failed_span, "failed_host", host = %hostname);
            host_span.pb_set_style(&failed_host_style());
            host_span.pb_set_message(&format!("{}: ✗ failed at {}", hostname, step));
            host_span.pb_start();
            state
                .failed_host_spans
                .insert(hostname.to_string(), host_span);
        }
    }

    fn create_host_span(step_span: &tracing::Span, hostname: &str, message: &str) -> tracing::Span {
        let span = info_span!(parent: step_span, "host", host = %hostname);
        span.pb_set_style(&host_style());
        span.pb_set_message(&format!("{}: {}", hostname, message));
        span.pb_start();
        span
    }

    fn complete_host_span(span: &tracing::Span, hostname: &str, success: bool) {
        let status = if success {
            format!("{}: ✓ done", hostname)
        } else {
            format!("{}: ✗ failed", hostname)
        };
        span.pb_set_style(&host_style());
        span.pb_set_message(&status);
    }

    fn activate_step_for_hook(
        state: &mut ObserverState,
        stage: HookStage,
    ) -> Option<tracing::Span> {
        match stage {
            HookStage::PreActivate | HookStage::PostActivate => {
                if let Some(ref mut step) = state.activate_step {
                    if !step.has_active_hosts {
                        step.span.pb_set_style(&step_style(StepColor::Cyan, true));
                        step.has_active_hosts = true;
                    }
                    Some(step.span.clone())
                } else {
                    None
                }
            }
            HookStage::PostReboot => {
                if let Some(ref mut step) = state.reboot_step {
                    if !step.has_active_hosts {
                        step.span
                            .pb_set_style(&step_style(StepColor::Magenta, true));
                        step.has_active_hosts = true;
                    }
                    Some(step.span.clone())
                } else {
                    None
                }
            }
            HookStage::OnFailure => {
                // OnFailure hooks show under the failed step
                state.failed_step.clone()
            }
        }
    }

    fn deactivate_step_for_hook_if_empty(state: &mut ObserverState, stage: HookStage) {
        let has_remaining_hooks = state.hook_host_spans.keys().any(|k| k.stage == stage);
        if has_remaining_hooks {
            return;
        }

        match stage {
            HookStage::PreActivate | HookStage::PostActivate => {
                // Only deactivate if no activate hosts are active either
                if state.activate_host_spans.is_empty() {
                    if let Some(ref mut step) = state.activate_step {
                        if step.has_active_hosts {
                            step.span.pb_set_style(&step_style(StepColor::Cyan, false));
                            step.span.pb_set_message("");
                            step.has_active_hosts = false;
                        }
                    }
                }
            }
            HookStage::PostReboot => {
                // Only deactivate if no reboot hosts are active either
                if state.reboot_host_spans.is_empty() {
                    if let Some(ref mut step) = state.reboot_step {
                        if step.has_active_hosts {
                            step.span
                                .pb_set_style(&step_style(StepColor::Magenta, false));
                            step.span.pb_set_message("");
                            step.has_active_hosts = false;
                        }
                    }
                }
            }
            HookStage::OnFailure => {
                // Failed step doesn't deactivate
            }
        }
    }

    fn mark_host_done(state: &mut ObserverState, hostname: &str) {
        if state.hosts_done.contains(hostname) {
            return;
        }
        state.hosts_done.insert(hostname.to_string());
        if let Some(ref phase_span) = state.phase_span {
            phase_span.pb_inc(1);
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PushObserver Implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

impl PushObserver for CliObserver {
    fn on_phase_start(&self, phase: &str, hosts: &[String]) {
        let mut state = self.state.lock();
        state.total_hosts = hosts.len();

        let phase_span = if phase == "default" || phase == "_all" {
            info_span!("▶ Deploying")
        } else {
            info_span!("▶ Phase")
        };

        let message = if phase == "default" || phase == "_all" {
            format!("to {} hosts", hosts.len())
        } else {
            format!("{} to {} hosts", phase, hosts.len())
        };

        // Progress bar is only useful when tracking multiple hosts
        let show_progress_bar = hosts.len() > 1;
        phase_span.pb_set_style(&phase_style(true, show_progress_bar));
        // Progress = 4 steps per host: build + copy + activate + completion
        // (completion = instant for non-rebooting hosts, reboot for rebooting hosts)
        phase_span.pb_set_length((hosts.len() * 4) as u64);
        phase_span.pb_set_message(&message);
        phase_span.pb_start();

        state.phase_span = Some(phase_span.clone());

        // Create and start step spans immediately with empty styles for stable ordering
        // indicatif shows most recently started at top, so start in reverse order
        // Display order: Build → Copy → Activate → Reboot (top to bottom)
        let reboot_span = info_span!(parent: &phase_span, "⟳ Reboot");
        reboot_span.pb_set_style(&step_style(StepColor::Magenta, false));
        reboot_span.pb_start();

        let activate_span = info_span!(parent: &phase_span, "★ Activate");
        activate_span.pb_set_style(&step_style(StepColor::Cyan, false));
        activate_span.pb_start();

        let copy_span = info_span!(parent: &phase_span, "⇢ Copy");
        copy_span.pb_set_style(&step_style(StepColor::Blue, false));
        copy_span.pb_start();

        let build_span = info_span!(parent: &phase_span, "⚙ Build");
        build_span.pb_set_style(&step_style(StepColor::Yellow, false));
        build_span.pb_start();

        state.build_step = Some(StepState {
            span: build_span,
            has_active_hosts: false,
            completed_hosts: 0,
        });
        state.copy_step = Some(StepState {
            span: copy_span,
            has_active_hosts: false,
            completed_hosts: 0,
        });
        state.activate_step = Some(StepState {
            span: activate_span,
            has_active_hosts: false,
            completed_hosts: 0,
        });
        state.reboot_step = Some(StepState {
            span: reboot_span,
            has_active_hosts: false,
            completed_hosts: 0,
        });
    }

    fn on_phase_complete(&self, phase: &str, success: bool) {
        let mut state = self.state.lock();

        // Mark any remaining activated hosts as done
        // (handles case where no reboot step happened)
        let remaining_hosts: Vec<String> = state
            .hosts_activated
            .iter()
            .filter(|h| !state.hosts_done.contains(*h) && !state.failed_hosts.contains(*h))
            .cloned()
            .collect();
        for hostname in remaining_hosts {
            Self::mark_host_done(&mut state, &hostname);
        }

        if let Some(ref span) = state.phase_span {
            let status = if success {
                if phase == "default" || phase == "_all" {
                    "✨ Deployment complete!"
                } else {
                    "✨ Phase complete!"
                }
            } else if phase == "default" || phase == "_all" {
                "💥 Deployment failed"
            } else {
                "💥 Phase failed"
            };
            span.pb_set_message(status);
        }

        state.build_host_spans.clear();
        state.copy_host_spans.clear();
        state.activate_host_spans.clear();
        state.reboot_host_spans.clear();
        state.hook_host_spans.clear();
        state.failed_host_spans.clear();
        state.failed_hosts.clear();
        state.hosts_activated.clear();
        state.hosts_will_reboot.clear();
        state.hosts_done.clear();
        state.build_step = None;
        state.copy_step = None;
        state.activate_step = None;
        state.reboot_step = None;
        state.failed_step = None;
        state.phase_span = None;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Build Step
    // ─────────────────────────────────────────────────────────────────────────

    fn on_build_step_start(&self, _phase: &str, _hosts: &[String]) {}

    fn on_build_start(&self, hostname: &str) {
        let mut state = self.state.lock();
        let step_span = activate_step_span!(
            state,
            build_step,
            "Building configurations",
            step_style(StepColor::Yellow, true)
        );
        let host_span = Self::create_host_span(&step_span, hostname, "building configuration...");
        state
            .build_host_spans
            .insert(hostname.to_string(), host_span);
    }

    fn on_build_complete(&self, hostname: &str, success: bool) {
        let mut state = self.state.lock();
        if let Some(span) = state.build_host_spans.get(hostname) {
            Self::complete_host_span(span, hostname, success);
        }
        state.build_host_spans.remove(hostname);
        if !success {
            Self::mark_host_failed(&mut state, hostname, "build");
        } else if let Some(ref phase_span) = state.phase_span {
            phase_span.pb_inc(1);
        }
        complete_host_in_step!(
            state,
            build_step,
            build_host_spans,
            success,
            step_style(StepColor::Yellow, false)
        );
    }

    fn on_host_unchanged(&self, hostname: &str) {
        let mut state = self.state.lock();
        if state.hosts_done.contains(hostname) {
            return;
        }
        if let Some(ref phase_span) = state.phase_span {
            // Covers the copy and activate steps this host skips; mark_host_done adds the last
            phase_span.pb_inc(2);
        }
        Self::mark_host_done(&mut state, hostname);
    }

    fn on_build_step_complete(&self, _phase: &str, _success: bool) {}

    // ─────────────────────────────────────────────────────────────────────────
    // Copy Step
    // ─────────────────────────────────────────────────────────────────────────

    fn on_copy_step_start(&self, _phase: &str, _hosts: &[String]) {}

    fn on_copy_start(&self, hostname: &str) {
        let mut state = self.state.lock();
        let step_span = activate_step_span!(
            state,
            copy_step,
            "Copying closures",
            step_style(StepColor::Blue, true)
        );
        let host_span = Self::create_host_span(&step_span, hostname, "copying closure...");
        state
            .copy_host_spans
            .insert(hostname.to_string(), host_span);
    }

    fn on_copy_complete(&self, hostname: &str, success: bool) {
        let mut state = self.state.lock();
        if let Some(span) = state.copy_host_spans.get(hostname) {
            Self::complete_host_span(span, hostname, success);
        }
        state.copy_host_spans.remove(hostname);
        if !success {
            Self::mark_host_failed(&mut state, hostname, "copy");
        } else if let Some(ref phase_span) = state.phase_span {
            phase_span.pb_inc(1);
        }
        complete_host_in_step!(
            state,
            copy_step,
            copy_host_spans,
            success,
            step_style(StepColor::Blue, false)
        );
    }

    fn on_copy_step_complete(&self, _phase: &str, _success: bool) {}

    // ─────────────────────────────────────────────────────────────────────────
    // Activate Step
    // ─────────────────────────────────────────────────────────────────────────

    fn on_deploy_step_start(&self, _phase: &str, _hosts: &[String]) {}

    fn on_activation_start(&self, hostname: &str) {
        let mut state = self.state.lock();
        let step_span = activate_step_span!(
            state,
            activate_step,
            "Activating systems",
            step_style(StepColor::Cyan, true)
        );
        let host_span = Self::create_host_span(&step_span, hostname, "activating configuration...");
        state
            .activate_host_spans
            .insert(hostname.to_string(), host_span);
    }

    fn on_activation_complete(&self, hostname: &str, success: bool) {
        let mut state = self.state.lock();
        if let Some(span) = state.activate_host_spans.get(hostname) {
            Self::complete_host_span(span, hostname, success);
        }
        state.activate_host_spans.remove(hostname);
        if !success {
            Self::mark_host_failed(&mut state, hostname, "activate");
        } else {
            // Track that this host activated successfully
            state.hosts_activated.insert(hostname.to_string());
            // Increment progress for activation step
            if let Some(ref phase_span) = state.phase_span {
                phase_span.pb_inc(1);
            }
        }
        complete_host_in_step!(
            state,
            activate_step,
            activate_host_spans,
            success,
            step_style(StepColor::Cyan, false)
        );
    }

    fn on_deploy_step_complete(&self, _phase: &str, _success: bool) {}

    // ─────────────────────────────────────────────────────────────────────────
    // Reboot Step
    // ─────────────────────────────────────────────────────────────────────────

    fn on_reboot_step_start(&self, _phase: &str, hosts: &[String]) {
        let mut state = self.state.lock();

        // Store which hosts will reboot
        state.hosts_will_reboot = hosts.iter().cloned().collect();

        // Mark activated hosts that won't reboot as done
        let non_rebooting_hosts: Vec<String> = state
            .hosts_activated
            .iter()
            .filter(|h| !state.hosts_will_reboot.contains(*h))
            .cloned()
            .collect();

        for hostname in non_rebooting_hosts {
            Self::mark_host_done(&mut state, &hostname);
        }
    }

    fn on_reboot_start(&self, hostname: &str) {
        let mut state = self.state.lock();
        let step_span = activate_step_span!(
            state,
            reboot_step,
            "Rebooting hosts",
            step_style(StepColor::Magenta, true)
        );
        let host_span = Self::create_host_span(&step_span, hostname, "rebooting...");
        state
            .reboot_host_spans
            .insert(hostname.to_string(), host_span);
    }

    fn on_reboot_complete(&self, hostname: &str, success: bool) {
        let mut state = self.state.lock();
        if let Some(span) = state.reboot_host_spans.get(hostname) {
            let status = if success {
                format!("{}: ✓ back online", hostname)
            } else {
                format!("{}: ✗ failed", hostname)
            };
            span.pb_set_style(&host_style());
            span.pb_set_message(&status);
        }
        state.reboot_host_spans.remove(hostname);
        if !success {
            Self::mark_host_failed(&mut state, hostname, "reboot");
        } else {
            // Reboot completed successfully - host is now done
            Self::mark_host_done(&mut state, hostname);
        }
        complete_host_in_step!(
            state,
            reboot_step,
            reboot_host_spans,
            success,
            step_style(StepColor::Magenta, false)
        );
    }

    fn on_reboot_step_complete(&self, _phase: &str, _success: bool) {}

    // ─────────────────────────────────────────────────────────────────────────
    // Hooks
    // ─────────────────────────────────────────────────────────────────────────

    fn on_hooks_start(&self, hostname: &str, stage: HookStage) {
        let mut state = self.state.lock();
        // Activate the appropriate step for this hook stage
        Self::activate_step_for_hook(&mut state, stage);
        tracing::debug!(host = %hostname, "Starting {} hooks", stage);
    }

    fn on_hooks_complete(&self, hostname: &str, stage: HookStage, success: bool) {
        let state = self.state.lock();
        if success {
            tracing::debug!(host = %hostname, "✓ {} hooks completed", stage);
        } else {
            tracing::warn!(host = %hostname, "✗ {} hooks failed", stage);
        }
        drop(state);
        // Deactivation happens in on_hook_complete
    }

    fn on_hook_start(&self, hostname: &str, stage: HookStage, hook_name: &str) {
        let mut state = self.state.lock();

        let step_span = Self::activate_step_for_hook(&mut state, stage);

        let stage_prefix = match stage {
            HookStage::PreActivate => "pre-activate",
            HookStage::PostActivate => "post-activate",
            HookStage::PostReboot => "post-reboot",
            HookStage::OnFailure => "on-failure",
        };

        if let Some(parent_span) = step_span {
            // Update the step message to show hook activity
            match stage {
                HookStage::PreActivate | HookStage::PostActivate => {
                    if let Some(ref step) = state.activate_step {
                        step.span
                            .pb_set_message(&format!("Running {} hooks", stage_prefix));
                    }
                }
                HookStage::PostReboot => {
                    if let Some(ref step) = state.reboot_step {
                        step.span
                            .pb_set_message(&format!("Running {} hooks", stage_prefix));
                    }
                }
                HookStage::OnFailure => {}
            }

            let host_span =
                info_span!(parent: &parent_span, "hook", host = %hostname, hook = %hook_name);
            host_span.pb_set_style(&host_style());
            host_span.pb_set_message(&format!(
                "{}: {} hook '{}'...",
                hostname, stage_prefix, hook_name
            ));
            host_span.pb_start();

            let key = HookKey {
                hostname: hostname.to_string(),
                stage,
                hook_name: hook_name.to_string(),
            };
            state.hook_host_spans.insert(key, host_span);
        }
    }

    fn on_hook_complete(&self, hostname: &str, stage: HookStage, hook_name: &str, success: bool) {
        let mut state = self.state.lock();

        let key = HookKey {
            hostname: hostname.to_string(),
            stage,
            hook_name: hook_name.to_string(),
        };

        if let Some(span) = state.hook_host_spans.get(&key) {
            let stage_prefix = match stage {
                HookStage::PreActivate => "pre-activate",
                HookStage::PostActivate => "post-activate",
                HookStage::PostReboot => "post-reboot",
                HookStage::OnFailure => "on-failure",
            };
            let status = if success {
                format!("{}: ✓ {} hook '{}' done", hostname, stage_prefix, hook_name)
            } else {
                format!(
                    "{}: ✗ {} hook '{}' failed",
                    hostname, stage_prefix, hook_name
                )
            };
            span.pb_set_style(&host_style());
            span.pb_set_message(&status);
        }

        state.hook_host_spans.remove(&key);

        if !success {
            let step_name = format!("hook:{}", hook_name);
            Self::mark_host_failed(&mut state, hostname, &step_name);
        }

        Self::deactivate_step_for_hook_if_empty(&mut state, stage);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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

        {
            let state = observer.state.lock();
            assert!(state.build_step.is_none());
        }

        observer.on_phase_start("default", &["host1".to_string(), "host2".to_string()]);
        {
            let state = observer.state.lock();
            assert!(state.phase_span.is_some());
            // Steps are pre-created at phase start for stable ordering
            assert!(state.build_step.is_some());
            assert!(!state.build_step.as_ref().unwrap().has_active_hosts);
        }

        observer.on_build_start("host1");
        {
            let state = observer.state.lock();
            assert!(state.build_step.is_some());
            assert!(state.build_step.as_ref().unwrap().has_active_hosts);
            assert!(state.build_host_spans.contains_key("host1"));
        }

        observer.on_build_start("host2");
        {
            let state = observer.state.lock();
            assert!(state.build_step.is_some());
            assert!(state.build_host_spans.contains_key("host1"));
            assert!(state.build_host_spans.contains_key("host2"));
        }

        observer.on_build_complete("host1", true);
        {
            let state = observer.state.lock();
            assert!(state.build_step.is_some());
            assert!(!state.build_host_spans.contains_key("host1"));
            assert!(state.build_host_spans.contains_key("host2"));
        }

        observer.on_build_complete("host2", true);
        {
            let state = observer.state.lock();
            // Step stays but has_active_hosts becomes false when empty
            assert!(state.build_step.is_some());
            assert!(!state.build_step.as_ref().unwrap().has_active_hosts);
            assert!(state.build_host_spans.is_empty());
        }

        observer.on_phase_complete("default", true);
        {
            let state = observer.state.lock();
            assert!(state.build_step.is_none());
        }
    }

    #[test]
    fn test_multiple_step_spans() {
        let observer = CliObserver::new();

        observer.on_phase_start("default", &["host1".to_string()]);
        {
            let state = observer.state.lock();
            assert!(state.phase_span.is_some());
            // Steps are pre-created at phase start for stable ordering
            assert!(state.build_step.is_some());
            assert!(!state.build_step.as_ref().unwrap().has_active_hosts);
        }

        observer.on_build_start("host1");
        {
            let state = observer.state.lock();
            assert!(state.build_step.as_ref().unwrap().has_active_hosts);
            assert!(state.copy_step.is_some()); // pre-created
            assert!(state.build_host_spans.contains_key("host1"));
        }

        observer.on_build_complete("host1", true);
        observer.on_copy_start("host1");
        {
            let state = observer.state.lock();
            // Build step stays but becomes inactive when empty
            assert!(state.build_step.is_some());
            assert!(!state.build_step.as_ref().unwrap().has_active_hosts);
            assert!(state.copy_step.as_ref().unwrap().has_active_hosts);
            assert!(state.copy_host_spans.contains_key("host1"));
        }

        observer.on_copy_complete("host1", true);
        {
            let state = observer.state.lock();
            // Step stays but becomes inactive when empty
            assert!(state.copy_step.is_some());
            assert!(!state.copy_step.as_ref().unwrap().has_active_hosts);
        }
        observer.on_phase_complete("default", true);
    }

    #[test]
    fn test_error_handling_with_multiple_hosts() {
        let observer = CliObserver::new();

        observer.on_phase_start("default", &["host1".to_string(), "host2".to_string()]);

        observer.on_build_start("host1");
        observer.on_build_start("host2");

        observer.on_build_complete("host1", false);
        {
            let state = observer.state.lock();
            assert!(!state.build_host_spans.contains_key("host1"));
            assert!(state.build_host_spans.contains_key("host2"));
            assert!(state.build_step.is_some());
        }

        observer.on_build_complete("host2", true);
        {
            let state = observer.state.lock();
            assert!(state.build_host_spans.is_empty());
            // Step stays but becomes inactive when empty
            assert!(state.build_step.is_some());
            assert!(!state.build_step.as_ref().unwrap().has_active_hosts);
        }

        observer.on_phase_complete("default", false);
        {
            let state = observer.state.lock();
            assert!(state.build_step.is_none());
        }
    }

    #[test]
    fn test_phase_complete_cleanup() {
        let observer = CliObserver::new();

        observer.on_phase_start("default", &["host1".to_string(), "host2".to_string()]);

        observer.on_build_start("host1");
        observer.on_copy_start("host2");

        observer.on_phase_complete("default", true);
        {
            let state = observer.state.lock();
            assert!(state.build_step.is_none());
            assert!(state.copy_step.is_none());
            assert!(state.activate_step.is_none());
            assert!(state.reboot_step.is_none());
            assert!(state.build_host_spans.is_empty());
            assert!(state.copy_host_spans.is_empty());
            assert!(state.activate_host_spans.is_empty());
            assert!(state.reboot_host_spans.is_empty());
        }
    }
}
