use clap::Args;
use snowlib::{
    create_deployment_plans, push_parallel, ActivationMode, BlockedHost, DeploymentMode,
    DeploymentPlan, HostMetadata, PushOptions, PushResults, RebootDetector, Result, SnowContext,
    SshDeployer,
};
use std::collections::HashMap;
use tabled::{Table, Tabled};

// Used in tests
#[allow(unused_imports)]
use snowlib::{DeployTarget, StorePath};

#[cfg(debug_assertions)]
use snowlib::MockDeployer;

/// Deploy configurations to hosts
#[derive(Debug, Args)]
pub struct PushArgs {
    /// Hosts to push (defaults to all hosts if not specified)
    pub hosts: Vec<String>,

    /// Dry run - show what would change without activating
    #[arg(long)]
    pub dry_run: bool,

    /// Force immediate activation (switch mode)
    #[arg(long)]
    pub switch: bool,

    /// Only set for next boot (don't activate now)
    #[arg(long)]
    pub boot: bool,

    /// Test activation without making permanent
    #[arg(long)]
    pub test: bool,

    /// Automatically reboot hosts after boot-mode deployment (enabled by default)
    #[arg(long, default_value = "true")]
    pub reboot: bool,

    /// Never automatically reboot (even if reboot is required)
    #[arg(long)]
    pub no_reboot: bool,

    /// Force operation despite safety checks (use with --switch to override reboot detection)
    #[arg(long)]
    pub force: bool,

    /// Quiet mode - suppress subprocess output, only show summary
    #[arg(short, long)]
    pub quiet: bool,

    /// SSH user to use for deployment (defaults to SSH config or current user)
    #[arg(long)]
    pub user: Option<String>,

    /// Maximum number of hosts to push in parallel (must be at least 1)
    #[arg(long, default_value = "3", value_parser = clap::value_parser!(u32).range(1..))]
    pub concurrency: u32,

    #[cfg(debug_assertions)]
    /// Path to fixture file for mock deployer (debug builds only)
    #[arg(long, hide = true)]
    pub mock_fixture: Option<String>,
}

#[derive(Debug, Tabled)]
struct DeploymentResult {
    #[tabled(rename = "Host")]
    host: String,

    #[tabled(rename = "Status")]
    status: String,

    #[tabled(rename = "Reboot")]
    reboot_required: String,

    #[tabled(rename = "Message")]
    message: String,
}

impl PushArgs {
    /// Validate flag combinations
    fn validate(&self) -> Result<()> {
        // Check for mutually exclusive activation modes
        let mode_count = [self.dry_run, self.switch, self.boot, self.test]
            .iter()
            .filter(|&&x| x)
            .count();

        if mode_count > 1 {
            return Err(snowlib::error::DeployError::Failed {
                host: "validation".to_string(),
                source: std::io::Error::other(
                    "Cannot use multiple activation modes (--dry-run, --switch, --boot, --test)",
                ),
            }
            .into());
        }

        // Check for conflicting reboot flags
        if self.reboot && self.no_reboot {
            return Err(snowlib::error::DeployError::Failed {
                host: "validation".to_string(),
                source: std::io::Error::other("Cannot use both --reboot and --no-reboot"),
            }
            .into());
        }

        // --force only makes sense with --switch
        if self.force && !self.switch {
            return Err(snowlib::error::DeployError::Failed {
                host: "validation".to_string(),
                source: std::io::Error::other("--force can only be used with --switch"),
            }
            .into());
        }

        Ok(())
    }

    /// Determine if automatic reboot should happen
    fn should_auto_reboot(&self) -> bool {
        // Never auto-reboot if --no-reboot is set
        if self.no_reboot {
            return false;
        }

        // Auto-reboot is the default (--reboot defaults to true)
        self.reboot
    }

    /// Determine the deployment mode from flags
    fn deployment_mode(&self) -> DeploymentMode {
        if self.dry_run {
            DeploymentMode::DryRun
        } else if self.test {
            DeploymentMode::Test
        } else if self.boot {
            DeploymentMode::Boot
        } else if self.switch {
            DeploymentMode::Switch
        } else {
            DeploymentMode::Auto
        }
    }

    /// Print error message for hosts blocked by --switch safety check
    fn print_switch_blocked_error(blocked_hosts: &[BlockedHost]) {
        eprintln!("\nError: Cannot use --switch when reboot is required\n");

        for host in blocked_hosts {
            eprintln!(
                "  {}: reboot required because {}",
                host.hostname, host.reason
            );
        }

        eprintln!(
            "\nThe --switch flag forces immediate activation, which can leave the system in an"
        );
        eprintln!("inconsistent state when kernel/initrd/init changes occur or critical services restart.\n");
        eprintln!("Options:");
        eprintln!("  1. Remove --switch to use automatic mode (will use boot mode + reboot)");
        eprintln!(
            "  2. Use --switch --force to force immediate activation anyway (not recommended)"
        );
        eprintln!("  3. Use --boot to set for next boot without immediate activation");
    }

    /// Resolve and validate the hosts to push
    ///
    /// If no hosts are provided, returns all hosts from the flake.
    /// If hosts are provided, validates they exist in the flake.
    fn resolve_hosts(&self, ctx: &SnowContext) -> Result<Vec<String>> {
        let all_hosts = snowlib::list_hosts(ctx, None)?;

        if self.hosts.is_empty() {
            // No hosts specified, push all hosts
            Ok(all_hosts.iter().map(|h| h.name.clone()).collect())
        } else {
            // Validate that all provided hosts exist in the flake
            let valid_hosts: std::collections::HashSet<String> =
                all_hosts.iter().map(|h| h.name.clone()).collect();

            let invalid_hosts: Vec<String> = self
                .hosts
                .iter()
                .filter(|h| !valid_hosts.contains(*h))
                .cloned()
                .collect();

            if !invalid_hosts.is_empty() {
                return Err(snowlib::error::FlakeError::InvalidHosts {
                    invalid_hosts: invalid_hosts.join(", "),
                    available_hosts: valid_hosts.iter().cloned().collect::<Vec<_>>().join(", "),
                }
                .into());
            }

            Ok(self.hosts.clone())
        }
    }

    /// Create the appropriate NixOps implementation based on settings
    fn get_nix_ops(&self) -> Result<Box<dyn snowlib::NixOps>> {
        #[cfg(debug_assertions)]
        {
            if let Some(fixture_path) = &self.mock_fixture {
                println!("⚠ Using mock deployer");
                let path = std::path::Path::new(fixture_path);
                let fixture = snowlib::DeploymentFixture::from_file(path)?;
                return Ok(Box::new(snowlib::MockNixOps::from_fixture(fixture)));
            }
        }

        Ok(Box::new(snowlib::SystemNixOps::new()))
    }

    /// Create the appropriate Deployer implementation based on settings
    fn get_deployer(&self) -> Result<Box<dyn snowlib::Deployer>> {
        #[cfg(debug_assertions)]
        {
            if let Some(fixture_path) = &self.mock_fixture {
                let path = std::path::Path::new(fixture_path);
                return Ok(Box::new(MockDeployer::from_file(path)?));
            }
        }

        Ok(Box::new(SshDeployer::new().with_quiet(self.quiet)))
    }

    /// Build NixOS configurations for all specified hosts
    fn build_configurations(
        &self,
        ctx: &SnowContext,
        nix_ops: &dyn snowlib::NixOps,
        hosts: &[String],
    ) -> Result<Vec<(HostMetadata, DeployTarget, StorePath)>> {
        println!("→ Building configurations...");

        let mut host_data = Vec::new();
        for host in hosts {
            let metadata = nix_ops.get_host_metadata(ctx, host)?;
            let store_path = nix_ops.build_host(ctx, host, self.quiet)?;

            let target = DeployTarget {
                user: self.user.clone(),
                hostname: metadata.fqdn.clone(),
                port: None,
            };

            host_data.push((metadata, target, store_path));
        }

        println!("✓ Built {} configuration(s)", hosts.len());
        Ok(host_data)
    }

    /// Copy closures to remote hosts
    ///
    /// This MUST happen before reboot detection because the new system paths
    /// must exist on the remote for symlink comparison to work.
    fn copy_closures(
        &self,
        deployer: &dyn snowlib::Deployer,
        host_data: &[(HostMetadata, DeployTarget, StorePath)],
    ) -> Result<()> {
        println!("\n→ Copying closures...");

        for (metadata, target, store_path) in host_data {
            if !self.quiet {
                println!("  Copying to {}...", metadata.name);
            }
            deployer.copy_closure(target, store_path)?;
        }

        if !self.quiet {
            println!("✓ Copied to {} host(s)", host_data.len());
        }

        Ok(())
    }

    /// Create deployment plans with reboot detection and safety checks
    ///
    /// Returns an error if hosts are blocked by safety checks (e.g., --switch without --force)
    fn create_deployment_plans_with_detection(
        &self,
        host_data: Vec<(HostMetadata, DeployTarget, StorePath)>,
    ) -> Result<Vec<DeploymentPlan>> {
        let mut reboot_detector = RebootDetector::default();
        if let Some(ref user) = self.user {
            reboot_detector = reboot_detector.with_ssh_user(user);
        }

        let mode = self.deployment_mode();
        let (deployment_plans, blocked_hosts) =
            create_deployment_plans(host_data, &reboot_detector, mode, self.force)?;

        // Handle blocked hosts (--switch safety check)
        if !blocked_hosts.is_empty() {
            Self::print_switch_blocked_error(&blocked_hosts);
            return Err(snowlib::error::DeployError::Failed {
                host: "multiple".to_string(),
                source: std::io::Error::other("Switch mode blocked by reboot requirement"),
            }
            .into());
        }

        // Show warnings for --switch --force (risky operation)
        if self.switch && self.force {
            for plan in &deployment_plans {
                if plan
                    .reboot_check
                    .as_ref()
                    .map(|c| c.required)
                    .unwrap_or(false)
                {
                    eprintln!(
                        "\n⚠  Warning: Forcing immediate activation on {} despite reboot requirement ({})",
                        plan.metadata.name,
                        plan.reboot_check.as_ref().unwrap().reason.as_ref().unwrap()
                    );
                    eprintln!("   This may leave the system in an inconsistent state.\n");
                }
            }
        }

        Ok(deployment_plans)
    }

    /// Execute deployments grouped by activation mode
    fn execute_deployments(
        &self,
        deployer: &dyn snowlib::Deployer,
        deployment_plans: &[DeploymentPlan],
    ) -> Result<PushResults> {
        println!("\n→ Deploying...");

        // Group deployment plans by activation mode (some hosts may need boot, others switch)
        let mut by_mode: HashMap<ActivationMode, Vec<usize>> = HashMap::new();
        for (idx, plan) in deployment_plans.iter().enumerate() {
            by_mode.entry(plan.activation_mode).or_default().push(idx);
        }

        // Deploy each group with its respective activation mode
        let mut all_results = Vec::new();
        for (mode, indices) in by_mode {
            let host_data_for_deploy: Vec<_> = indices
                .iter()
                .map(|&idx| {
                    let plan = &deployment_plans[idx];
                    (
                        plan.metadata.clone(),
                        plan.target.clone(),
                        plan.store_path.clone(),
                    )
                })
                .collect();

            let opts = PushOptions {
                activation_mode: mode,
                concurrency: self.concurrency as usize,
            };

            let results = push_parallel(deployer, host_data_for_deploy, &opts)?;
            all_results.extend(results.results);
        }

        Ok(PushResults {
            results: all_results,
        })
    }

    /// Handle automatic reboots for hosts that need them
    ///
    /// Reboots hosts that:
    /// - Deployed successfully
    /// - Used boot activation mode
    /// - Require reboot (as determined by deployment planning, which respects reboot policies)
    fn handle_auto_reboots(
        &self,
        deployment_plans: &[DeploymentPlan],
        results: &mut PushResults,
    ) -> Result<()> {
        if !self.should_auto_reboot() {
            return Ok(());
        }

        let mut reboot_detector = RebootDetector::default();
        if let Some(ref user) = self.user {
            reboot_detector = reboot_detector.with_ssh_user(user);
        }

        let hosts_to_reboot: Vec<_> = deployment_plans
            .iter()
            .filter(|plan| {
                let deployed_successfully = results
                    .results
                    .iter()
                    .any(|(h, r)| h == &plan.metadata.name && r.success);

                deployed_successfully
                    && plan.activation_mode == ActivationMode::Boot
                    && plan
                        .reboot_check
                        .as_ref()
                        .map(|c| c.required)
                        .unwrap_or(false)
            })
            .collect();

        if hosts_to_reboot.is_empty() {
            return Ok(());
        }

        println!("\n→ Rebooting hosts...\n");

        for plan in hosts_to_reboot {
            print!("  Rebooting {}...", plan.metadata.name);
            std::io::Write::flush(&mut std::io::stdout()).ok();

            match reboot_detector.reboot_and_wait(&plan.target) {
                Ok(_) => {
                    println!(" ✓ Back online");

                    // Update the result message
                    if let Some((_, result)) = results
                        .results
                        .iter_mut()
                        .find(|(h, _)| h == &plan.metadata.name)
                    {
                        result.message = "Deployed & Rebooted".to_string();
                    }
                }
                Err(e) => {
                    println!(" ✗ Failed to reboot: {}", e);

                    // Update the result to show reboot failure
                    if let Some((_, result)) = results
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

        Ok(())
    }

    /// Format reboot status for display
    fn format_reboot_status(plan: &DeploymentPlan) -> String {
        match &plan.reboot_check {
            Some(check) if check.required => {
                format!(
                    "Yes ({})",
                    check
                        .reason
                        .as_ref()
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                )
            }
            Some(_) => "No".to_string(),
            None => "N/A".to_string(),
        }
    }

    /// Print deployment results table and warnings
    fn print_results(
        &self,
        deployment_plans: &[DeploymentPlan],
        results: &PushResults,
    ) -> Result<()> {
        // Match results with deployment plans by hostname
        let plan_by_host: HashMap<String, &DeploymentPlan> = deployment_plans
            .iter()
            .map(|plan| (plan.metadata.name.clone(), plan))
            .collect();

        let table_data: Vec<DeploymentResult> = results
            .results
            .iter()
            .map(|(host, result)| {
                let reboot_required = plan_by_host
                    .get(host)
                    .map(|plan| Self::format_reboot_status(plan))
                    .unwrap_or_else(|| "N/A".to_string());

                DeploymentResult {
                    host: host.clone(),
                    status: if result.success { "✓" } else { "✗" }.to_string(),
                    reboot_required,
                    message: result.message.clone(),
                }
            })
            .collect();

        println!("\nPush Results\n");
        let table = Table::new(&table_data).to_string();
        println!("{}", table);

        // Summary
        println!(
            "\n{}/{} hosts deployed successfully",
            results.success_count(),
            results.results.len()
        );

        // Show warnings
        self.print_manual_reboot_warning(deployment_plans, results);
        self.print_forced_switch_warning(deployment_plans, results);

        Ok(())
    }

    /// Print warning about hosts that need manual reboot
    fn print_manual_reboot_warning(
        &self,
        deployment_plans: &[DeploymentPlan],
        results: &PushResults,
    ) {
        if self.should_auto_reboot() {
            return;
        }

        let manual_reboot_hosts: Vec<_> = deployment_plans
            .iter()
            .filter(|plan| {
                plan.reboot_check
                    .as_ref()
                    .map(|c| c.required)
                    .unwrap_or(false)
                    && results
                        .results
                        .iter()
                        .any(|(h, r)| h == &plan.metadata.name && r.success)
            })
            .collect();

        if !manual_reboot_hosts.is_empty() {
            println!("\n⚠  The following hosts require a manual reboot:");
            for plan in manual_reboot_hosts {
                let reason = plan
                    .reboot_check
                    .as_ref()
                    .and_then(|c| c.reason.as_ref())
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                println!("  - {} ({})", plan.metadata.name, reason);
            }
        }
    }

    /// Print warning about hosts that were force-switched despite reboot requirement
    fn print_forced_switch_warning(
        &self,
        deployment_plans: &[DeploymentPlan],
        results: &PushResults,
    ) {
        if !self.switch || !self.force {
            return;
        }

        let forced_hosts: Vec<_> = deployment_plans
            .iter()
            .filter(|plan| {
                plan.reboot_check
                    .as_ref()
                    .map(|c| c.required)
                    .unwrap_or(false)
                    && results
                        .results
                        .iter()
                        .any(|(h, r)| h == &plan.metadata.name && r.success)
            })
            .collect();

        if !forced_hosts.is_empty() {
            println!("\n⚠  Warning: System may be in an inconsistent state");
            println!("   - New configuration active but old kernel/services still running");
            println!("   - Consider rebooting soon:");
            for plan in forced_hosts {
                println!("     - {}", plan.metadata.name);
            }
        }
    }

    /// Execute the push command
    ///
    /// # Deployment Flow
    ///
    /// The deployment process follows these steps in order:
    ///
    /// 1. **Build**: Build all NixOS configurations locally
    /// 2. **Copy**: Copy closures to remote hosts (paths must exist for reboot detection)
    /// 3. **Detect**: Run reboot detection to determine activation modes
    /// 4. **Plan**: Create deployment plans with appropriate activation modes
    /// 5. **Activate**: Activate configurations on remote hosts
    /// 6. **Reboot**: Automatically reboot hosts if needed (unless --no-reboot)
    ///
    /// # Critical Ordering
    ///
    /// The copy step MUST happen before reboot detection because:
    /// - Reboot detection checks symlinks like `/run/booted-system/kernel`
    /// - These symlinks are compared against paths in the new system closure
    /// - Those paths must exist on the remote for the comparison to work
    pub fn run(self, ctx: &SnowContext) -> Result<()> {
        // Validate flag combinations
        self.validate()?;

        // When using --mock-fixture, use the mock flake instead of the real one
        #[cfg(debug_assertions)]
        let mock_ctx;
        #[cfg(debug_assertions)]
        let ctx = if self.mock_fixture.is_some() {
            // Use the mock flake bundled with the test fixtures
            let mock_flake_path =
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
            mock_ctx = SnowContext::new(Some(mock_flake_path.to_str().unwrap()))?;
            &mock_ctx
        } else {
            ctx
        };

        let nix_ops = self.get_nix_ops()?;
        let hosts = self.resolve_hosts(ctx)?;

        // === STEP 1: BUILD CONFIGURATIONS ===
        let host_data = self.build_configurations(ctx, nix_ops.as_ref(), &hosts)?;

        let deployer = self.get_deployer()?;

        // === STEP 2: COPY CLOSURES ===
        self.copy_closures(deployer.as_ref(), &host_data)?;

        // === STEP 3: CREATE DEPLOYMENT PLANS WITH REBOOT DETECTION ===
        let deployment_plans = self.create_deployment_plans_with_detection(host_data)?;

        // === STEP 4: EXECUTE DEPLOYMENTS ===
        let mut results = self.execute_deployments(deployer.as_ref(), &deployment_plans)?;

        // === STEP 5: AUTOMATIC REBOOTS ===
        self.handle_auto_reboots(&deployment_plans, &mut results)?;

        // === STEP 6: PRINT RESULTS ===
        self.print_results(&deployment_plans, &results)?;

        if !results.all_succeeded() {
            return Err(snowlib::error::DeployError::Failed {
                host: "multiple".to_string(),
                source: std::io::Error::other("Some deployments failed"),
            }
            .into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowlib::{DeployTarget, StorePath};
    use snowlib::{Deployer, DeploymentFixture, HostFixture, MockDeployer, OperationFixture};

    #[test]
    fn test_mock_deployer_success() {
        let deployer = MockDeployer::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "test.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .is_ok());
    }

    #[test]
    fn test_mock_deployer_with_failure() {
        let deployer = MockDeployer::new().with_failures(vec!["fail.example.com".to_string()]);

        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "fail.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_err());
    }

    #[test]
    fn test_mock_deployer_from_fixture() {
        let fixture = DeploymentFixture {
            name: "Test deployment".to_string(),
            description: Some("Test scenario".to_string()),
            failures: std::collections::HashMap::new(),
            hosts: vec![HostFixture {
                hostname: "hydrogen.example.com".to_string(),
                metadata: None,
                build: None,
                copy_closure: OperationFixture {
                    success: true,
                    stdout: "copying...\n".to_string(),
                    stderr: "".to_string(),
                    duration_ms: 10,
                },
                activate: OperationFixture {
                    success: true,
                    stdout: "activating...\n".to_string(),
                    stderr: "".to_string(),
                    duration_ms: 5,
                },
            }],
        };

        let deployer = MockDeployer::from_fixture(fixture);
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .is_ok());
    }

    #[test]
    fn test_mock_deployer_from_file() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/push_scenarios/success.json");

        let deployer = MockDeployer::from_file(&fixture_path).unwrap();

        // Test with hydrogen
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .is_ok());
    }

    #[test]
    fn test_mock_deployer_partial_failure() {
        use std::path::PathBuf;

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/push_scenarios/partial_failure.json");

        let deployer = MockDeployer::from_file(&fixture_path).unwrap();

        // Hydrogen should succeed
        let target_hydrogen = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target_hydrogen, &store_path).is_ok());
        assert!(deployer
            .activate(&target_hydrogen, &store_path, ActivationMode::Switch)
            .is_ok());

        // Nitrogen should fail on activation
        let target_nitrogen = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nitrogen.example.com".to_string(),
            port: None,
        };

        assert!(deployer.copy_closure(&target_nitrogen, &store_path).is_ok());
        assert!(deployer
            .activate(&target_nitrogen, &store_path, ActivationMode::Switch)
            .is_err());
    }

    // Tests for PushArgs methods
    #[test]
    fn test_validate_mutually_exclusive_modes() {
        let args = PushArgs {
            hosts: vec![],
            dry_run: true,
            switch: true,
            boot: false,
            test: false,
            reboot: true,
            no_reboot: false,
            force: false,
            quiet: false,
            user: None,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_conflicting_reboot_flags() {
        let args = PushArgs {
            hosts: vec![],
            dry_run: false,
            switch: false,
            boot: false,
            test: false,
            reboot: true,
            no_reboot: true,
            force: false,
            quiet: false,
            user: None,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_force_without_switch() {
        let args = PushArgs {
            hosts: vec![],
            dry_run: false,
            switch: false,
            boot: false,
            test: false,
            reboot: true,
            no_reboot: false,
            force: true,
            quiet: false,
            user: None,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_valid_combination() {
        let args = PushArgs {
            hosts: vec![],
            dry_run: false,
            switch: false,
            boot: false,
            test: false,
            reboot: true,
            no_reboot: false,
            force: false,
            quiet: false,
            user: None,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_should_auto_reboot_default() {
        let args = PushArgs {
            hosts: vec![],
            dry_run: false,
            switch: false,
            boot: false,
            test: false,
            reboot: true,
            no_reboot: false,
            force: false,
            quiet: false,
            user: None,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert!(args.should_auto_reboot());
    }

    #[test]
    fn test_should_auto_reboot_with_no_reboot_flag() {
        let args = PushArgs {
            hosts: vec![],
            dry_run: false,
            switch: false,
            boot: false,
            test: false,
            reboot: true,
            no_reboot: true,
            force: false,
            quiet: false,
            user: None,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert!(!args.should_auto_reboot());
    }

    #[test]
    fn test_deployment_mode_auto() {
        let args = PushArgs {
            hosts: vec![],
            dry_run: false,
            switch: false,
            boot: false,
            test: false,
            reboot: true,
            no_reboot: false,
            force: false,
            quiet: false,
            user: None,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.deployment_mode(), DeploymentMode::Auto);
    }

    #[test]
    fn test_deployment_mode_switch() {
        let args = PushArgs {
            hosts: vec![],
            dry_run: false,
            switch: true,
            boot: false,
            test: false,
            reboot: true,
            no_reboot: false,
            force: false,
            quiet: false,
            user: None,
            concurrency: 3,
            #[cfg(debug_assertions)]
            mock_fixture: None,
        };

        assert_eq!(args.deployment_mode(), DeploymentMode::Switch);
    }
}
