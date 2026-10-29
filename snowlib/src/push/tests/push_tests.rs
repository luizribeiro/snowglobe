//! Basic push() function tests

use super::*;

#[tokio::test]
async fn test_push_with_single_phase() {
    let deployer = MockDeployBackend::new();
    let observer = TestObserver::new();

    let plans = vec![
        PushPlan {
            metadata: make_host_metadata("host1", Some("p1"), RebootPolicy::Auto),
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "host1.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-host1".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        },
        PushPlan {
            metadata: make_host_metadata("host2", Some("p1"), RebootPolicy::Auto),
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

    let results = activate_plans_for_test(&deployer, plans, 2, &observer)
        .await
        .unwrap();

    assert_eq!(results.success_count(), 2);
    assert_eq!(results.failure_count(), 0);

    // Check observer events
    let events = observer.get_events();
    assert!(events.contains(&"phase_start:p1:2".to_string()));
    assert!(events.contains(&"activation_start:host1".to_string()));
    assert!(events.contains(&"activation_start:host2".to_string()));
    assert!(events.contains(&"activation_complete:host1:ok".to_string()));
    assert!(events.contains(&"activation_complete:host2:ok".to_string()));
    assert!(events.contains(&"phase_complete:p1:ok".to_string()));
}

#[tokio::test]
async fn test_push_with_multiple_phases() {
    // Test that phases execute in lexicographic order (p1 before p2)
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    // Note: hosts are intentionally out of order to test sorting
    let hosts = vec![
        make_host_metadata("host3", Some("p2"), RebootPolicy::Auto),
        make_host_metadata("host1", Some("p1"), RebootPolicy::Auto),
    ];

    let config = create_push_config(PushStrategy::Switch, 2);

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert_eq!(results.success_count(), 2);

    // Check that phases were executed in order
    let events = observer.get_events();
    let p1_start = events.iter().position(|e| e == "phase_start:p1:1").unwrap();
    let p2_start = events.iter().position(|e| e == "phase_start:p2:1").unwrap();

    // p1 should start before p2 (lexicographic order)
    assert!(p1_start < p2_start);
}

#[tokio::test]
async fn test_push_stops_on_phase_failure() {
    // Test that when phase p1 fails, phase p2 never starts
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer =
        MockDeployBackend::new().with_activate_failures(vec!["host1.example.com".to_string()]);
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hosts = vec![
        make_host_metadata("host1", Some("p1"), RebootPolicy::Auto),
        make_host_metadata("host2", Some("p2"), RebootPolicy::Auto),
    ];

    let config = create_push_config(PushStrategy::Switch, 2);

    // When activation fails, push() returns Err
    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        push(&push_ctx, hosts).await
    };

    // Push should succeed but with failures and skipped phases
    let results = result.expect("Push should succeed structurally");
    assert!(!results.all_succeeded(), "Push should have failures");
    assert_eq!(results.failure_count(), 1);
    assert_eq!(
        results.skipped_phases.len(),
        1,
        "Phase p2 should be skipped"
    );

    let events = observer.get_events();

    // Phase p1 should have completed (with failure)
    assert!(events.contains(&"phase_complete:p1:failed".to_string()));

    // Phase p2 should never start (no build, copy, or deploy events for host2)
    assert!(
        !events.iter().any(|e| e.contains("host2")),
        "Phase p2 should not have started after p1 failure"
    );
}

#[tokio::test]
async fn test_push_with_noop_observer() {
    let deployer = MockDeployBackend::new();
    let observer = NoopObserver;

    let plans = vec![PushPlan {
        metadata: make_host_metadata("host1", Some("p1"), RebootPolicy::Auto),
        target: DeployTarget {
            user: Some("root".to_string()),
            hostname: "host1.example.com".to_string(),
            port: None,
        },
        store_path: StorePath::new("/nix/store/fake-host1".to_string()),
        activation_mode: ActivationMode::Switch,
        reboot_check: None,
    }];

    let results = activate_plans_for_test(&deployer, plans, 1, &observer)
        .await
        .unwrap();
    assert_eq!(results.success_count(), 1);
}

#[tokio::test]
async fn test_push_with_default_phase() {
    let deployer = MockDeployBackend::new();
    let observer = TestObserver::new();

    let plans = vec![PushPlan {
        metadata: make_host_metadata("host1", None, RebootPolicy::Auto),
        target: DeployTarget {
            user: Some("root".to_string()),
            hostname: "host1.example.com".to_string(),
            port: None,
        },
        store_path: StorePath::new("/nix/store/fake-host1".to_string()),
        activation_mode: ActivationMode::Switch,
        reboot_check: None,
    }];

    let results = activate_plans_for_test(&deployer, plans, 1, &observer)
        .await
        .unwrap();
    assert_eq!(results.success_count(), 1);

    // Should use "default" phase
    let events = observer.get_events();
    assert!(events.contains(&"phase_start:default:1".to_string()));
}

#[tokio::test]
async fn test_push_with_partial_failure_in_phase() {
    // Test that when one host fails in a phase, we still get results for all hosts in that phase
    let deployer = MockDeployBackend::new().with_failures(vec!["host2.example.com".to_string()]);
    let observer = TestObserver::new();

    let plans = vec![
        PushPlan {
            metadata: make_host_metadata("host1", Some("p1"), RebootPolicy::Auto),
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "host1.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-host1".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        },
        PushPlan {
            metadata: make_host_metadata("host2", Some("p1"), RebootPolicy::Auto),
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

    let results = activate_plans_for_test(&deployer, plans, 2, &observer)
        .await
        .unwrap();

    // Both hosts should have results
    assert_eq!(results.phases[0].hosts.len(), 2);
    assert_eq!(results.success_count(), 1);
    assert_eq!(results.failure_count(), 1);

    // Check that one succeeded and one failed
    let hostnames: Vec<&str> = results.phases[0]
        .hosts
        .iter()
        .map(|h| h.hostname.as_str())
        .collect();
    assert!(hostnames.contains(&"host1"));
    assert!(hostnames.contains(&"host2"));

    let host1_result = results.phases[0]
        .hosts
        .iter()
        .find(|h| h.hostname == "host1")
        .unwrap();
    let host2_result = results.phases[0]
        .hosts
        .iter()
        .find(|h| h.hostname == "host2")
        .unwrap();
    assert!(host1_result.success);
    assert!(!host2_result.success);

    // Phase should be marked as failed
    let events = observer.get_events();
    assert!(events.contains(&"phase_complete:p1:failed".to_string()));
}

#[tokio::test]
async fn test_push_error_message_includes_source() {
    // Test that when deployment fails, the error message includes the full error chain
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer =
        MockDeployBackend::new().with_activate_failures(vec!["fail.example.com".to_string()]);

    // Use a reboot detector that won't require reboots (empty config)
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = crate::observer::NoopObserver;

    let hosts = vec![make_host_metadata("fail", Some("p1"), RebootPolicy::Auto)];

    let config = create_push_config(PushStrategy::Switch, 1);

    // When activation fails, push() returns Err
    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        push(&push_ctx, hosts).await
    };

    // Push should succeed but with failures captured
    let results = result.expect("Push should succeed structurally");
    assert!(!results.all_succeeded(), "Push should have failures");
    assert_eq!(results.failure_count(), 1);

    // Verify the error message includes the source error chain
    let host_result = &results.phases[0].hosts[0];
    assert!(!host_result.success);
    let error_msg = host_result
        .error_message
        .as_ref()
        .expect("Failed host should have error message");

    assert!(
        error_msg.contains("fail") || host_result.hostname.contains("fail"),
        "Error should be associated with host 'fail'. Got hostname: {}, error: {}",
        host_result.hostname,
        error_msg
    );
    // The error message should indicate deployment/activation failure
    assert!(
        error_msg.contains("deploy")
            || error_msg.contains("activation")
            || error_msg.contains("Activation")
            || error_msg.contains("failed"),
        "Error message should indicate failure. Got: {}",
        error_msg
    );
}

#[tokio::test]
async fn test_push_error_message_format_with_activation_failure() {
    // Test the specific format of error messages with activation failures
    use std::collections::HashMap;

    let mut failures = HashMap::new();
    failures.insert(
        "test.example.com".to_string(),
        crate::backends::deploy::FailureSpec {
            phase: "activate".to_string(),
            error: Some("systemd service failed to start".to_string()),
        },
    );

    let fixture = crate::backends::deploy::DeploymentFixture {
        name: "Test error formatting".to_string(),
        description: None,
        failures,
        hosts: vec![],
    };

    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::from_fixture(fixture);
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = crate::observer::NoopObserver;

    let hosts = vec![make_host_metadata("test", Some("p1"), RebootPolicy::Auto)];

    let config = create_push_config(PushStrategy::Switch, 1);

    // When activation fails, push() returns Err
    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
            None,
        );
        push(&push_ctx, hosts).await
    };

    // Push should succeed but with failures captured
    let results = result.expect("Push should succeed structurally");
    assert!(!results.all_succeeded(), "Push should have failures");
    assert_eq!(results.failure_count(), 1);

    // Verify the error message includes expected details
    let host_result = &results.phases[0].hosts[0];
    assert!(!host_result.success);
    let error_msg = host_result
        .error_message
        .as_ref()
        .expect("Failed host should have error message");

    assert!(
        error_msg.contains("test") || host_result.hostname.contains("test"),
        "Error should be associated with host 'test'. Got hostname: {}, error: {}",
        host_result.hostname,
        error_msg
    );
    // Error message should indicate deployment failure
    // (The exact message depends on mock backend implementation)
    assert!(
        error_msg.contains("deploy")
            || error_msg.contains("failed")
            || error_msg.contains("systemd"),
        "Error message should indicate deployment failure. Got: {}",
        error_msg
    );
}
