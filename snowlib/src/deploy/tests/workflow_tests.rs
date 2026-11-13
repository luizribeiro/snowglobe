//! Full workflow integration tests for deploy()

use super::*;
use crate::backends::reboot::SystemRebootBackend;

// ============================================================================
// Integration Tests for deploy() Function
// ============================================================================

#[tokio::test]
async fn test_deploy_full_workflow_single_phase_success() {
    // Test the complete workflow: build → copy → plan → activate for a single phase
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![
        make_host_metadata("web1", Some("web"), RebootPolicy::Never),
        make_host_metadata("web2", Some("web"), RebootPolicy::Never),
    ];

    let config = create_deploy_config(DeployStrategy::Switch, 2);

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
    .unwrap();

    // Verify both hosts succeeded
    assert_eq!(results.success_count(), 2);
    assert_eq!(results.failure_count(), 0);

    // Verify observer events fired in correct order (parallel-per-host model)
    let events = observer.get_events();

    // Build events (per-host)
    assert!(events.contains(&"build_start:web1".to_string()));
    assert!(events.contains(&"build_complete:web1:ok".to_string()));
    assert!(events.contains(&"build_start:web2".to_string()));
    assert!(events.contains(&"build_complete:web2:ok".to_string()));

    // Copy events (per-host)
    assert!(events.contains(&"copy_start:web1".to_string()));
    assert!(events.contains(&"copy_complete:web1:ok".to_string()));
    assert!(events.contains(&"copy_start:web2".to_string()));
    assert!(events.contains(&"copy_complete:web2:ok".to_string()));

    // Activation phase events
    assert!(events.contains(&"phase_start:web:2".to_string()));
    assert!(events.contains(&"activation_start:web1".to_string()));
    assert!(events.contains(&"activation_complete:web1:ok".to_string()));
    assert!(events.contains(&"activation_start:web2".to_string()));
    assert!(events.contains(&"activation_complete:web2:ok".to_string()));
    assert!(events.contains(&"phase_complete:web:ok".to_string()));
}

#[tokio::test]
async fn test_deploy_full_workflow_multi_phase_success() {
    // Test multiple phases deploying sequentially with all steps
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![
        make_host_metadata("db1", Some("database"), RebootPolicy::Never),
        make_host_metadata("web1", Some("web"), RebootPolicy::Never),
    ];

    let config = create_deploy_config(DeployStrategy::Switch, 2);

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
    .unwrap();

    assert_eq!(results.success_count(), 2);

    let events = observer.get_events();

    // Database phase should complete before web phase starts (parallel-per-host model)
    let db_phase_start = events
        .iter()
        .position(|e| e == "phase_start:database:1")
        .expect("database phase should start");
    let db_phase_complete = events
        .iter()
        .position(|e| e == "phase_complete:database:ok")
        .expect("database phase should complete");
    let web_phase_start = events
        .iter()
        .position(|e| e == "phase_start:web:1")
        .expect("web phase should start");

    assert!(db_phase_start < db_phase_complete);
    assert!(db_phase_complete < web_phase_start);
}

#[tokio::test]
async fn test_deploy_build_failure_stops_deployment() {
    // Build fails for one host, should stop immediately with no copy/activate
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new().with_build_failures(vec!["fail-host".to_string()]);
    let deployer = MockDeployBackend::new();
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata("fail-host", None, RebootPolicy::Never)];

    let config = create_deploy_config(DeployStrategy::Switch, 1);

    let result = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    };

    // Should succeed but with failures captured
    let results = result.unwrap();
    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);

    let events = observer.get_events();

    // Phase lifecycle events should be present
    assert!(events.contains(&"phase_start:default:1".to_string()));
    assert!(events.contains(&"phase_complete:default:failed".to_string()));

    // Build events should be present (parallel-per-host model)
    assert!(events.contains(&"build_start:fail-host".to_string()));
    assert!(events.contains(&"build_complete:fail-host:fail".to_string()));

    // NO copy/activate/reboot events should fire (build failed before these steps)
    assert!(!events.iter().any(|e| e.starts_with("copy_")));
    assert!(!events.iter().any(|e| e.starts_with("deploy_")));
    assert!(!events.iter().any(|e| e.starts_with("reboot_")));
}

#[tokio::test]
async fn test_deploy_copy_failure_stops_deployment() {
    // Copy fails for one host after successful build
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer =
        MockDeployBackend::new().with_copy_failures(vec!["fail-copy.example.com".to_string()]);
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata("fail-copy", None, RebootPolicy::Never)];

    let config = create_deploy_config(DeployStrategy::Switch, 1);

    let result = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    };

    // Should succeed but with failures captured
    let results = result.unwrap();
    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);

    let events = observer.get_events();

    // Phase lifecycle events should be present
    assert!(events.contains(&"phase_start:default:1".to_string()));
    assert!(events.contains(&"phase_complete:default:failed".to_string()));

    // Build should succeed (parallel-per-host model)
    assert!(events.contains(&"build_start:fail-copy".to_string()));
    assert!(events.contains(&"build_complete:fail-copy:ok".to_string()));

    // Copy should fail
    assert!(events.contains(&"copy_start:fail-copy".to_string()));
    assert!(events.contains(&"copy_complete:fail-copy:fail".to_string()));

    // NO activate/reboot events (copy failed before these steps)
    assert!(!events.iter().any(|e| e.starts_with("deploy_")));
    assert!(!events.iter().any(|e| e.starts_with("reboot_")));
}

#[tokio::test]
async fn test_deploy_copy_observer_hooks_called() {
    // Verify copy observer hooks fire in correct order
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![
        make_host_metadata("host1", Some("p1"), RebootPolicy::Never),
        make_host_metadata("host2", Some("p1"), RebootPolicy::Never),
    ];

    let config = create_deploy_config(DeployStrategy::Switch, 2);

    {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
    .unwrap();

    let events = observer.get_events();

    // In parallel-per-host model, verify each host completes its copy
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

    // Each host's copy_start should come before its copy_complete
    assert!(copy1_start_idx < copy1_complete_idx);
    assert!(copy2_start_idx < copy2_complete_idx);
}

#[tokio::test]
async fn test_deploy_build_observer_hooks_called() {
    // Verify build observer hooks fire in correct order
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![
        make_host_metadata("host1", Some("p1"), RebootPolicy::Never),
        make_host_metadata("host2", Some("p1"), RebootPolicy::Never),
    ];

    let config = create_deploy_config(DeployStrategy::Switch, 2);

    {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
    .unwrap();

    let events = observer.get_events();

    // In parallel-per-host model, verify each host completes its build
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

    // Each host's build_start should come before its build_complete
    assert!(build1_start_idx < build1_complete_idx);
    assert!(build2_start_idx < build2_complete_idx);
}

#[tokio::test]
async fn test_deploy_build_partial_failure() {
    // Build fails for host 2 of 2 in a phase
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new().with_build_failures(vec!["host2".to_string()]);
    let deployer = MockDeployBackend::new();
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![
        make_host_metadata("host1", Some("p1"), RebootPolicy::Never),
        make_host_metadata("host2", Some("p1"), RebootPolicy::Never),
    ];

    let config = create_deploy_config(DeployStrategy::Switch, 2);

    let result = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    };

    // Should succeed but with failures captured
    let results = result.unwrap();
    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);
    assert_eq!(results.success_count(), 1);

    let events = observer.get_events();

    // Parallel-per-host model: host1 can proceed even if host2 fails
    // host1 build should succeed
    assert!(events.contains(&"build_complete:host1:ok".to_string()));

    // host2 build should fail
    assert!(events.contains(&"build_complete:host2:fail".to_string()));

    // In parallel model, host1 continues through the pipeline even though host2 failed
    // host1 should complete copy, activate, etc.
    assert!(events
        .iter()
        .any(|e| e.starts_with("copy_") && e.contains("host1")));

    // host2 should NOT proceed to copy (it failed build)
    assert!(!events
        .iter()
        .any(|e| e.starts_with("copy_") && e.contains("host2")));
}

#[tokio::test]
async fn test_deploy_ignore_phases_flag() {
    // Test that ignore_phases groups all hosts into "_all" phase
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![
        make_host_metadata("db1", Some("database"), RebootPolicy::Never),
        make_host_metadata("web1", Some("web"), RebootPolicy::Never),
    ];

    let config = DeployConfiguration {
        ignore_phases: true,
        ..create_deploy_config(DeployStrategy::Switch, 2)
    };

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
    .unwrap();

    assert_eq!(results.success_count(), 2);

    let events = observer.get_events();

    // Both hosts should be in "_all" phase (parallel-per-host model)
    assert!(events.contains(&"phase_start:_all:2".to_string()));

    // Should NOT have separate database/web phases
    assert!(!events.iter().any(|e| e.contains(":database:")));
    assert!(!events.iter().any(|e| e.contains(":web:")));
}

#[tokio::test]
async fn test_deploy_phase_failure_stops_next_phase() {
    // Phase 1 activation fails, Phase 2 should never start
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new()
        .with_activate_failures(vec!["phase1-host.example.com".to_string()]);
    let reboot_detector = SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hosts = vec![
        make_host_metadata("phase1-host", Some("phase1"), RebootPolicy::Never),
        make_host_metadata("phase2-host", Some("phase2"), RebootPolicy::Never),
    ];

    let config = create_deploy_config(DeployStrategy::Switch, 1);

    // When activation fails, deploy() returns Err but observer events still fire
    let result = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    };

    // Deploy should succeed but with failures and skipped phases
    let results = result.expect("Deploy should succeed structurally");
    assert!(!results.all_succeeded(), "Deploy should have failures");
    assert_eq!(results.failure_count(), 1);
    assert_eq!(results.skipped_phases.len(), 1, "Phase2 should be skipped");

    let events = observer.get_events();

    // Phase1 should have all events (build, copy, deploy)
    assert!(events.iter().any(|e| e.contains("phase1")));
    assert!(events.iter().any(|e| e.contains("phase1-host")));

    // Phase2 should have NO events (build, copy, or activate)
    assert!(
        !events.iter().any(|e| e.contains("phase2")),
        "Phase2 should not start after phase1 failure"
    );
    assert!(
        !events.iter().any(|e| e.contains("phase2-host")),
        "Phase2 host should not be processed"
    );
}

#[tokio::test]
async fn test_deploy_reboot_execution_success() {
    // Test that reboot is executed when required and observer hooks are called
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // Configure reboot detection to require reboot
    use crate::backends::reboot::MockRebootBackend;
    use crate::backends::reboot::RebootReason;
    let mut reboot_required = std::collections::HashMap::new();
    reboot_required.insert(
        "reboot-host.example.com".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    );
    let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required.clone());

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "reboot-host",
        Some("default"),
        RebootPolicy::Auto,
    )];

    let config = create_deploy_config(DeployStrategy::Auto, 1);

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
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
    let activation_complete_idx = events
        .iter()
        .position(|e| e.starts_with("activation_complete:reboot-host"))
        .unwrap();

    assert!(
        activation_complete_idx < reboot_start_idx,
        "Reboot should happen after activation"
    );
}

#[tokio::test]
async fn test_deploy_reboot_execution_failure() {
    // Test that reboot failures are properly handled and reported
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // Configure reboot to be required but fail during execution
    use crate::backends::reboot::MockRebootBackend;
    use crate::backends::reboot::RebootReason;
    let mut reboot_required = std::collections::HashMap::new();
    reboot_required.insert(
        "fail-reboot.example.com".to_string(),
        RebootReason::InitrdChanged {
            old: "/nix/store/old-initrd".to_string(),
            new: "/nix/store/new-initrd".to_string(),
        },
    );
    let reboot_backend = MockRebootBackend::new()
        .with_reboot_required(reboot_required)
        .with_reboot_failures(vec!["fail-reboot.example.com".to_string()]);

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "fail-reboot",
        Some("default"),
        RebootPolicy::Auto,
    )];

    let config = create_deploy_config(DeployStrategy::Auto, 1);

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
    .unwrap();

    // Should report failure due to reboot failure
    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);

    // Check that deployment itself succeeded but reboot failed
    let host_result = &results.phases[0].hosts[0];
    assert_eq!(host_result.hostname, "fail-reboot");
    assert!(!host_result.success);
    assert!(
        host_result
            .error_message
            .as_ref()
            .unwrap()
            .contains("reboot failed"),
        "Error should mention reboot failure: {:?}",
        host_result.error_message
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

#[tokio::test]
async fn test_deploy_skip_reboot_when_not_required() {
    // Test that reboot hooks are NOT called when reboot is not required
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // No reboot required (empty HashMap)
    use crate::backends::reboot::MockRebootBackend;
    let reboot_backend = MockRebootBackend::new();

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "no-reboot",
        Some("default"),
        RebootPolicy::Auto,
    )];

    let config = create_deploy_config(DeployStrategy::Auto, 1);

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
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

    // Should have completed activation
    assert!(events.iter().any(|e| e.starts_with("activation_complete")));
}

#[tokio::test]
async fn test_deploy_boot_mode_never_reboots() {
    use crate::backends::reboot::{MockRebootBackend, RebootReason};

    for policy in [
        RebootPolicy::Auto,
        RebootPolicy::Never,
        RebootPolicy::Always,
    ] {
        let harness = DeployHarness {
            reboot_backend: MockRebootBackend::new().with_reboot_required(
                [(
                    "bravo.example.com".to_string(),
                    RebootReason::KernelChanged {
                        old: "/nix/store/old-kernel".to_string(),
                        new: "/nix/store/new-kernel".to_string(),
                    },
                )]
                .into_iter()
                .collect(),
            ),
            config: create_deploy_config(DeployStrategy::Boot, 1),
            ..DeployHarness::new()
        };

        let run = harness
            .run(vec![make_host_metadata("bravo", None, policy.clone())])
            .await;

        let bravo = run.host("bravo");
        assert!(bravo.success, "{:?}: {:?}", policy, bravo);
        assert_eq!(bravo.activation_mode, Some(ActivationMode::Boot));
        assert!(!bravo.rebooted, "{:?} rebooted", policy);
        assert!(
            !run.has_event_starting_with("reboot_start"),
            "{:?}: {:?}",
            policy,
            run.events
        );
    }
}

#[tokio::test]
async fn test_deploy_reboot_policy_always() {
    // Test that RebootPolicy::Always forces reboot even if not detected
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // No reboot detected, but policy is Always
    use crate::backends::reboot::MockRebootBackend;
    let reboot_backend = MockRebootBackend::new(); // Empty - no reboot required

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "always-reboot",
        Some("default"),
        RebootPolicy::Always, // Always reboot
    )];

    let config = create_deploy_config(DeployStrategy::Auto, 1);

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
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

#[tokio::test]
async fn test_deploy_reboot_policy_never() {
    // Test that RebootPolicy::Never prevents reboot even if detected
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // Reboot required but policy is Never
    use crate::backends::reboot::MockRebootBackend;
    use crate::backends::reboot::RebootReason;
    let mut reboot_required = std::collections::HashMap::new();
    reboot_required.insert(
        "never-reboot.example.com".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    );
    let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "never-reboot",
        Some("default"),
        RebootPolicy::Never, // Never reboot
    )];

    let config = create_deploy_config(DeployStrategy::Auto, 1);

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
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

    // Should still have activated successfully
    assert!(events.iter().any(|e| e.starts_with("activation_complete")));
}

#[tokio::test]
async fn test_deploy_skip_reboot_flag_overrides_always_policy() {
    // Test that allow_reboot=false prevents reboot even with RebootPolicy::Always
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // Policy is Always, but user says --skip-reboot
    use crate::backends::reboot::MockRebootBackend;
    let reboot_backend = MockRebootBackend::new();

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "always-reboot",
        Some("default"),
        RebootPolicy::Always, // Policy says always reboot
    )];

    let config = DeployConfiguration {
        allow_reboot: false,
        ..create_deploy_config(DeployStrategy::Auto, 1)
    };

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
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

    // Should still have activated successfully
    assert!(events.iter().any(|e| e.starts_with("activation_complete")));
}

#[tokio::test]
async fn test_deploy_skip_reboot_flag_with_detected_reboot() {
    // Test that allow_reboot=false prevents reboot even when detection says it's needed
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // Reboot detected as required but user says --skip-reboot
    use crate::backends::reboot::MockRebootBackend;
    use crate::backends::reboot::RebootReason;
    let mut reboot_required = std::collections::HashMap::new();
    reboot_required.insert(
        "needs-reboot.example.com".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    );
    let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "needs-reboot",
        Some("default"),
        RebootPolicy::Auto, // Auto policy
    )];

    let config = DeployConfiguration {
        allow_reboot: false,
        ..create_deploy_config(DeployStrategy::Auto, 1)
    };

    let results = {
        let deploy_ctx = DeployContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            None,
        );
        deploy(&deploy_ctx, hosts).await
    }
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

    // Should still have activated successfully (Boot mode without reboot)
    assert!(events.iter().any(|e| e.starts_with("activation_complete")));
}

#[tokio::test]
async fn test_deploy_never_reboots_darwin_hosts() {
    use crate::backends::reboot::{MockRebootBackend, RebootReason};

    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_backend = MockRebootBackend::new().with_reboot_required(
        [(
            "falcon.example.com".to_string(),
            RebootReason::CriticalService("audit.service".to_string()),
        )]
        .into_iter()
        .collect(),
    );

    for strategy in [DeployStrategy::Auto, DeployStrategy::Switch] {
        let observer = TestObserver::new();
        let mut falcon = make_host_metadata("falcon", Some("default"), RebootPolicy::Always);
        falcon.system_type = SystemType::Darwin;

        let config = create_deploy_config(strategy, 1);

        let results = {
            let deploy_ctx = DeployContext::new(
                &flake,
                &nix_backend,
                &deployer,
                &reboot_backend,
                &config,
                &observer,
                None,
            );
            deploy(&deploy_ctx, vec![falcon]).await
        }
        .unwrap();

        assert!(!results.phases[0].hosts[0].rebooted);
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.starts_with("reboot_start")),
            "{:?} deployment rebooted a Darwin host. Events: {:?}",
            strategy,
            events
        );
    }
}

#[tokio::test]
async fn test_each_host_activates_as_its_own_system_type() {
    let harness = DeployHarness::new();
    let mut falcon = make_host_metadata("falcon", Some("p1"), RebootPolicy::Auto);
    falcon.system_type = SystemType::Darwin;

    let run = harness
        .run(vec![
            falcon,
            make_host_metadata("bravo", Some("p1"), RebootPolicy::Never),
        ])
        .await;

    assert_eq!(run.results.success_count(), 2);
    assert_eq!(
        harness
            .deployer
            .activated_system_types("falcon.example.com"),
        [SystemType::Darwin]
    );
    assert_eq!(
        harness.deployer.activated_system_types("bravo.example.com"),
        [SystemType::NixOS]
    );
}

#[tokio::test]
async fn test_host_without_a_hostname_fails_alone() {
    let harness = DeployHarness::new();
    let mut falcon = make_host_metadata("falcon", Some("p1"), RebootPolicy::Auto);
    falcon.system_type = SystemType::Darwin;
    falcon.fqdn = None;

    let run = harness
        .run(vec![
            falcon,
            make_host_metadata("bravo", Some("p1"), RebootPolicy::Never),
        ])
        .await;

    let falcon = run.host("falcon");
    assert_eq!(falcon.failed_at_step, Some(PipelineStep::Build));
    assert!(
        falcon
            .error_message
            .as_deref()
            .is_some_and(|reason| reason.contains("Host 'falcon' has no networking.hostName")),
        "{falcon:?}"
    );
    assert!(run.host("bravo").success);
    assert_eq!(harness.deployer.copy_count("bravo.example.com"), 1);
}

#[tokio::test]
async fn test_each_host_gets_its_closure_copied_once() {
    let harness = DeployHarness::new();

    let run = harness
        .run(vec![
            make_host_metadata("bravo", Some("p1"), RebootPolicy::Never),
            make_host_metadata("raven", Some("p1"), RebootPolicy::Never),
        ])
        .await;

    assert_eq!(run.results.success_count(), 2);
    assert_eq!(harness.deployer.copy_count("bravo.example.com"), 1);
    assert_eq!(harness.deployer.copy_count("raven.example.com"), 1);
}
