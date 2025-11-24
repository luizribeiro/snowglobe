//! Profile update integration tests

use super::*;
use crate::backends::reboot::SystemRebootBackend;

// ============================================================================
// Integration Tests for Profile Update Feature
// ============================================================================

fn create_profile_test_backends() -> (
    Flake,
    MockNixBackend,
    MockDeployBackend,
    SystemRebootBackend,
    TestObserver,
) {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_detector = SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();
    (flake, nix_backend, deployer, reboot_detector, observer)
}

fn create_nixos_host_metadata(name: &str, fqdn: &str) -> HostMetadata {
    HostMetadata {
        name: name.to_string(),
        fqdn: fqdn.to_string(),
        phase: Some("default".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Auto,
        system_type: SystemType::NixOS,
        hooks: vec![],
    }
}

fn create_push_config(strategy: PushStrategy, concurrency: usize) -> PushConfiguration {
    PushConfiguration {
        strategy,
        force: false,
        concurrency,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    }
}

#[test]
fn test_push_profile_update_for_switch_mode() {
    let (flake, nix_backend, deployer, reboot_detector, observer) = create_profile_test_backends();

    let hosts = vec![create_nixos_host_metadata(
        "test-host",
        "test-host.example.com",
    )];
    let config = create_push_config(PushStrategy::Switch, 1);

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
        push(&push_ctx, hosts)
    }
    .unwrap();

    // Deployment should succeed
    assert!(results.all_succeeded());
    assert_eq!(results.success_count(), 1);
    assert_eq!(results.failure_count(), 0);

    let host_result = &results.phases[0].hosts[0];
    assert!(host_result.success);
    assert_eq!(host_result.activation_mode, Some(ActivationMode::Switch));

    // Verify profile update was called for Switch mode
    let profile_updates = deployer.get_profile_updates();
    assert_eq!(
        profile_updates.len(),
        1,
        "Profile update should be called once"
    );
    assert_eq!(profile_updates[0].0, "test-host.example.com");
    assert_eq!(profile_updates[0].1, ActivationMode::Switch);
}

#[test]
fn test_push_profile_update_skipped_for_test_mode() {
    let (flake, nix_backend, deployer, reboot_detector, observer) = create_profile_test_backends();

    let hosts = vec![create_nixos_host_metadata(
        "test-host",
        "test-host.example.com",
    )];
    let config = create_push_config(PushStrategy::Test, 1);

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
        push(&push_ctx, hosts)
    }
    .unwrap();

    // Deployment should succeed
    assert!(results.all_succeeded());
    assert_eq!(results.success_count(), 1);

    let host_result = &results.phases[0].hosts[0];
    assert!(host_result.success);
    assert_eq!(host_result.activation_mode, Some(ActivationMode::Test));

    // Verify profile update was NOT called for Test mode (ephemeral)
    let profile_updates = deployer.get_profile_updates();
    assert_eq!(
        profile_updates.len(),
        0,
        "Profile update should not be called for Test mode"
    );
}

#[test]
fn test_push_profile_update_multiple_hosts() {
    let (flake, nix_backend, deployer, reboot_detector, observer) = create_profile_test_backends();

    let hosts = vec![
        create_nixos_host_metadata("host1", "host1.example.com"),
        create_nixos_host_metadata("host2", "host2.example.com"),
        create_nixos_host_metadata("host3", "host3.example.com"),
    ];
    let config = create_push_config(PushStrategy::Switch, 3);

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
        push(&push_ctx, hosts)
    }
    .unwrap();

    // All hosts should succeed
    assert!(results.all_succeeded());
    assert_eq!(results.success_count(), 3);
    assert_eq!(results.failure_count(), 0);

    // All hosts should have used Switch mode (profile update)
    for host_result in &results.phases[0].hosts {
        assert!(host_result.success);
        assert_eq!(host_result.activation_mode, Some(ActivationMode::Switch));
    }

    // Verify profile update was called for all hosts
    let profile_updates = deployer.get_profile_updates();
    assert_eq!(
        profile_updates.len(),
        3,
        "Profile update should be called once per host"
    );

    // Verify each host had profile update called with Switch mode
    for host in [
        "host1.example.com",
        "host2.example.com",
        "host3.example.com",
    ] {
        assert!(
            deployer.profile_update_called_for(host),
            "Profile update should be called for {}",
            host
        );
    }
}
