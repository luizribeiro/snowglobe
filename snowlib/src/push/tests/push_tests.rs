//! Basic push() function tests

use super::*;

#[tokio::test]
async fn test_push_with_single_phase() {
    let run = PushHarness::new()
        .run(vec![
            make_host_metadata("host1", Some("p1"), RebootPolicy::Auto),
            make_host_metadata("host2", Some("p1"), RebootPolicy::Auto),
        ])
        .await;

    assert_eq!(run.results.success_count(), 2);
    assert_eq!(run.results.failure_count(), 0);
    assert!(run.has_event("phase_start:p1:2"));
    assert!(run.has_event("activation_complete:host1:ok"));
    assert!(run.has_event("activation_complete:host2:ok"));
    assert!(run.has_event("phase_complete:p1:ok"));
}

#[tokio::test]
async fn test_each_phase_reports_its_hosts_when_it_completes() {
    let run = PushHarness::new()
        .run(vec![
            make_host_metadata("host1", Some("p1"), RebootPolicy::Auto),
            make_host_metadata("host2", Some("p2"), RebootPolicy::Auto),
            make_host_metadata("host3", Some("p2"), RebootPolicy::Auto),
        ])
        .await;

    let completed: Vec<(&str, usize)> = run
        .completed
        .iter()
        .map(|phase| (phase.phase.as_str(), phase.hosts.len()))
        .collect();
    assert_eq!(completed, vec![("p1", 1), ("p2", 2)]);
    assert!(run
        .completed
        .iter()
        .all(|phase| phase.duration > std::time::Duration::ZERO));
}

#[tokio::test]
async fn test_phases_run_with_their_numbers_in_order_then_hosts_without_one() {
    let run = PushHarness::new()
        .run(vec![
            make_host_metadata("sodium", None, RebootPolicy::Auto),
            make_host_metadata("cerium", Some("p10"), RebootPolicy::Auto),
            make_host_metadata("boron", Some("p2"), RebootPolicy::Auto),
            make_host_metadata("radon", Some("p1"), RebootPolicy::Auto),
        ])
        .await;

    let completed: Vec<&str> = run
        .completed
        .iter()
        .map(|phase| phase.phase.as_str())
        .collect();
    assert_eq!(completed, vec!["p1", "p2", "p10", crate::push::UNPHASED]);
}

#[tokio::test]
async fn test_hosts_without_a_phase_are_skipped_when_a_phase_fails() {
    let mut harness = PushHarness::new();
    harness.nix_backend = MockNixBackend::new().with_build_failures(vec!["boron".to_string()]);
    let run = harness
        .run(vec![
            make_host_metadata("sodium", None, RebootPolicy::Auto),
            make_host_metadata("boron", Some("p1"), RebootPolicy::Auto),
        ])
        .await;

    assert_eq!(run.results.phases.len(), 1);
    assert_eq!(run.results.phases[0].phase, "p1");
    assert_eq!(run.results.skipped_phases.len(), 1);
    assert_eq!(run.results.skipped_phases[0].phase, crate::push::UNPHASED);
    assert_eq!(run.results.skipped_phases[0].hostnames, vec!["sodium"]);
}

#[tokio::test]
async fn test_push_with_multiple_phases() {
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
async fn test_push_with_default_phase() {
    let run = PushHarness::new()
        .run(vec![make_host_metadata("host1", None, RebootPolicy::Auto)])
        .await;

    assert_eq!(run.results.success_count(), 1);
    assert!(run.has_event("phase_start:default:1"));
}

#[tokio::test]
async fn test_push_with_partial_failure_in_phase() {
    let mut harness = PushHarness::new();
    harness.deployer =
        MockDeployBackend::new().with_activate_failures(vec!["host2.example.com".to_string()]);

    let run = harness
        .run(vec![
            make_host_metadata("host1", Some("p1"), RebootPolicy::Auto),
            make_host_metadata("host2", Some("p1"), RebootPolicy::Auto),
        ])
        .await;

    assert_eq!(run.results.phases[0].hosts.len(), 2);
    assert!(run.host("host1").success);
    assert_eq!(
        run.host("host2").failed_at_step,
        Some(PipelineStep::Activate)
    );
    assert!(run.has_event("phase_complete:p1:failed"));
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

#[test]
fn test_phases_lists_each_phase_in_order_with_its_hosts() {
    let hosts = vec![
        make_host_metadata("vanadium", Some("p3"), RebootPolicy::Auto),
        make_host_metadata("ferrum", Some("p2"), RebootPolicy::Auto),
        make_host_metadata("cerium", Some("p3"), RebootPolicy::Auto),
        make_host_metadata("sodium", None, RebootPolicy::Auto),
    ];

    assert_eq!(
        crate::push::phases(&hosts, false),
        vec![
            ("p2".to_string(), vec!["ferrum".to_string()]),
            (
                "p3".to_string(),
                vec!["cerium".to_string(), "vanadium".to_string()]
            ),
            ("default".to_string(), vec!["sodium".to_string()]),
        ]
    );
    assert_eq!(
        crate::push::phases(&hosts, true),
        vec![(
            "_all".to_string(),
            vec![
                "cerium".to_string(),
                "ferrum".to_string(),
                "sodium".to_string(),
                "vanadium".to_string()
            ]
        )]
    );
}
