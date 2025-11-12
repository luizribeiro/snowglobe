use crate::error::Result;
use crate::observer::DeploymentObserver;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::deploy::reboot::{RebootCheck, RebootOps};
use crate::deploy::{ActivationMode, DeployResult, Deployer};
use crate::nix::ops::NixOps;
use crate::nix::{DeployTarget, HostMetadata, SnowContext, StorePath};

/// Configuration for push operations
#[derive(Debug, Clone)]
pub struct PushConfiguration {
    /// Deployment mode (auto, switch, boot, test, dry-run)
    pub mode: DeploymentMode,
    /// Force deployment even when reboot is detected in switch mode
    pub force: bool,
    /// Maximum number of parallel deployments within a phase
    pub concurrency: usize,
    /// Ignore push phases and deploy all hosts in parallel
    pub ignore_phases: bool,
    /// Suppress build output
    pub quiet: bool,
    /// SSH user for deployment (if None, uses SSH config default)
    pub ssh_user: Option<String>,
    /// Allow automatic reboots (false = never reboot, overrides policy/detection)
    pub allow_reboot: bool,
}

impl Default for PushConfiguration {
    fn default() -> Self {
        Self {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 3,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true, // Allow reboots by default
        }
    }
}

/// Deployment mode requested by the user
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentMode {
    /// Auto mode - detect reboot need and choose appropriate activation mode
    Auto,
    /// Force immediate activation (switch mode)
    Switch,
    /// Set for next boot only
    Boot,
    /// Test activation (non-persistent)
    Test,
    /// Dry run - preview only
    DryRun,
}

/// A deployment plan for a single host
#[derive(Debug, Clone)]
pub struct DeploymentPlan {
    pub metadata: HostMetadata,
    pub target: DeployTarget,
    pub store_path: StorePath,
    pub activation_mode: ActivationMode,
    pub reboot_check: Option<RebootCheck>,
}

/// A host that was blocked from deployment
#[derive(Debug, Clone)]
pub struct BlockedHost {
    pub hostname: String,
    pub reason: String,
}

/// Options for push operations (internal use only)
#[derive(Debug, Clone)]
struct PushOptions {
    activation_mode: ActivationMode,
    concurrency: usize,
}

impl Default for PushOptions {
    fn default() -> Self {
        Self {
            activation_mode: ActivationMode::Switch,
            concurrency: 3,
        }
    }
}

/// Result of pushing multiple hosts
#[derive(Debug)]
pub struct PushResults {
    pub results: Vec<(String, DeployResult)>, // (hostname, result)
}

impl PushResults {
    pub fn success_count(&self) -> usize {
        self.results.iter().filter(|(_, r)| r.success).count()
    }

    pub fn failure_count(&self) -> usize {
        self.results.iter().filter(|(_, r)| !r.success).count()
    }

    pub fn all_succeeded(&self) -> bool {
        self.results.iter().all(|(_, r)| r.success)
    }
}

/// Main push orchestration function - handles complete deployment workflow
///
/// This is the primary entry point for deploying configurations. It orchestrates
/// the entire deployment process by phase:
///
/// For each phase (sequential):
/// 1. **Build** - Build NixOS configurations for all hosts in the phase
/// 2. **Copy** - Copy closures to remote hosts
/// 3. **Plan** - Create deployment plans with reboot detection
/// 4. **Activate** - Activate configurations (parallel within phase)
/// 5. **Reboot** - Reboot hosts that need it
///
/// If any step fails in a phase, deployment stops immediately.
///
/// # Arguments
///
/// - `ctx`: Nix context (flake root)
/// - `nix_ops`: Nix operations interface (for building)
/// - `deployer`: Deployer implementation (for copying and activation)
/// - `reboot_detector`: Reboot detection and execution
/// - `hosts`: List of hosts to deploy
/// - `config`: Push configuration (mode, concurrency, etc.)
/// - `observer`: Observer to receive lifecycle events
///
/// # Example
///
/// ```no_run
/// use snowlib::{push, PushConfiguration, SnowContext, SystemNixOps, SshDeployer, RebootOps, NoopObserver};
///
/// # fn example(hosts: Vec<snowlib::HostMetadata>) -> snowlib::Result<()> {
/// let ctx = SnowContext::new(None)?;
/// let nix_ops = SystemNixOps::new();
/// let deployer = SshDeployer::new();
/// let reboot_ops: &dyn RebootOps = &snowlib::deploy::reboot::RebootDetector::default();
/// let config = PushConfiguration::default();
/// let observer = NoopObserver;
///
/// let results = push(&ctx, &nix_ops, &deployer, reboot_ops, hosts, &config, &observer)?;
/// # Ok(())
/// # }
/// ```
pub fn push(
    ctx: &SnowContext,
    nix_ops: &dyn NixOps,
    deployer: &dyn Deployer,
    reboot_ops: &dyn RebootOps,
    hosts: Vec<HostMetadata>,
    config: &PushConfiguration,
    observer: &dyn DeploymentObserver,
) -> Result<PushResults> {
    // Group hosts by phase
    let mut by_phase: HashMap<String, Vec<HostMetadata>> = HashMap::new();

    for host in hosts {
        let phase = if config.ignore_phases {
            "_all".to_string()
        } else {
            host.phase.clone().unwrap_or_else(|| "default".to_string())
        };
        by_phase.entry(phase).or_default().push(host);
    }

    // Sort phases lexicographically
    let mut phases: Vec<String> = by_phase.keys().cloned().collect();
    phases.sort();

    let mut all_results = Vec::new();

    // Execute each phase sequentially
    for phase in phases {
        let phase_hosts = by_phase.remove(&phase).unwrap();
        let host_names: Vec<String> = phase_hosts.iter().map(|h| h.name.clone()).collect();

        // STEP 1: Build configurations for this phase
        observer.on_build_phase_start(&phase, &host_names)?;

        let mut phase_host_data = Vec::new();
        for mut host in phase_hosts {
            observer.on_build_start(&host.name)?;

            match nix_ops.build_host(ctx, &host.name, config.quiet) {
                Ok(store_path) => {
                    observer.on_build_complete(&host.name, true)?;

                    // Create deployment target
                    let target = DeployTarget {
                        user: config.ssh_user.clone(),
                        hostname: host.fqdn.clone(),
                        port: None,
                    };

                    // Ensure the metadata reflects the current phase we're processing
                    // (important for ignore_phases mode where all hosts are grouped into "_all")
                    host.phase = Some(phase.clone());

                    phase_host_data.push((host, target, store_path));
                }
                Err(e) => {
                    observer.on_build_complete(&host.name, false)?;
                    return Err(e);
                }
            }
        }

        // STEP 2: Copy closures for this phase
        observer.on_copy_phase_start(&phase, &host_names)?;

        for (metadata, target, store_path) in &phase_host_data {
            observer.on_copy_start(&metadata.name)?;

            match deployer.copy_closure(target, store_path) {
                Ok(_) => {
                    observer.on_copy_complete(&metadata.name, true)?;
                }
                Err(e) => {
                    observer.on_copy_complete(&metadata.name, false)?;
                    return Err(e);
                }
            }
        }

        // STEP 3: Create deployment plans with reboot detection
        let (plans, blocked) =
            create_deployment_plans(phase_host_data, reboot_ops, config.mode, config.force)?;

        // Check for blocked hosts
        if !blocked.is_empty() {
            // For now, treat blocked hosts as deployment failures
            // TODO: Better error handling for blocked hosts
            return Err(crate::error::SnowError::Deploy(
                crate::error::DeployError::Failed {
                    host: blocked[0].hostname.clone(),
                    source: std::io::Error::other(blocked[0].reason.clone()),
                },
            ));
        }

        // STEP 4: Activate configurations for this phase
        let mut phase_results =
            activate_by_phase(deployer, plans.clone(), config.concurrency, observer)?;

        let phase_success = phase_results.all_succeeded();

        // STEP 5: Reboot hosts in this phase that need it
        if phase_success && config.allow_reboot {
            for plan in &plans {
                // Should reboot if using Boot mode AND either:
                // 1. Reboot check says reboot is required (Auto mode with detected reboot)
                // 2. No reboot check exists (explicit Boot mode)
                let should_reboot = plan.activation_mode == ActivationMode::Boot
                    && (plan.reboot_check.is_none()
                        || plan
                            .reboot_check
                            .as_ref()
                            .map(|c| c.required)
                            .unwrap_or(false));

                if should_reboot {
                    observer.on_reboot_start(&plan.metadata.name)?;

                    match reboot_ops.reboot_and_wait(&plan.target) {
                        Ok(_) => {
                            observer.on_reboot_complete(&plan.metadata.name, true)?;

                            // Update the result message
                            if let Some((_, result)) = phase_results
                                .results
                                .iter_mut()
                                .find(|(h, _)| h == &plan.metadata.name)
                            {
                                result.message = "Deployed & Rebooted".to_string();
                            }
                        }
                        Err(e) => {
                            observer.on_reboot_complete(&plan.metadata.name, false)?;

                            // Update the result to show reboot failure
                            if let Some((_, result)) = phase_results
                                .results
                                .iter_mut()
                                .find(|(h, _)| h == &plan.metadata.name)
                            {
                                result.success = false;
                                result.message = format!("Deployed but reboot failed: {}", e);
                            }
                        }
                    }
                }
            }
        }

        all_results.extend(phase_results.results);

        // If this phase failed, stop deployment
        if !phase_success {
            return Ok(PushResults {
                results: all_results,
            });
        }
    }

    Ok(PushResults {
        results: all_results,
    })
}

/// Activate hosts by phase with observer notifications
///
/// Internal function that handles the activation step of deployment.
/// Hosts are grouped by phase and activated sequentially, with parallel
/// activation within each phase.
///
/// # Arguments
///
/// - `deployer`: The deployer implementation (SSH, mock, etc.)
/// - `plans`: Deployment plans with activation modes already determined
/// - `concurrency`: Maximum number of parallel deployments within a phase
/// - `observer`: Observer to receive lifecycle events
///
/// # Note
///
/// This is an internal function. Use `push()` for the full deployment workflow.
fn activate_by_phase(
    deployer: &dyn Deployer,
    plans: Vec<DeploymentPlan>,
    concurrency: usize,
    observer: &dyn DeploymentObserver,
) -> Result<PushResults> {
    // Group plans by phase
    let mut by_phase: HashMap<String, Vec<DeploymentPlan>> = HashMap::new();

    for plan in plans {
        let phase = plan
            .metadata
            .phase
            .clone()
            .unwrap_or_else(|| "default".to_string());
        by_phase.entry(phase).or_default().push(plan);
    }

    // Sort phases lexicographically
    let mut phases: Vec<String> = by_phase.keys().cloned().collect();
    phases.sort();

    let mut all_results = Vec::new();

    // Execute each phase sequentially
    for phase in phases {
        let phase_plans = by_phase.remove(&phase).unwrap();

        // Collect host names for observer notification
        let host_names: Vec<String> = phase_plans
            .iter()
            .map(|p| p.metadata.name.clone())
            .collect();

        // Notify observer of phase start
        observer.on_phase_start(&phase, &host_names)?;

        // Within each phase, group by activation mode
        let mut by_mode: HashMap<ActivationMode, Vec<DeploymentPlan>> = HashMap::new();
        for plan in phase_plans {
            by_mode.entry(plan.activation_mode).or_default().push(plan);
        }

        let mut phase_success = true;

        // Deploy each activation mode group in parallel
        for (mode, mode_plans) in by_mode {
            let host_data: Vec<_> = mode_plans
                .iter()
                .map(|plan| {
                    (
                        plan.metadata.clone(),
                        plan.target.clone(),
                        plan.store_path.clone(),
                    )
                })
                .collect();

            let opts = PushOptions {
                activation_mode: mode,
                concurrency,
            };

            let mode_results =
                activate_parallel_with_observer(deployer, host_data, &opts, observer)?;

            // If any deployment in this mode group failed, mark phase as failed
            if !mode_results.all_succeeded() {
                phase_success = false;
            }

            all_results.extend(mode_results.results);
        }

        // Notify observer of phase completion
        observer.on_phase_complete(&phase, phase_success)?;

        // If this phase failed, stop deployment
        if !phase_success {
            return Ok(PushResults {
                results: all_results,
            });
        }
    }

    Ok(PushResults {
        results: all_results,
    })
}

/// Activate hosts in parallel with concurrency limit (with observer notifications)
///
/// Internal function used by `activate_by_phase()`. Activates hosts in parallel and emits
/// observer events for each deployment.
fn activate_parallel_with_observer(
    deployer: &dyn Deployer,
    hosts: Vec<(HostMetadata, DeployTarget, StorePath)>,
    opts: &PushOptions,
    observer: &dyn DeploymentObserver,
) -> Result<PushResults> {
    let deployer = Arc::new(deployer);
    let observer = Arc::new(observer);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.concurrency)
        .build()
        .map_err(|e| {
            crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                host: "thread pool".to_string(),
                source: std::io::Error::other(e.to_string()),
            })
        })?;

    let results: Vec<(String, DeployResult)> = pool.install(|| {
        hosts
            .into_par_iter()
            .map(|(metadata, target, store_path)| {
                let hostname = metadata.name.clone();
                let deployer = Arc::clone(&deployer);
                let observer = Arc::clone(&observer);

                // Notify observer of deployment start
                if let Err(e) = observer.on_deploy_start(&hostname, opts.activation_mode) {
                    return (
                        hostname,
                        DeployResult::failure(format!("Observer error: {}", e)),
                    );
                }

                // Perform deployment
                let result = deployer
                    .deploy(&target, &store_path, opts.activation_mode)
                    .unwrap_or_else(|e| {
                        let mut error_msg = e.to_string();
                        let mut source = std::error::Error::source(&e);
                        while let Some(err) = source {
                            error_msg.push_str(&format!("\n  Caused by: {}", err));
                            source = std::error::Error::source(err);
                        }
                        DeployResult::failure(error_msg)
                    });

                // Notify observer of deployment completion
                if let Err(e) = observer.on_deploy_complete(&hostname, &result) {
                    // If observer fails, we still return the deployment result
                    // but log the observer error in the result message
                    let mut modified_result = result.clone();
                    modified_result.message =
                        format!("{}\nObserver error: {}", modified_result.message, e);
                    return (hostname, modified_result);
                }

                (hostname, result)
            })
            .collect()
    });

    Ok(PushResults { results })
}

/// Strategy for handling a host deployment
enum ActivationStrategy {
    /// Deploy with the given activation mode and optional reboot check
    Deploy(ActivationMode, Option<RebootCheck>),
    /// Block deployment with the given reason
    Block(String),
}

/// Determine the activation strategy for a single host
fn determine_activation_strategy(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_ops: &dyn RebootOps,
    mode: DeploymentMode,
    force: bool,
) -> Result<ActivationStrategy> {
    match mode {
        DeploymentMode::DryRun => Ok(ActivationStrategy::Deploy(
            ActivationMode::DryActivate,
            None,
        )),
        DeploymentMode::Test => Ok(ActivationStrategy::Deploy(ActivationMode::Test, None)),
        DeploymentMode::Boot => Ok(ActivationStrategy::Deploy(ActivationMode::Boot, None)),
        DeploymentMode::Switch => {
            handle_switch_mode(metadata, target, store_path, reboot_ops, force)
        }
        DeploymentMode::Auto => handle_auto_mode(metadata, target, store_path, reboot_ops),
    }
}

/// Handle switch mode: force immediate activation with safety checks
fn handle_switch_mode(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_ops: &dyn RebootOps,
    force: bool,
) -> Result<ActivationStrategy> {
    use crate::nix::RebootPolicy;

    // Check reboot policy - 'never' and 'always' policies override switch mode
    match metadata.reboot_policy {
        RebootPolicy::Never => {
            // Never policy: use boot mode without reboot
            return Ok(ActivationStrategy::Deploy(
                ActivationMode::Boot,
                Some(RebootCheck::not_required()),
            ));
        }
        RebootPolicy::Always => {
            // Always policy: use boot mode and will reboot
            return Ok(ActivationStrategy::Deploy(
                ActivationMode::Boot,
                Some(RebootCheck::required(
                    crate::deploy::reboot::RebootReason::Policy,
                )),
            ));
        }
        RebootPolicy::Auto => {
            // Continue with normal switch mode logic
        }
    }

    let check = reboot_ops.check_reboot_required(target, store_path)?;

    if check.required && !force {
        // Reboot required but --force not set - block this host
        let reason = check
            .reason
            .as_ref()
            .map(|r| r.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(ActivationStrategy::Block(reason))
    } else {
        Ok(ActivationStrategy::Deploy(
            ActivationMode::Switch,
            Some(check),
        ))
    }
}

/// Handle auto mode: detect reboot requirement and choose activation mode
fn handle_auto_mode(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_ops: &dyn RebootOps,
) -> Result<ActivationStrategy> {
    use crate::nix::RebootPolicy;

    // Check reboot policy
    match metadata.reboot_policy {
        RebootPolicy::Never => {
            // Never policy: always use boot mode, never reboot
            return Ok(ActivationStrategy::Deploy(
                ActivationMode::Boot,
                Some(RebootCheck::not_required()),
            ));
        }
        RebootPolicy::Always => {
            // Always policy: always use boot mode and always reboot
            return Ok(ActivationStrategy::Deploy(
                ActivationMode::Boot,
                Some(RebootCheck::required(
                    crate::deploy::reboot::RebootReason::Policy,
                )),
            ));
        }
        RebootPolicy::Auto => {
            // Continue with automatic detection
        }
    }

    let check = reboot_ops.check_reboot_required(target, store_path)?;

    let activation = if check.required {
        ActivationMode::Boot
    } else {
        ActivationMode::Switch
    };

    Ok(ActivationStrategy::Deploy(activation, Some(check)))
}

/// Create deployment plans with reboot detection
///
/// This function determines the appropriate activation mode for each host based on:
/// - User-requested deployment mode
/// - Reboot detection results
/// - Force flag
///
/// Returns (plans, blocked_hosts) where blocked_hosts contains hosts that were
/// blocked by safety checks (e.g., --switch without --force when reboot required)
pub fn create_deployment_plans(
    hosts: Vec<(HostMetadata, DeployTarget, StorePath)>,
    reboot_ops: &dyn RebootOps,
    mode: DeploymentMode,
    force: bool,
) -> Result<(Vec<DeploymentPlan>, Vec<BlockedHost>)> {
    let mut plans = Vec::new();
    let mut blocked = Vec::new();

    for (metadata, target, store_path) in hosts {
        match determine_activation_strategy(
            &metadata,
            &target,
            &store_path,
            reboot_ops,
            mode,
            force,
        )? {
            ActivationStrategy::Deploy(activation_mode, reboot_check) => {
                plans.push(DeploymentPlan {
                    metadata,
                    target,
                    store_path,
                    activation_mode,
                    reboot_check,
                });
            }
            ActivationStrategy::Block(reason) => {
                blocked.push(BlockedHost {
                    hostname: metadata.name.clone(),
                    reason,
                });
            }
        }
    }

    Ok((plans, blocked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::mock::MockDeployer;
    use crate::nix::ops::mock::MockNixOps;
    use crate::nix::SnowContext;

    /// Helper to create just HostMetadata for push tests
    fn make_host_metadata(
        name: &str,
        phase: Option<&str>,
        reboot_policy: crate::nix::RebootPolicy,
    ) -> HostMetadata {
        HostMetadata {
            name: name.to_string(),
            phase: phase.map(|s| s.to_string()),
            tags: vec![],
            fqdn: format!("{}.example.com", name),
            reboot_policy,
        }
    }

    /// Helper to create a tuple for create_deployment_plans tests
    fn make_host(
        name: &str,
        phase: Option<&str>,
        reboot_policy: crate::nix::RebootPolicy,
    ) -> (HostMetadata, DeployTarget, StorePath) {
        let metadata = make_host_metadata(name, phase, reboot_policy);

        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: format!("{}.example.com", name),
            port: None,
        };

        let store_path = StorePath::new(format!("/nix/store/fake-{}", name));

        (metadata, target, store_path)
    }

    #[test]
    fn test_push_results_counts() {
        let results = PushResults {
            results: vec![
                ("host1".to_string(), DeployResult::success("ok")),
                ("host2".to_string(), DeployResult::failure("failed")),
                ("host3".to_string(), DeployResult::success("ok")),
            ],
        };

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 1);
        assert!(!results.all_succeeded());
    }

    #[test]
    fn test_push_results_all_succeeded() {
        let results = PushResults {
            results: vec![
                ("host1".to_string(), DeployResult::success("ok")),
                ("host2".to_string(), DeployResult::success("ok")),
            ],
        };

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);
        assert!(results.all_succeeded());
    }

    // Tests for create_deployment_plans
    use crate::deploy::reboot::{RebootDetectionConfig, RebootDetector};

    #[test]
    fn test_create_deployment_plans_auto_mode_no_reboot() {
        // Create a detector that will never detect reboot needed
        // This simulates the edge case where:
        // - No kernel/initrd/init changes
        // - No critical services to restart (empty list)
        // - /run/nixos/dry-activation-restart-list won't exist on remote
        // This configuration was causing "No such file or directory" errors
        // before we added the file existence check in check_critical_services()
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = RebootDetector::new(config);

        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use Switch mode since no reboot is required
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
        // Reboot check should indicate no reboot needed
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_create_deployment_plans_dry_run() {
        let detector = RebootDetector::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::DryRun, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::DryActivate);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_boot_mode() {
        let detector = RebootDetector::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Boot, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_test_mode() {
        let detector = RebootDetector::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Test, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::Test);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_switch_with_force() {
        // In reality, reboot would be detected, but with force flag it should still use Switch
        let detector = RebootDetector::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, true).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Force flag causes switch mode even if reboot detected
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
    }

    #[test]
    fn test_create_deployment_plans_multiple_hosts_mixed_results() {
        // Create a detector that will never detect reboot needed
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = RebootDetector::new(config);

        let hosts = vec![
            make_host("host1", None, crate::nix::RebootPolicy::Auto),
            make_host("host2", None, crate::nix::RebootPolicy::Auto),
            make_host("host3", None, crate::nix::RebootPolicy::Auto),
        ];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 3);
        assert_eq!(blocked.len(), 0);
        // All should use switch mode since no reboot detected
        for plan in plans {
            assert_eq!(plan.activation_mode, ActivationMode::Switch);
        }
    }

    #[test]
    fn test_reboot_policy_never_uses_boot_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = RebootDetector::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode due to 'never' policy
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        // Should indicate no reboot required
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_always_uses_boot_mode_with_reboot() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = RebootDetector::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode due to 'always' policy
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        // Should indicate reboot required
        assert!(plans[0].reboot_check.is_some());
        assert!(plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_auto_uses_detection() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = RebootDetector::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use switch mode (no reboot detected)
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_never_overrides_switch_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = RebootDetector::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode despite switch flag (policy overrides)
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_always_overrides_switch_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = RebootDetector::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode despite switch flag (policy overrides)
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(plans[0].reboot_check.as_ref().unwrap().required);
    }

    // Tests for the new push() function with observer support
    use crate::observer::{DeploymentObserver, NoopObserver};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct TestObserver {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl TestObserver {
        fn new() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn get_events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl DeploymentObserver for TestObserver {
        fn on_build_phase_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "build_phase_start:{}:{}",
                phase,
                hosts.len()
            ));
            Ok(())
        }

        fn on_build_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("build_start:{}", hostname));
            Ok(())
        }

        fn on_build_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "build_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_copy_phase_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("copy_phase_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_copy_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("copy_start:{}", hostname));
            Ok(())
        }

        fn on_copy_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "copy_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_phase_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("phase_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_deploy_start(
            &self,
            hostname: &str,
            _mode: ActivationMode,
        ) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("deploy_start:{}", hostname));
            Ok(())
        }

        fn on_deploy_complete(
            &self,
            hostname: &str,
            result: &DeployResult,
        ) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "deploy_complete:{}:{}",
                hostname,
                if result.success { "ok" } else { "failed" }
            ));
            Ok(())
        }

        fn on_phase_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "phase_complete:{}:{}",
                phase,
                if success { "ok" } else { "failed" }
            ));
            Ok(())
        }

        fn on_reboot_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("reboot_start:{}", hostname));
            Ok(())
        }

        fn on_reboot_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "reboot_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }
    }

    #[test]
    fn test_push_with_single_phase() {
        let deployer = MockDeployer::new();
        let observer = TestObserver::new();

        let plans = vec![
            DeploymentPlan {
                metadata: HostMetadata {
                    name: "host1".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host1.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host1.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host1".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
            DeploymentPlan {
                metadata: HostMetadata {
                    name: "host2".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host2.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host2.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host2".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
        ];

        let results = activate_by_phase(&deployer, plans, 2, &observer).unwrap();

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);

        // Check observer events
        let events = observer.get_events();
        assert!(events.contains(&"phase_start:p1:2".to_string()));
        assert!(events.contains(&"deploy_start:host1".to_string()));
        assert!(events.contains(&"deploy_start:host2".to_string()));
        assert!(events.contains(&"deploy_complete:host1:ok".to_string()));
        assert!(events.contains(&"deploy_complete:host2:ok".to_string()));
        assert!(events.contains(&"phase_complete:p1:ok".to_string()));
    }

    #[test]
    fn test_push_with_multiple_phases() {
        let deployer = MockDeployer::new();
        let observer = TestObserver::new();

        let plans = vec![
            DeploymentPlan {
                metadata: HostMetadata {
                    name: "host3".to_string(),
                    phase: Some("p2".to_string()),
                    tags: vec![],
                    fqdn: "host3.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host3.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host3".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
            DeploymentPlan {
                metadata: HostMetadata {
                    name: "host1".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host1.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host1.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host1".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
        ];

        let results = activate_by_phase(&deployer, plans, 2, &observer).unwrap();

        assert_eq!(results.success_count(), 2);

        // Check that phases were executed in order
        let events = observer.get_events();
        let p1_start = events.iter().position(|e| e == "phase_start:p1:1").unwrap();
        let p2_start = events.iter().position(|e| e == "phase_start:p2:1").unwrap();

        // p1 should start before p2 (lexicographic order)
        assert!(p1_start < p2_start);
    }

    #[test]
    fn test_push_stops_on_phase_failure() {
        let deployer = MockDeployer::new().with_failures(vec!["host1.example.com".to_string()]);
        let observer = TestObserver::new();

        let plans = vec![
            DeploymentPlan {
                metadata: HostMetadata {
                    name: "host1".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host1.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host1.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host1".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
            DeploymentPlan {
                metadata: HostMetadata {
                    name: "host2".to_string(),
                    phase: Some("p2".to_string()),
                    tags: vec![],
                    fqdn: "host2.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host2.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host2".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
        ];

        let results = activate_by_phase(&deployer, plans, 2, &observer).unwrap();

        // Only p1 should have run (and failed), p2 should not have run
        assert_eq!(results.results.len(), 1);
        assert_eq!(results.success_count(), 0);
        assert_eq!(results.failure_count(), 1);

        // Check that p2 never started
        let events = observer.get_events();
        assert!(!events.iter().any(|e| e.contains("p2")));
        assert!(events.contains(&"phase_complete:p1:failed".to_string()));
    }

    #[test]
    fn test_push_with_noop_observer() {
        let deployer = MockDeployer::new();
        let observer = NoopObserver;

        let plans = vec![DeploymentPlan {
            metadata: HostMetadata {
                name: "host1".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                fqdn: "host1.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "host1.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-host1".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        }];

        let results = activate_by_phase(&deployer, plans, 1, &observer).unwrap();
        assert_eq!(results.success_count(), 1);
    }

    #[test]
    fn test_push_with_default_phase() {
        let deployer = MockDeployer::new();
        let observer = TestObserver::new();

        let plans = vec![DeploymentPlan {
            metadata: HostMetadata {
                name: "host1".to_string(),
                phase: None, // No phase specified
                tags: vec![],
                fqdn: "host1.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "host1.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-host1".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        }];

        let results = activate_by_phase(&deployer, plans, 1, &observer).unwrap();
        assert_eq!(results.success_count(), 1);

        // Should use "default" phase
        let events = observer.get_events();
        assert!(events.contains(&"phase_start:default:1".to_string()));
    }

    #[test]
    fn test_push_with_partial_failure_in_phase() {
        // Test that when one host fails in a phase, we still get results for all hosts in that phase
        let deployer = MockDeployer::new().with_failures(vec!["host2.example.com".to_string()]);
        let observer = TestObserver::new();

        let plans = vec![
            DeploymentPlan {
                metadata: HostMetadata {
                    name: "host1".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host1.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host1.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host1".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
            DeploymentPlan {
                metadata: HostMetadata {
                    name: "host2".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host2.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host2.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host2".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
        ];

        let results = activate_by_phase(&deployer, plans, 2, &observer).unwrap();

        // Both hosts should have results
        assert_eq!(results.results.len(), 2);
        assert_eq!(results.success_count(), 1);
        assert_eq!(results.failure_count(), 1);

        // Check that one succeeded and one failed
        let host_names: Vec<&str> = results.results.iter().map(|(n, _)| n.as_str()).collect();
        assert!(host_names.contains(&"host1"));
        assert!(host_names.contains(&"host2"));

        let host1_result = results.results.iter().find(|(n, _)| n == "host1").unwrap();
        let host2_result = results.results.iter().find(|(n, _)| n == "host2").unwrap();
        assert!(host1_result.1.success);
        assert!(!host2_result.1.success);

        // Phase should be marked as failed
        let events = observer.get_events();
        assert!(events.contains(&"phase_complete:p1:failed".to_string()));
    }

    #[test]
    fn test_push_error_message_includes_source() {
        // Test that when deployment fails, the error message includes the full error chain
        let deployer = MockDeployer::new().with_failures(vec!["fail.example.com".to_string()]);
        let observer = NoopObserver;

        let plans = vec![DeploymentPlan {
            metadata: HostMetadata {
                name: "fail".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                fqdn: "fail.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "fail.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-fail".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        }];

        let results = activate_by_phase(&deployer, plans, 1, &observer).unwrap();

        assert_eq!(results.failure_count(), 1);

        // Verify the error message includes the source error
        let (_, deploy_result) = &results.results[0];
        assert!(!deploy_result.success);
        assert!(
            deploy_result.message.contains("Caused by:"),
            "Error message should include 'Caused by:' to show the error chain. Got: {}",
            deploy_result.message
        );
        assert!(
            deploy_result.message.contains("copy_closure failed"),
            "Error message should include the source error details. Got: {}",
            deploy_result.message
        );
    }

    #[test]
    fn test_push_error_message_format_with_activation_failure() {
        // Test the specific format of error messages with activation failures
        use std::collections::HashMap;

        let mut failures = HashMap::new();
        failures.insert(
            "test.example.com".to_string(),
            crate::deploy::mock::FailureSpec {
                phase: "activate".to_string(),
                error: Some("systemd service failed to start".to_string()),
            },
        );

        let fixture = crate::deploy::mock::DeploymentFixture {
            name: "Test error formatting".to_string(),
            description: None,
            failures,
            hosts: vec![],
        };

        let deployer = MockDeployer::from_fixture(fixture);
        let observer = NoopObserver;

        let plans = vec![DeploymentPlan {
            metadata: HostMetadata {
                name: "test".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                fqdn: "test.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "test.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-test".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        }];

        let results = activate_by_phase(&deployer, plans, 1, &observer).unwrap();

        assert_eq!(results.failure_count(), 1);

        let (_, deploy_result) = &results.results[0];
        assert!(!deploy_result.success);

        // The error message should be structured as:
        // "Failed to deploy to host 'test.example.com'"
        //   Caused by: Activation failed
        assert!(
            deploy_result.message.contains("Failed to deploy"),
            "Error message should start with main error. Got: {}",
            deploy_result.message
        );
        assert!(
            deploy_result.message.contains("Caused by:"),
            "Error message should include 'Caused by:' separator. Got: {}",
            deploy_result.message
        );
        assert!(
            deploy_result.message.contains("Activation failed"),
            "Error message should include the source error. Got: {}",
            deploy_result.message
        );
    }

    // ============================================================================
    // Integration Tests for push() Function
    // ============================================================================

    #[test]
    fn test_push_full_workflow_single_phase_success() {
        // Test the complete workflow: build → copy → plan → activate for a single phase
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never, // No reboot needed
            },
            HostMetadata {
                name: "web2".to_string(),
                fqdn: "web2.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Verify both hosts succeeded
        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);

        // Verify observer events fired in correct order
        let events = observer.get_events();

        // Build phase events
        assert!(events.contains(&"build_phase_start:web:2".to_string()));
        assert!(events.contains(&"build_start:web1".to_string()));
        assert!(events.contains(&"build_complete:web1:ok".to_string()));
        assert!(events.contains(&"build_start:web2".to_string()));
        assert!(events.contains(&"build_complete:web2:ok".to_string()));

        // Copy phase events
        assert!(events.contains(&"copy_phase_start:web:2".to_string()));
        assert!(events.contains(&"copy_start:web1".to_string()));
        assert!(events.contains(&"copy_complete:web1:ok".to_string()));
        assert!(events.contains(&"copy_start:web2".to_string()));
        assert!(events.contains(&"copy_complete:web2:ok".to_string()));

        // Activation phase events
        assert!(events.contains(&"phase_start:web:2".to_string()));
        assert!(events.contains(&"deploy_start:web1".to_string()));
        assert!(events.contains(&"deploy_complete:web1:ok".to_string()));
        assert!(events.contains(&"deploy_start:web2".to_string()));
        assert!(events.contains(&"deploy_complete:web2:ok".to_string()));
        assert!(events.contains(&"phase_complete:web:ok".to_string()));
    }

    #[test]
    fn test_push_full_workflow_multi_phase_success() {
        // Test multiple phases deploying sequentially with all steps
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "db1".to_string(),
                fqdn: "db1.example.com".to_string(),
                phase: Some("database".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        assert_eq!(results.success_count(), 2);

        let events = observer.get_events();

        // Database phase should complete before web phase starts
        let db_phase_start = events
            .iter()
            .position(|e| e == "build_phase_start:database:1")
            .expect("database build should start");
        let db_phase_complete = events
            .iter()
            .position(|e| e == "phase_complete:database:ok")
            .expect("database activation should complete");
        let web_phase_start = events
            .iter()
            .position(|e| e == "build_phase_start:web:1")
            .expect("web build should start");

        assert!(db_phase_start < db_phase_complete);
        assert!(db_phase_complete < web_phase_start);
    }

    #[test]
    fn test_push_build_failure_stops_deployment() {
        // Build fails for one host, should stop immediately with no copy/activate
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new().with_build_failures(vec!["fail-host".to_string()]);
        let deployer = MockDeployer::new();
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![HostMetadata {
            name: "fail-host".to_string(),
            fqdn: "fail-host.example.com".to_string(),
            phase: None,
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        );

        // Should fail during build
        assert!(result.is_err());

        let events = observer.get_events();

        // Build events should be present
        assert!(events.contains(&"build_phase_start:default:1".to_string()));
        assert!(events.contains(&"build_start:fail-host".to_string()));
        assert!(events.contains(&"build_complete:fail-host:fail".to_string()));

        // NO copy/activate/reboot events should fire
        assert!(!events.iter().any(|e| e.starts_with("copy_")));
        assert!(!events.iter().any(|e| e.starts_with("phase_start:")));
        assert!(!events.iter().any(|e| e.starts_with("deploy_")));
        assert!(!events.iter().any(|e| e.starts_with("reboot_")));
    }

    #[test]
    fn test_push_copy_failure_stops_deployment() {
        // Copy fails for one host after successful build
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer =
            MockDeployer::new().with_copy_failures(vec!["fail-copy.example.com".to_string()]);
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![HostMetadata {
            name: "fail-copy".to_string(),
            fqdn: "fail-copy.example.com".to_string(),
            phase: None,
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        );

        // Should fail during copy
        assert!(result.is_err());

        let events = observer.get_events();

        // Build should succeed
        assert!(events.contains(&"build_complete:fail-copy:ok".to_string()));

        // Copy should fail
        assert!(events.contains(&"copy_start:fail-copy".to_string()));
        assert!(events.contains(&"copy_complete:fail-copy:fail".to_string()));

        // NO activate/reboot events
        assert!(!events.iter().any(|e| e.starts_with("phase_start:")));
        assert!(!events.iter().any(|e| e.starts_with("deploy_")));
        assert!(!events.iter().any(|e| e.starts_with("reboot_")));
    }

    #[test]
    fn test_push_copy_observer_hooks_called() {
        // Verify copy observer hooks fire in correct order
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        let events = observer.get_events();

        // Verify copy sequence: phase_start → copy_start → copy_complete
        let copy_phase_idx = events
            .iter()
            .position(|e| e == "copy_phase_start:p1:2")
            .unwrap();
        let copy1_start_idx = events.iter().position(|e| e == "copy_start:host1").unwrap();
        let copy1_complete_idx = events
            .iter()
            .position(|e| e == "copy_complete:host1:ok")
            .unwrap();
        let copy2_start_idx = events.iter().position(|e| e == "copy_start:host2").unwrap();
        let copy2_complete_idx = events
            .iter()
            .position(|e| e == "copy_complete:host2:ok")
            .unwrap();

        assert!(copy_phase_idx < copy1_start_idx);
        assert!(copy1_start_idx < copy1_complete_idx);
        assert!(copy_phase_idx < copy2_start_idx);
        assert!(copy2_start_idx < copy2_complete_idx);
    }

    #[test]
    fn test_push_build_observer_hooks_called() {
        // Verify build observer hooks fire in correct order
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        let events = observer.get_events();

        // Verify build sequence: phase_start → build_start → build_complete
        let build_phase_idx = events
            .iter()
            .position(|e| e == "build_phase_start:p1:2")
            .unwrap();
        let build1_start_idx = events
            .iter()
            .position(|e| e == "build_start:host1")
            .unwrap();
        let build1_complete_idx = events
            .iter()
            .position(|e| e == "build_complete:host1:ok")
            .unwrap();
        let build2_start_idx = events
            .iter()
            .position(|e| e == "build_start:host2")
            .unwrap();
        let build2_complete_idx = events
            .iter()
            .position(|e| e == "build_complete:host2:ok")
            .unwrap();

        assert!(build_phase_idx < build1_start_idx);
        assert!(build1_start_idx < build1_complete_idx);
        assert!(build_phase_idx < build2_start_idx);
        assert!(build2_start_idx < build2_complete_idx);
    }

    #[test]
    fn test_push_build_partial_failure() {
        // Build fails for host 2 of 2 in a phase
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new().with_build_failures(vec!["host2".to_string()]);
        let deployer = MockDeployer::new();
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        );

        assert!(result.is_err());

        let events = observer.get_events();

        // host1 build should succeed
        assert!(events.contains(&"build_complete:host1:ok".to_string()));

        // host2 build should fail
        assert!(events.contains(&"build_complete:host2:fail".to_string()));

        // No copy should happen for either host
        assert!(!events.iter().any(|e| e.starts_with("copy_")));
    }

    #[test]
    fn test_push_ignore_phases_flag() {
        // Test that ignore_phases groups all hosts into "_all" phase
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "db1".to_string(),
                fqdn: "db1.example.com".to_string(),
                phase: Some("database".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: true, // Ignore phase ordering
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        assert_eq!(results.success_count(), 2);

        let events = observer.get_events();

        // Both hosts should be in "_all" phase
        assert!(events.contains(&"build_phase_start:_all:2".to_string()));
        assert!(events.contains(&"copy_phase_start:_all:2".to_string()));
        assert!(events.contains(&"phase_start:_all:2".to_string()));

        // Should NOT have separate database/web phases
        assert!(!events.iter().any(|e| e.contains(":database:")));
        assert!(!events.iter().any(|e| e.contains(":web:")));
    }

    #[test]
    fn test_push_phase_failure_stops_next_phase() {
        // Phase 1 activation fails, Phase 2 should never start
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer =
            MockDeployer::new().with_activate_failures(vec!["phase1-host.example.com".to_string()]);
        let reboot_detector = RebootDetector::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "phase1-host".to_string(),
                fqdn: "phase1-host.example.com".to_string(),
                phase: Some("phase1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "phase2-host".to_string(),
                fqdn: "phase2-host.example.com".to_string(),
                phase: Some("phase2".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_detector,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Phase1 should fail, Phase2 should not run
        assert_eq!(results.success_count(), 0);
        assert_eq!(results.failure_count(), 1);

        let events = observer.get_events();

        // Phase1 should have all events
        assert!(events.iter().any(|e| e.contains("phase1")));

        // Phase2 should have NO events (build, copy, or activate)
        assert!(!events.iter().any(|e| e.contains("phase2")));
    }

    #[test]
    fn test_push_reboot_execution_success() {
        // Test that reboot is executed when required and observer hooks are called
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();

        // Configure reboot detection to require reboot
        use crate::deploy::mock::MockRebootOps;
        use crate::deploy::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "reboot-host.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_ops = MockRebootOps::new().with_reboot_required(reboot_required.clone());

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "reboot-host",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto, // Auto mode will detect reboot requirement
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_ops,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 1);
        assert_eq!(results.failure_count(), 0);

        // Check observer events - should include reboot events
        let events = observer.get_events();

        // Should have reboot events
        assert!(
            events.iter().any(|e| e == "reboot_start:reboot-host"),
            "Missing reboot_start event. Events: {:?}",
            events
        );
        assert!(
            events.iter().any(|e| e == "reboot_complete:reboot-host:ok"),
            "Missing reboot_complete event. Events: {:?}",
            events
        );

        // Verify event sequence: build -> copy -> deploy -> reboot
        let reboot_start_idx = events
            .iter()
            .position(|e| e == "reboot_start:reboot-host")
            .unwrap();
        let deploy_complete_idx = events
            .iter()
            .position(|e| e.starts_with("deploy_complete:reboot-host"))
            .unwrap();

        assert!(
            deploy_complete_idx < reboot_start_idx,
            "Reboot should happen after deployment"
        );
    }

    #[test]
    fn test_push_reboot_execution_failure() {
        // Test that reboot failures are properly handled and reported
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();

        // Configure reboot to be required but fail during execution
        use crate::deploy::mock::MockRebootOps;
        use crate::deploy::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "fail-reboot.example.com".to_string(),
            RebootReason::InitrdChanged,
        );
        let reboot_ops = MockRebootOps::new()
            .with_reboot_required(reboot_required)
            .with_reboot_failures(vec!["fail-reboot.example.com".to_string()]);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "fail-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_ops,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Should report failure due to reboot failure
        assert!(!results.all_succeeded());
        assert_eq!(results.failure_count(), 1);

        // Check that deployment itself succeeded but reboot failed
        let (hostname, result) = &results.results[0];
        assert_eq!(hostname, "fail-reboot");
        assert!(!result.success);
        assert!(
            result.message.contains("reboot failed"),
            "Error should mention reboot failure: {}",
            result.message
        );

        // Check observer events
        let events = observer.get_events();

        // Should have both reboot_start and reboot_complete:fail
        assert!(
            events.iter().any(|e| e == "reboot_start:fail-reboot"),
            "Missing reboot_start event"
        );
        assert!(
            events
                .iter()
                .any(|e| e == "reboot_complete:fail-reboot:fail"),
            "Missing reboot_complete:fail event"
        );
    }

    #[test]
    fn test_push_no_reboot_when_not_required() {
        // Test that reboot hooks are NOT called when reboot is not required
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();

        // No reboot required (empty HashMap)
        use crate::deploy::mock::MockRebootOps;
        let reboot_ops = MockRebootOps::new();

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "no-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_ops,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 1);

        // Check observer events - should NOT include reboot events
        let events = observer.get_events();

        // Should NOT have reboot events
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "Should not have reboot_start when reboot not required"
        );
        assert!(
            !events.iter().any(|e| e.contains("reboot_complete")),
            "Should not have reboot_complete when reboot not required"
        );

        // Should have completed deployment
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_reboot_with_boot_mode() {
        // Test explicit Boot mode with reboot required
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();

        // Configure reboot required
        use crate::deploy::mock::MockRebootOps;
        use crate::deploy::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "boot-host.example.com".to_string(),
            RebootReason::InitChanged,
        );
        let reboot_ops = MockRebootOps::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "boot-host",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Boot, // Explicit Boot mode
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_ops,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was executed
        let events = observer.get_events();
        assert!(
            events.iter().any(|e| e.starts_with("reboot_start")),
            "Boot mode with reboot required should execute reboot"
        );
        assert!(events.iter().any(|e| e.starts_with("reboot_complete")));
    }

    #[test]
    fn test_push_reboot_policy_always() {
        // Test that RebootPolicy::Always forces reboot even if not detected
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();

        // No reboot detected, but policy is Always
        use crate::deploy::mock::MockRebootOps;
        let reboot_ops = MockRebootOps::new(); // Empty - no reboot required

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "always-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Always, // Always reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_ops,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was executed despite no detection
        let events = observer.get_events();
        assert!(
            events.iter().any(|e| e == "reboot_start:always-reboot"),
            "RebootPolicy::Always should execute reboot. Events: {:?}",
            events
        );
        assert!(events
            .iter()
            .any(|e| e == "reboot_complete:always-reboot:ok"));
    }

    #[test]
    fn test_push_reboot_policy_never() {
        // Test that RebootPolicy::Never prevents reboot even if detected
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();

        // Reboot required but policy is Never
        use crate::deploy::mock::MockRebootOps;
        use crate::deploy::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "never-reboot.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_ops = MockRebootOps::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "never-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Never, // Never reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_ops,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite detection
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "RebootPolicy::Never should prevent reboot"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_no_reboot_flag_overrides_always_policy() {
        // Test that allow_reboot=false prevents reboot even with RebootPolicy::Always
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();

        // Policy is Always, but user says --no-reboot
        use crate::deploy::mock::MockRebootOps;
        let reboot_ops = MockRebootOps::new();

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "always-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Always, // Policy says always reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: false, // User says --no-reboot
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_ops,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite policy
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "allow_reboot=false should prevent reboot even with RebootPolicy::Always"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_no_reboot_flag_with_detected_reboot() {
        // Test that allow_reboot=false prevents reboot even when detection says it's needed
        let ctx = SnowContext::test_context();
        let nix_ops = MockNixOps::new();
        let deployer = MockDeployer::new();

        // Reboot detected as required but user says --no-reboot
        use crate::deploy::mock::MockRebootOps;
        use crate::deploy::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "needs-reboot.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_ops = MockRebootOps::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "needs-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto, // Auto policy
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: false, // User says --no-reboot
        };

        let results = push(
            &ctx,
            &nix_ops,
            &deployer,
            &reboot_ops,
            hosts,
            &config,
            &observer,
        )
        .unwrap();

        // Should succeed (uses Boot mode but doesn't reboot)
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite detection
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "allow_reboot=false should prevent reboot even when detection says reboot needed"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully (Boot mode without reboot)
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }
}
