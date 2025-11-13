# Refactor Plan: Dynamic Step Spans with Observer Pattern and Parallel Host Execution

## Goal
Transform the push deployment to use parallel-per-host execution with dynamic step span visibility, where step spans (build, copy, activate, reboot) only appear when hosts are actively in that step.

## Desired Progress Indicator UI

### Visual Layout

The progress indicator should show a hierarchical tree structure using `tracing-indicatif`:

```
⠦ phase{phase="default" hosts=3}
  ↳ ⠤ build{}
    ↳ ⠂ host{host=hydrogen}
    ↳ ⠴ host{host=oxygen}
  ↳ ⠄ copy{}
    ↳ ⠦ host{host=carbon}
```

**Key behaviors:**

1. **Phase span** - Always visible for the duration of the phase
   - Shows phase name (or "default" for single phase)
   - Shows total host count

2. **Step spans** - **Dynamically appear/disappear**
   - Only visible when at least one host is in that step
   - As soon as the first host enters a step (e.g., build), the step span appears
   - As soon as the last host leaves a step, the step span disappears
   - Steps can be visible simultaneously (e.g., some hosts building while others are copying)

3. **Host spans** - Nested under the current step
   - Shows which host is being processed
   - Moves from step to step as the host progresses
   - Child of the step span (not the phase span)

### Example Execution Flow

**Time T0 - All hosts start building:**
```
⠦ phase{phase="default" hosts=3}
  ↳ ⠤ build{}
    ↳ ⠂ host{host=hydrogen}
    ↳ ⠴ host{host=oxygen}
    ↳ ⠦ host{host=carbon}
```

**Time T1 - Hydrogen finishes build, starts copy; others still building:**
```
⠦ phase{phase="default" hosts=3}
  ↳ ⠤ build{}
    ↳ ⠴ host{host=oxygen}
    ↳ ⠦ host{host=carbon}
  ↳ ⠄ copy{}
    ↳ ⠂ host{host=hydrogen}
```

**Time T2 - Hydrogen activating, oxygen copying, carbon still building:**
```
⠦ phase{phase="default" hosts=3}
  ↳ ⠤ build{}
    ↳ ⠦ host{host=carbon}
  ↳ ⠄ copy{}
    ↳ ⠴ host{host=oxygen}
  ↳ ⠂ activate{}
    ↳ ⠄ host{host=hydrogen}
```

**Time T3 - All hosts finished building and copying, all activating:**
```
⠦ phase{phase="default" hosts=3}
  ↳ ⠂ activate{}
    ↳ ⠄ host{host=hydrogen}
    ↳ ⠦ host{host=oxygen}
    ↳ ⠴ host{host=carbon}
```
*Note: build and copy spans have disappeared*

**Time T4 - Hydrogen and oxygen rebooting, carbon still activating:**
```
⠦ phase{phase="default" hosts=3}
  ↳ ⠂ activate{}
    ↳ ⠴ host{host=carbon}
  ↳ ⠄ reboot{}
    ↳ ⠦ host{host=hydrogen}
    ↳ ⠂ host{host=oxygen}
```

**Time T5 - All done:**
```
⠦ phase{phase="default" hosts=3}
```
*All step spans have disappeared, phase completes*

### What Makes This "Dynamic"

The **key feature** that makes this UI powerful:

1. **Empty steps are hidden** - You never see empty step spans
2. **Steps appear on-demand** - A step span only appears when the first host enters it
3. **Steps disappear when done** - A step span disappears as soon as the last host leaves it
4. **Visual parallelism** - You can see multiple steps active simultaneously, showing true parallel execution
5. **Progress awareness** - You can see which steps are bottlenecks (many hosts) vs which are flowing quickly

## Current Architecture (What We're Changing From)

Currently, `execute_phase()` runs steps **sequentially for all hosts**:

```rust
fn execute_phase() {
    // All hosts build (in parallel via execute_step)
    let build_outputs = execute_step(&build_step, ...);

    // All hosts copy (in parallel via execute_step)
    let copy_outputs = execute_step(&copy_step, ...);

    // All hosts activate (in parallel via execute_step)
    let activate_outputs = execute_step(&activate_step, ...);

    // All hosts reboot (in parallel via execute_step)
    let reboot_outputs = execute_step(&reboot_step, ...);
}
```

This means all hosts must finish building before any can start copying.

## New Architecture (What We're Changing To)

We want **parallel-per-host** execution where each host runs through all steps independently:

```rust
fn execute_phase() {
    // Each host processes independently in parallel:
    // - Host A: build -> copy -> activate -> reboot
    // - Host B: build -> copy -> activate -> reboot  (running simultaneously)
    // - Host C: build -> copy -> activate -> reboot  (running simultaneously)

    pool.install(|| {
        phase_hosts.par_iter().map(|host| {
            execute_host_pipeline(host, ...)
        }).collect()
    })
}

fn execute_host_pipeline(host) {
    // Build this host
    observer.on_build_start(hostname)?;
    let build_output = /* build logic */;
    observer.on_build_complete(hostname, success)?;

    // Copy this host
    observer.on_copy_start(hostname)?;
    let copy_output = /* copy logic */;
    observer.on_copy_complete(hostname, success)?;

    // Activate this host
    observer.on_deploy_start(hostname, mode)?;
    let deploy_result = /* activate logic */;
    observer.on_deploy_complete(hostname, &result)?;

    // Reboot this host (if needed)
    if should_reboot {
        observer.on_reboot_start(hostname)?;
        /* reboot logic */;
        observer.on_reboot_complete(hostname, success)?;
    }
}
```

This allows Host A to be activating while Host B is still building!

## Why Observer Pattern for Step Spans?

The observer pattern is perfect for managing step span lifecycle because:

1. **Thread-safe shared state** - Observer is wrapped in `Arc` and shared across worker threads, with internal `Mutex` for safe state access
2. **Simple state management** - Just track counters and spans with mutex protection
3. **Already exists** - We already have all the callbacks we need (`on_build_start`, `on_build_complete`, etc.)
4. **Works with parallel execution** - Observer methods are called from worker threads, but mutex ensures correct counter updates

**Important**: Observer callbacks happen from within worker threads (inside `execute_host_pipeline`), not the main thread. This means:
- We must use `Arc<&dyn PushObserver>` to share the observer across threads
- The observer needs internal `Mutex` to protect shared state (counters and spans)
- `EnteredSpan` is NOT `Send`, but it's fine because it stays in the mutex-protected state and is only accessed by one thread at a time (when mutex is locked)

## Implementation Steps

### 1. Add Dependencies

**File**: `snow/Cargo.toml`

Add:
```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-indicatif = "0.3"
indicatif = "0.17"
```

**File**: `snowlib/Cargo.toml`

Add:
```toml
tracing = "0.1"
parking_lot = "0.12.5"  # For Mutex in observer
```

### 2. Set Up Tracing in CLI

**File**: `snow/src/main.rs`

Add at the top of `main()` before running any commands:

```rust
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// Set up tracing with indicatif integration for progress bars
let indicatif_layer = tracing_indicatif::IndicatifLayer::new();
tracing_subscriber::registry()
    .with(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "snow=info,snowlib=info".into()),
    )
    .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
    .with(indicatif_layer)
    .init();
```

### 3. Update CliObserver for Dynamic Span Management

**File**: `snow/src/commands/push/observer.rs`

Replace the entire implementation with:

```rust
//! CLI observer implementation for push deployment progress reporting
//!
//! Manages dynamic step span lifecycle - step spans appear when the first host
//! enters that step and disappear when the last host leaves.

use snowlib::{ActivationMode, DeployResult, PushObserver, Result};
use parking_lot::Mutex;
use tracing::info_span;

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
    // Step spans - only visible when hosts are in that step
    build_span: Option<tracing::span::EnteredSpan>,
    copy_span: Option<tracing::span::EnteredSpan>,
    activate_span: Option<tracing::span::EnteredSpan>,
    reboot_span: Option<tracing::span::EnteredSpan>,

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
        if state.build_count == 0 {
            // First host entering build - create and enter span
            state.build_span = Some(info_span!("build").entered());
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

    fn on_build_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Copy step
    fn on_copy_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_copy_start(&self, _hostname: &str) -> Result<()> {
        let mut state = self.state.lock();
        if state.copy_count == 0 {
            state.copy_span = Some(info_span!("copy").entered());
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

    fn on_copy_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Deploy/Activate step
    fn on_deploy_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_deploy_start(&self, _hostname: &str, _mode: ActivationMode) -> Result<()> {
        let mut state = self.state.lock();
        if state.activate_count == 0 {
            state.activate_span = Some(info_span!("activate").entered());
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

    fn on_deploy_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // Reboot step
    fn on_reboot_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    fn on_reboot_start(&self, _hostname: &str) -> Result<()> {
        let mut state = self.state.lock();
        if state.reboot_count == 0 {
            state.reboot_span = Some(info_span!("reboot").entered());
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
    fn test_span_lifecycle() {
        let observer = CliObserver::new(false);

        // Initially no spans
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
}
```

### 4. Refactor execute_phase to Use Parallel Host Execution

**File**: `snowlib/src/push/mod.rs`

Replace the `execute_phase` function with:

```rust
use rayon::prelude::*;
use std::sync::Arc;

/// Execute all deployment steps for a single phase
///
/// Uses parallel-per-host execution where each host independently runs through
/// all steps (build -> copy -> activate -> reboot). This allows hosts to progress
/// at different rates - one host can be activating while another is still building.
fn execute_phase(
    phase: &str,
    phase_hosts: &[HostMetadata],
    flake: &Flake,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<Vec<(String, crate::backends::deploy::DeployResult)>> {
    use tracing::info_span;

    // Create phase span
    let phase_span = info_span!("phase", phase = phase, hosts = phase_hosts.len());
    let _entered = phase_span.enter();

    // Log phase start
    if phase == "default" || phase == "_all" {
        tracing::info!(hosts = phase_hosts.len(), "Starting deployment");
    } else {
        tracing::info!(phase = phase, hosts = phase_hosts.len(), "Starting phase");
    }

    // Wrap resources for parallel access
    let flake = Arc::new(flake);
    let push_ctx = Arc::new(push_ctx);
    let observer = Arc::new(observer);

    // Create thread pool with concurrency limit
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(push_ctx.config.concurrency)
        .build()
        .map_err(|e| {
            crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                host: "thread pool".to_string(),
                source: std::io::Error::other(e.to_string()),
            })
        })?;

    // Execute hosts in parallel - each host runs through all steps independently
    let results: Vec<Result<(String, crate::backends::deploy::DeployResult)>> = pool.install(|| {
        phase_hosts
            .par_iter()
            .map(|host_metadata| {
                execute_host_pipeline(
                    host_metadata,
                    Arc::clone(&flake),
                    Arc::clone(&push_ctx),
                    Arc::clone(&observer),
                )
            })
            .collect()
    });

    // Collect successful results and handle errors
    let mut phase_results = Vec::new();
    let mut has_error = false;
    let mut first_error = None;

    for result in results {
        match result {
            Ok((hostname, deploy_result)) => {
                phase_results.push((hostname, deploy_result));
            }
            Err(e) => {
                has_error = true;
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    // Determine overall success
    let phase_success = !has_error && phase_results.iter().all(|(_, r)| r.success);

    // Log phase completion
    if phase_success {
        if phase == "default" {
            tracing::info!("✓ Phase completed successfully");
        } else {
            tracing::info!(phase = phase, "✓ Phase completed successfully");
        }
    } else if phase == "default" {
        tracing::error!("✗ Phase failed, stopping deployment");
    } else {
        tracing::error!(phase = phase, "✗ Phase failed, stopping deployment");
    }

    // Return first error if any
    if let Some(err) = first_error {
        return Err(err);
    }

    Ok(phase_results)
}
```

### 5. Create execute_host_pipeline Function

**File**: `snowlib/src/push/mod.rs`

Add this new function that runs one host through all steps:

```rust
/// Execute the full deployment pipeline for a single host
///
/// Runs: Build → Copy → Plan → Activate → Reboot
///
/// This function is called in parallel for each host in a phase. Observer callbacks
/// are used to track when hosts enter/leave steps, enabling dynamic step span visibility.
fn execute_host_pipeline(
    host_metadata: &HostMetadata,
    flake: Arc<&Flake>,
    push_ctx: Arc<&PushContext>,
    observer: Arc<&dyn PushObserver>,
) -> Result<(String, crate::backends::deploy::DeployResult)> {
    use tracing::info_span;

    let hostname = &host_metadata.name;

    // Deref Arc to get the actual references
    let flake_ref = flake.as_ref();
    let push_ctx_ref = push_ctx.as_ref();
    let observer_ref = observer.as_ref();

    // Create host span (will be child of current step span)
    let host_span = info_span!("host", host = %hostname);
    let _host_entered = host_span.enter();

    // Step 1: Build
    observer_ref.on_build_start(hostname)?;
    let build_output = {
        tracing::info!("Building configuration");

        match push_ctx_ref.nix_backend.build_host(flake_ref, hostname, push_ctx_ref.config.quiet) {
            Ok(store_path) => {
                tracing::info!("✓ Build completed");

                // Create deployment target
                let target = crate::backends::deploy::DeployTarget {
                    user: push_ctx_ref.config.ssh_user.clone(),
                    hostname: host_metadata.fqdn.clone(),
                    port: None,
                };

                BuildOutput {
                    metadata: host_metadata.clone(),
                    target,
                    store_path,
                }
            }
            Err(e) => {
                tracing::error!("✗ Build failed");
                observer_ref.on_build_complete(hostname, false)?;
                return Err(e);
            }
        }
    };
    observer_ref.on_build_complete(hostname, true)?;

    // Step 2: Copy
    observer_ref.on_copy_start(hostname)?;
    let copy_output = {
        tracing::info!("Copying closure");

        match push_ctx_ref.deploy_backend.copy_closure(&build_output.target, &build_output.store_path) {
            Ok(()) => {
                tracing::info!("✓ Copy completed");
                CopyOutput {
                    metadata: build_output.metadata,
                    target: build_output.target,
                    store_path: build_output.store_path,
                }
            }
            Err(e) => {
                tracing::error!("✗ Copy failed");
                observer_ref.on_copy_complete(hostname, false)?;
                return Err(e);
            }
        }
    };
    observer_ref.on_copy_complete(hostname, true)?;

    // Step 3: Planning
    let (plans, blocked) = steps::plan::create_deployment_plans(
        vec![(copy_output.metadata.clone(), copy_output.target.clone(), copy_output.store_path.clone())],
        push_ctx_ref.reboot_backend,
        push_ctx_ref.config.mode,
        push_ctx_ref.config.force,
    )?;

    // Handle blocked hosts
    if !blocked.is_empty() {
        return Err(crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
            host: blocked[0].hostname.clone(),
            source: std::io::Error::other(blocked[0].reason.clone()),
        }));
    }

    // Should only have one plan
    let plan = plans.into_iter().next().ok_or_else(|| {
        crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
            host: hostname.clone(),
            source: std::io::Error::other("No deployment plan created"),
        })
    })?;

    // Step 4: Activate
    observer_ref.on_deploy_start(hostname, plan.activation_mode)?;
    let activate_result = {
        tracing::info!("Activating configuration");

        let result = push_ctx_ref.deploy_backend.deploy(
            &plan.target,
            &plan.store_path,
            plan.activation_mode,
        ).unwrap_or_else(|e| {
            // Build comprehensive error message with error chain
            let mut error_msg = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(err) = source {
                error_msg.push_str(&format!("\n  Caused by: {}", err));
                source = std::error::Error::source(err);
            }
            crate::backends::deploy::DeployResult::failure(error_msg)
        });

        if result.success {
            tracing::info!("✓ Activation completed");
        } else {
            tracing::error!(message = %result.message, "✗ Activation failed");
        }

        result
    };
    observer_ref.on_deploy_complete(hostname, &activate_result)?;

    if !activate_result.success {
        return Err(crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
            host: hostname.clone(),
            source: std::io::Error::other(activate_result.message.clone()),
        }));
    }

    // Step 5: Reboot (if needed)
    let reboot_check = plan.reboot_check.as_ref();
    let should_reboot = reboot_check.map(|c| c.required).unwrap_or(false) && push_ctx_ref.config.allow_reboot;

    let final_result = if should_reboot {
        observer_ref.on_reboot_start(hostname)?;
        tracing::info!("Rebooting host");

        match push_ctx_ref.reboot_backend.reboot_and_wait(&plan.target) {
            Ok(()) => {
                tracing::info!("✓ Back online");
                observer_ref.on_reboot_complete(hostname, true)?;
                activate_result
            }
            Err(e) => {
                tracing::error!("✗ Reboot failed");
                observer_ref.on_reboot_complete(hostname, false)?;
                crate::backends::deploy::DeployResult::failure(format!(
                    "Deployment succeeded but reboot failed: {}",
                    e
                ))
            }
        }
    } else {
        activate_result
    };

    Ok((hostname.clone(), final_result))
}
```

### 6. Update Imports in push/mod.rs

Make sure these imports are present at the top of `snowlib/src/push/mod.rs`:

```rust
use crate::backends::deploy::DeployResult;
use crate::error::{Result, SnowError};
use crate::nix::{Flake, HostMetadata};
use crate::observer::PushObserver;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
```

### 7. Remove Old execute_step Function and Step Execution Code

After implementing `execute_host_pipeline`, you can **remove or keep as backup**:
- The old `execute_step()` helper function
- The step-by-step execution code in the old `execute_phase`

The new `execute_host_pipeline` contains all the step logic inline.

### 8. Update Observer Trait to Require Send + Sync

**File**: `snowlib/src/observer.rs`

Ensure the trait definition includes `Send + Sync`:

```rust
/// Observer trait for push deployment lifecycle events
pub trait PushObserver: Send + Sync {
    // ... methods
}
```

## Testing Strategy

1. **Test with single host**:
   - Verify spans appear and disappear correctly
   - Check that logs appear under the correct host span

2. **Test with multiple hosts at different speeds**:
   - Use hosts that take different amounts of time to build
   - Verify you see hosts in different steps simultaneously
   - Confirm step spans appear/disappear correctly

3. **Test with different concurrency levels**:
   - Try concurrency=1 (sequential) vs concurrency=4 (parallel)
   - Ensure parallel execution works correctly

4. **Test error cases**:
   - Verify spans are cleaned up even when builds/deploys fail
   - Ensure observer counters don't get corrupted on errors

5. **Test multi-phase deployments**:
   - Verify each phase creates its own phase span
   - Confirm step spans are scoped to the current phase

## Key Implementation Notes

### Observer Call Pattern

**Always call observer completion even on error** to keep counters accurate:

```rust
observer.on_build_start(hostname)?;
match build_logic() {
    Ok(output) => {
        observer.on_build_complete(hostname, true)?;
        output
    }
    Err(e) => {
        observer.on_build_complete(hostname, false)?;  // <- Important!
        return Err(e);
    }
}
```

### Span Parenting

Host spans are created inside worker threads (in `execute_host_pipeline`). They will automatically appear under the currently active step span because:
1. When a worker thread calls `observer.on_build_start()`, the observer (from that worker thread) enters the build span, making it the current tracing parent
2. The host span created immediately after in that same thread becomes a child of the build span
3. tracing-indicatif shows host spans nested under step spans in the tree UI
4. The step span is shared globally via tracing's span registry, so all threads see the same span hierarchy

### Error Handling

When a host fails, it should:
1. Log the error via tracing
2. Call the observer completion callback (with `success=false`)
3. Return an error from `execute_host_pipeline`
4. The phase will collect errors and fail the entire phase

### Performance

The concurrency limit (from `push_ctx.config.concurrency`) controls how many hosts run in parallel. This prevents overwhelming the system when deploying to many hosts.

## Benefits of This Approach

1. ✅ **Parallel execution** - Hosts progress independently, much faster deployment
2. ✅ **Dynamic UI** - Step spans only appear when hosts are in them
3. ✅ **Simple observer** - No Arc/Mutex/FnOnce complexity in span management
4. ✅ **No threading issues** - Observer runs in main thread, can safely hold `EnteredSpan`
5. ✅ **Visual parallelism** - See multiple steps active simultaneously
6. ✅ **Accurate tracing** - Direct tracing events from worker threads
7. ✅ **Clean separation** - Observer handles UI/presentation, workers handle execution

## Migration Checklist

- [ ] Add dependencies to Cargo.toml files
- [ ] Set up tracing in CLI main.rs
- [ ] Update CliObserver with span management and counters
- [ ] Replace execute_phase with parallel version
- [ ] Add execute_host_pipeline function
- [ ] Update imports
- [ ] Remove old execute_step code (optional)
- [ ] Ensure PushObserver trait requires Send + Sync
- [ ] Test with single host
- [ ] Test with multiple hosts
- [ ] Test error cases
- [ ] Test multi-phase deployments
