# Migration Sequence: Phased Approach to Dynamic Step Spans

This document breaks down the refactor into independent, self-contained commits that can be tested and merged incrementally.

## Overview

We're breaking this into 5 phases:
1. **Phase 1**: Add tracing infrastructure (no behavior change)
2. **Phase 2**: Implement observer-based span management (still step-based execution)
3. **Phase 3**: Refactor to parallel-per-host execution
4. **Phase 4**: Add comprehensive tracing events
5. **Phase 5**: Cleanup and optimization

Each phase can be committed independently and the system remains functional throughout.

---

## Phase 1: Add Tracing Infrastructure

**Goal**: Set up tracing dependencies and initialization without changing any existing behavior.

### Changes

**File**: `snow/Cargo.toml`
```toml
# Add these dependencies
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-indicatif = "0.3"
indicatif = "0.17"
```

**File**: `snowlib/Cargo.toml`
```toml
# Add these dependencies
tracing = "0.1"
parking_lot = "0.12.5"
```

**File**: `snow/src/main.rs`
```rust
// Add at the top of main(), before the CLI parsing
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// Set up tracing with indicatif integration
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

### Testing

```bash
# Build should succeed
cargo build

# Run push command - should work exactly as before
cargo run --bin snow -- push

# No visible changes yet, but tracing is ready
```

### Commit Message

```
feat: Add tracing infrastructure for progress reporting

- Add tracing, tracing-subscriber, tracing-indicatif dependencies
- Initialize tracing subscriber in CLI main
- No behavior changes - infrastructure only

This sets up the foundation for dynamic progress indicators
in future commits.
```

---

## Phase 2: Implement Observer-Based Span Management (Step-Based)

**Goal**: Update CliObserver to manage step spans dynamically, but keep the current step-based execution model. This proves the observer pattern works before we change the execution model.

**Important Threading Note**: In this phase, observer callbacks still happen from the main thread (within `execute_step`). However, we're implementing it with `Mutex` from the start because Phase 3 will call these callbacks from worker threads. The `EnteredSpan` is NOT `Send`, but it's safe because it's protected by the mutex and only accessed by one thread at a time when locked.

### Changes

**File**: `snowlib/src/observer.rs`

Update the trait to require `Send + Sync`:
```rust
/// Observer trait for push deployment lifecycle events
pub trait PushObserver: Send + Sync {
    // ... existing methods
}
```

**File**: `snow/src/commands/push/observer.rs`

Replace the entire implementation:

```rust
//! CLI observer implementation for push deployment progress reporting
//!
//! Manages dynamic step span lifecycle - step spans appear when the first host
//! enters that step and disappear when the last host leaves.

use snowlib::{ActivationMode, DeployResult, PushObserver, Result};
use parking_lot::Mutex;
use tracing::info_span;

/// CLI observer that manages step span lifecycle
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
            state.build_span = Some(info_span!("build").entered());
        }
        state.build_count += 1;
        Ok(())
    }

    fn on_build_complete(&self, _hostname: &str, _success: bool) -> Result<()> {
        let mut state = self.state.lock();
        state.build_count = state.build_count.saturating_sub(1);
        if state.build_count == 0 {
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
            assert!(state.copy_span.is_some());   // Copy span appeared
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
```

**File**: `snowlib/src/push/mod.rs`

Add phase span to `execute_phase`:

```rust
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

    // Rest of the function remains the same...
    // Step 1: Build
    let build_inputs: Vec<BuildInput> = phase_hosts
        .iter()
        .map(|h| BuildInput {
            metadata: h.clone(),
        })
        .collect();
    let build_step = BuildStep { flake, push_ctx };
    let build_outputs = execute_step(&build_step, phase, build_inputs, observer)?;

    // ... rest of existing code unchanged
}
```

### Testing

```bash
# Run tests
cargo test

# Test observer span lifecycle
cargo test -p snow test_build_span_lifecycle
cargo test -p snow test_multiple_step_spans
cargo test -p snow test_error_handling_maintains_counter

# Run actual deployment - should see step spans appear/disappear
# In step-based execution, all hosts will be in the same step at once
cargo run --bin snow -- push

# Expected output:
# ⠦ phase{phase="default" hosts=3}
#   ↳ ⠤ build{}
#   (all hosts finish build, build span disappears)
#   ↳ ⠄ copy{}
#   (all hosts finish copy, copy span disappears)
#   etc.
```

### Commit Message

```
feat: Implement observer-based dynamic step span management

- Update PushObserver trait to require Send + Sync
- Refactor CliObserver to manage step span lifecycle
- Add span counters to track hosts in each step
- Step spans appear when first host enters, disappear when last leaves
- Add comprehensive tests for span lifecycle
- Add phase span to execute_phase

Behavior: Still step-based execution (all hosts do step N before step N+1)
Next: Parallel-per-host execution will enable true visual parallelism

Tests validate that:
- Spans appear/disappear correctly based on host count
- Multiple spans can coexist
- Error handling maintains correct counters
- Phase cleanup works correctly
```

---

## Phase 3: Refactor to Parallel-Per-Host Execution

**Goal**: Change execution model from step-based to parallel-per-host while keeping all existing functionality.

**Critical Threading Change**: In this phase, observer callbacks will be called from Rayon worker threads (inside `execute_host_pipeline`), NOT from the main thread. This is why we:
- Wrap observer in `Arc<&dyn PushObserver>` to share it across threads
- Already implemented `Mutex` in Phase 2 to protect the observer state
- The `Mutex` ensures that even though multiple threads call observer methods, the state (counters and `EnteredSpan` objects) is accessed safely - only one thread can hold the lock at a time

### Changes

**File**: `snowlib/src/push/mod.rs`

Add imports at the top:
```rust
use rayon::prelude::*;
use std::sync::Arc;
```

Replace `execute_phase` function:

```rust
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

    // Return first error if any
    if let Some(err) = first_error {
        return Err(err);
    }

    Ok(phase_results)
}
```

Add new `execute_host_pipeline` function:

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
    let flake_ref = flake.as_ref();
    let push_ctx_ref = push_ctx.as_ref();
    let observer_ref = observer.as_ref();

    // Create host span
    let host_span = info_span!("host", host = %hostname);
    let _host_entered = host_span.enter();

    // Step 1: Build
    observer_ref.on_build_start(hostname)?;
    let build_output = {
        match push_ctx_ref.nix_backend.build_host(flake_ref, hostname, push_ctx_ref.config.quiet) {
            Ok(store_path) => {
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
                observer_ref.on_build_complete(hostname, false)?;
                return Err(e);
            }
        }
    };
    observer_ref.on_build_complete(hostname, true)?;

    // Step 2: Copy
    observer_ref.on_copy_start(hostname)?;
    let copy_output = {
        match push_ctx_ref.deploy_backend.copy_closure(&build_output.target, &build_output.store_path) {
            Ok(()) => {
                CopyOutput {
                    metadata: build_output.metadata,
                    target: build_output.target,
                    store_path: build_output.store_path,
                }
            }
            Err(e) => {
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

    if !blocked.is_empty() {
        return Err(crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
            host: blocked[0].hostname.clone(),
            source: std::io::Error::other(blocked[0].reason.clone()),
        }));
    }

    let plan = plans.into_iter().next().ok_or_else(|| {
        crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
            host: hostname.clone(),
            source: std::io::Error::other("No deployment plan created"),
        })
    })?;

    // Step 4: Activate
    observer_ref.on_deploy_start(hostname, plan.activation_mode)?;
    let activate_result = {
        push_ctx_ref.deploy_backend.deploy(
            &plan.target,
            &plan.store_path,
            plan.activation_mode,
        ).unwrap_or_else(|e| {
            let mut error_msg = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(err) = source {
                error_msg.push_str(&format!("\n  Caused by: {}", err));
                source = std::error::Error::source(err);
            }
            crate::backends::deploy::DeployResult::failure(error_msg)
        })
    };
    observer_ref.on_deploy_complete(hostname, &activate_result)?;

    if !activate_result.success {
        return Err(crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
            host: hostname.clone(),
            source: std::io::Error::other(activate_result.message.clone()),
        }));
    }

    // Step 5: Reboot
    let reboot_check = plan.reboot_check.as_ref();
    let should_reboot = reboot_check.map(|c| c.required).unwrap_or(false) && push_ctx_ref.config.allow_reboot;

    let final_result = if should_reboot {
        observer_ref.on_reboot_start(hostname)?;
        match push_ctx_ref.reboot_backend.reboot_and_wait(&plan.target) {
            Ok(()) => {
                observer_ref.on_reboot_complete(hostname, true)?;
                activate_result
            }
            Err(e) => {
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

**Optional**: Mark `execute_step` as deprecated or remove it (keep as backup initially):

```rust
// This function is no longer used after parallel-per-host refactor
// Kept temporarily for reference
#[allow(dead_code)]
fn execute_step<S: Step>(
    step: &S,
    phase: &str,
    inputs: Vec<S::Input>,
    observer: &dyn PushObserver,
) -> Result<Vec<S::Output>> {
    // ... existing code
}
```

### Testing

```bash
# Run all tests
cargo test

# Test with single host - should work exactly as before
cargo run --bin snow -- push hydrogen

# Test with multiple hosts - NOW you should see parallel execution!
cargo run --bin snow -- push hydrogen oxygen carbon

# Expected output (hosts progress independently):
# ⠦ phase{phase="default" hosts=3}
#   ↳ ⠤ build{}
#     ↳ ⠂ host{host=hydrogen}
#     ↳ ⠴ host{host=oxygen}
#   ↳ ⠄ copy{}
#     ↳ ⠦ host{host=carbon}  <- carbon finished build while others still building!

# Test error handling - if one host fails, others should continue
# Test with different concurrency levels
cargo run --bin snow -- push --concurrency 1  # Sequential
cargo run --bin snow -- push --concurrency 4  # Parallel

# Test multi-phase deployments
cargo run --bin snow -- push  # All hosts with phases
```

### Commit Message

```
refactor: Change to parallel-per-host execution model

BREAKING CHANGE: Execution model changed from step-based to parallel-per-host

Previous: All hosts complete step N before any start step N+1
New: Each host runs through all steps independently in parallel

Benefits:
- Faster deployments (hosts don't wait for slowest in each step)
- Visual parallelism in progress UI
- Better resource utilization

Changes:
- Refactor execute_phase to use Rayon thread pool
- Add execute_host_pipeline for per-host execution
- Wrap shared resources in Arc for thread safety
- Mark execute_step as deprecated (kept for reference)

Observer callbacks now truly enable dynamic step span visibility:
- Multiple step spans can be active simultaneously
- Step spans show exactly which hosts are in which steps

Testing: All existing tests pass. Verified with single host, multiple
hosts, error cases, and multi-phase deployments.
```

---

## Phase 4: Add Comprehensive Tracing Events

**Goal**: Add detailed tracing events throughout the pipeline for better visibility.

### Changes

**File**: `snowlib/src/push/mod.rs`

Update `execute_host_pipeline` to add tracing events:

```rust
fn execute_host_pipeline(
    // ... parameters
) -> Result<(String, crate::backends::deploy::DeployResult)> {
    use tracing::info_span;

    let hostname = &host_metadata.name;
    let flake_ref = flake.as_ref();
    let push_ctx_ref = push_ctx.as_ref();
    let observer_ref = observer.as_ref();

    let host_span = info_span!("host", host = %hostname);
    let _host_entered = host_span.enter();

    // Step 1: Build
    observer_ref.on_build_start(hostname)?;
    let build_output = {
        tracing::info!("Building configuration");  // <- ADD

        match push_ctx_ref.nix_backend.build_host(flake_ref, hostname, push_ctx_ref.config.quiet) {
            Ok(store_path) => {
                tracing::info!("✓ Build completed");  // <- ADD

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
                tracing::error!("✗ Build failed");  // <- ADD
                observer_ref.on_build_complete(hostname, false)?;
                return Err(e);
            }
        }
    };
    observer_ref.on_build_complete(hostname, true)?;

    // Step 2: Copy
    observer_ref.on_copy_start(hostname)?;
    let copy_output = {
        tracing::info!("Copying closure");  // <- ADD

        match push_ctx_ref.deploy_backend.copy_closure(&build_output.target, &build_output.store_path) {
            Ok(()) => {
                tracing::info!("✓ Copy completed");  // <- ADD

                CopyOutput {
                    metadata: build_output.metadata,
                    target: build_output.target,
                    store_path: build_output.store_path,
                }
            }
            Err(e) => {
                tracing::error!("✗ Copy failed");  // <- ADD
                observer_ref.on_copy_complete(hostname, false)?;
                return Err(e);
            }
        }
    };
    observer_ref.on_copy_complete(hostname, true)?;

    // Step 3: Planning (no user-visible messages needed)

    // Step 4: Activate
    observer_ref.on_deploy_start(hostname, plan.activation_mode)?;
    let activate_result = {
        tracing::info!("Activating configuration");  // <- ADD

        let result = push_ctx_ref.deploy_backend.deploy(
            &plan.target,
            &plan.store_path,
            plan.activation_mode,
        ).unwrap_or_else(|e| {
            let mut error_msg = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(err) = source {
                error_msg.push_str(&format!("\n  Caused by: {}", err));
                source = std::error::Error::source(err);
            }
            crate::backends::deploy::DeployResult::failure(error_msg)
        });

        if result.success {
            tracing::info!("✓ Activation completed");  // <- ADD
        } else {
            tracing::error!(message = %result.message, "✗ Activation failed");  // <- ADD
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

    // Step 5: Reboot
    let reboot_check = plan.reboot_check.as_ref();
    let should_reboot = reboot_check.map(|c| c.required).unwrap_or(false) && push_ctx_ref.config.allow_reboot;

    let final_result = if should_reboot {
        observer_ref.on_reboot_start(hostname)?;
        tracing::info!("Rebooting host");  // <- ADD

        match push_ctx_ref.reboot_backend.reboot_and_wait(&plan.target) {
            Ok(()) => {
                tracing::info!("✓ Back online");  // <- ADD
                observer_ref.on_reboot_complete(hostname, true)?;
                activate_result
            }
            Err(e) => {
                tracing::error!("✗ Reboot failed");  // <- ADD
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

Also add phase-level logging in `execute_phase`:

```rust
fn execute_phase(
    phase: &str,
    phase_hosts: &[HostMetadata],
    flake: &Flake,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<Vec<(String, crate::backends::deploy::DeployResult)>> {
    use tracing::info_span;

    let phase_span = info_span!("phase", phase = phase, hosts = phase_hosts.len());
    let _entered = phase_span.enter();

    // Log phase start  <- ADD
    if phase == "default" || phase == "_all" {
        tracing::info!(hosts = phase_hosts.len(), "Starting deployment");
    } else {
        tracing::info!(phase = phase, hosts = phase_hosts.len(), "Starting phase");
    }

    // ... parallel execution code ...

    // Determine overall success
    let phase_success = !has_error && phase_results.iter().all(|(_, r)| r.success);

    // Log phase completion  <- ADD
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

### Testing

```bash
# Run tests
cargo test

# Run deployment and verify logs appear
cargo run --bin snow -- push hydrogen oxygen

# Should see logs like:
# ⠦ phase{phase="default" hosts=2}
#   2025-01-13T10:30:00Z  INFO Starting deployment hosts=2
#   ↳ ⠤ build{}
#     ↳ ⠂ host{host=hydrogen}
#       2025-01-13T10:30:01Z  INFO Building configuration
#       2025-01-13T10:30:10Z  INFO ✓ Build completed
#     ↳ ⠴ host{host=oxygen}
#       2025-01-13T10:30:01Z  INFO Building configuration
#       2025-01-13T10:30:15Z  INFO ✓ Build completed
#   ↳ ⠄ copy{}
#     ↳ ⠦ host{host=hydrogen}
#       2025-01-13T10:30:10Z  INFO Copying closure
#       2025-01-13T10:30:20Z  INFO ✓ Copy completed

# Test error logging
# (Force an error and verify ✗ messages appear)
```

### Commit Message

```
feat: Add comprehensive tracing events for deployment progress

Add detailed tracing events throughout the deployment pipeline:

Per-host events:
- Build: "Building configuration", "✓ Build completed", "✗ Build failed"
- Copy: "Copying closure", "✓ Copy completed", "✗ Copy failed"
- Activate: "Activating configuration", "✓ Activation completed", "✗ Activation failed"
- Reboot: "Rebooting host", "✓ Back online", "✗ Reboot failed"

Phase events:
- Phase start: "Starting deployment" / "Starting phase"
- Phase completion: "✓ Phase completed successfully" / "✗ Phase failed"

These events appear as logs under the host spans in the progress tree,
providing detailed visibility into what's happening at each step.

Testing: Verified logs appear correctly for success and error cases.
```

---

## Phase 5: Cleanup and Optimization

**Goal**: Remove deprecated code, optimize performance, and add final polish.

### Changes

**File**: `snowlib/src/push/mod.rs`

Remove the old `execute_step` function entirely:

```rust
// DELETE THIS FUNCTION:
// fn execute_step<S: Step>(...) { ... }
```

Add better error context:

```rust
fn execute_host_pipeline(
    // ...
) -> Result<(String, crate::backends::deploy::DeployResult)> {
    // Wrap all errors with hostname context for better debugging
    let result = (|| {
        // ... existing pipeline code ...
    })();

    result.map_err(|e| {
        // Add hostname to error context if not already there
        match &e {
            crate::error::SnowError::Deploy(deploy_err) => {
                if !deploy_err.to_string().contains(hostname) {
                    tracing::error!(host = %hostname, error = %e, "Host deployment failed");
                }
            }
            _ => {
                tracing::error!(host = %hostname, error = %e, "Host deployment failed");
            }
        }
        e
    })
}
```

**File**: `snow/src/commands/push/observer.rs`

Add documentation comments:

```rust
/// CLI observer that manages step span lifecycle dynamically
///
/// # How It Works
///
/// The observer tracks how many hosts are currently in each deployment step
/// (build, copy, activate, reboot). When the first host enters a step, the
/// observer creates and enters a tracing span for that step. When the last
/// host leaves, the span is dropped and disappears from the UI.
///
/// This creates a dynamic progress view where:
/// - Step spans only appear when hosts are actively in them
/// - Multiple step spans can be visible simultaneously (true parallelism)
/// - Empty steps are never shown
///
/// # Thread Safety
///
/// Uses a `Mutex` to protect internal state since observer methods may be
/// called from multiple threads (via Rayon's parallel execution). The mutex
/// ensures counters and span references remain consistent.
pub struct CliObserver {
    state: Mutex<ObserverState>,
}
```

**File**: `REFACTOR_PLAN.md` and `MIGRATION_SEQUENCE.md`

Update to reflect completed migration and document final architecture.

### Testing

```bash
# Full test suite
cargo test

# Integration tests with real deployment
cargo run --bin snow -- push --all

# Performance test with many hosts (if available)
cargo run --bin snow -- push --concurrency 8

# Verify no regressions
# - Single host deployment
# - Multiple hosts with different speeds
# - Error handling (one host fails)
# - Multi-phase deployments
# - Dry-run mode
# - Boot mode vs switch mode
# - Reboot handling
```

### Commit Message

```
refactor: Remove deprecated code and add final polish

Cleanup after parallel-per-host migration:

- Remove deprecated execute_step function
- Add comprehensive documentation to CliObserver
- Add better error context with hostname
- Update migration docs to reflect completed work

All tests passing. Migration complete!

The new architecture provides:
✅ Parallel-per-host execution for faster deployments
✅ Dynamic step spans that appear/disappear based on active hosts
✅ Visual parallelism showing true concurrent execution
✅ Detailed tracing events for debugging
✅ Simpler observer pattern for span management
```

---

## Migration Summary

### Commits Created

1. ✅ **Phase 1**: Add tracing infrastructure (no behavior change)
2. ✅ **Phase 2**: Implement observer-based span management (step-based)
3. ✅ **Phase 3**: Refactor to parallel-per-host execution (BREAKING)
4. ✅ **Phase 4**: Add comprehensive tracing events
5. ✅ **Phase 5**: Cleanup and optimization

### Testing Checklist

After each phase, verify:

- [ ] `cargo build` succeeds
- [ ] `cargo test` passes (all tests)
- [ ] `cargo run --bin snow -- push hydrogen` works (single host)
- [ ] `cargo run --bin snow -- push hydrogen oxygen` works (multiple hosts)
- [ ] Error handling works (forced failure)
- [ ] Multi-phase deployments work
- [ ] Different concurrency levels work
- [ ] Dry-run mode works
- [ ] Boot vs switch modes work
- [ ] Reboot handling works

### Rollback Strategy

Each phase is independently committable. If issues arise:

1. **Phase 1**: Just remove dependencies (pure addition)
2. **Phase 2**: Revert to old CliObserver (single commit revert)
3. **Phase 3**: Revert to step-based execution (single commit revert)
4. **Phase 4**: Remove tracing events (cosmetic change)
5. **Phase 5**: Revert cleanup (restore execute_step)

The most risky change is **Phase 3** (parallel execution), but it's isolated
in a single commit and can be reverted cleanly.

### Benefits of This Approach

1. **Incremental progress**: Each commit adds value
2. **Easy to review**: Small, focused commits
3. **Easy to debug**: If something breaks, bisect finds the exact commit
4. **Easy to rollback**: Any phase can be reverted independently
5. **Testable**: Each phase is fully functional and testable
6. **Low risk**: Changes are isolated and well-tested before moving on
