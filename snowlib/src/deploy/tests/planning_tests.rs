//! Tests for create_deployment_plans and reboot policies

use super::*;
use crate::backends::reboot::{
    RebootCheck, RebootDetectionConfig, RebootReason, SystemRebootBackend,
};
use std::time::Duration;

#[test]
fn test_deploy_results_counts() {
    let results = DeployResults {
        phases: vec![PhaseResult {
            phase: "default".to_string(),
            duration: Duration::ZERO,
            hosts: vec![
                HostDeploymentResult {
                    hostname: "host1".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    unchanged: false,
                    optional: false,
                    ignored_failing: Vec::new(),
                    error_message: None,
                },
                HostDeploymentResult {
                    hostname: "host2".to_string(),
                    success: false,
                    failed_at_step: Some(PipelineStep::Activate),
                    activation_mode: None,
                    rebooted: false,
                    unchanged: false,
                    optional: false,
                    ignored_failing: Vec::new(),
                    error_message: Some("failed".to_string()),
                },
                HostDeploymentResult {
                    hostname: "host3".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    unchanged: false,
                    optional: false,
                    ignored_failing: Vec::new(),
                    error_message: None,
                },
            ],
        }],
        skipped_phases: vec![],
    };

    assert_eq!(results.success_count(), 2);
    assert_eq!(results.failure_count(), 1);
    assert!(!results.all_succeeded());
}

#[test]
fn test_deploy_results_all_succeeded() {
    let results = DeployResults {
        phases: vec![PhaseResult {
            phase: "default".to_string(),
            duration: Duration::ZERO,
            hosts: vec![
                HostDeploymentResult {
                    hostname: "host1".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    unchanged: false,
                    optional: false,
                    ignored_failing: Vec::new(),
                    error_message: None,
                },
                HostDeploymentResult {
                    hostname: "host2".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    unchanged: false,
                    optional: false,
                    ignored_failing: Vec::new(),
                    error_message: None,
                },
            ],
        }],
        skipped_phases: vec![],
    };

    assert_eq!(results.success_count(), 2);
    assert_eq!(results.failure_count(), 0);
    assert!(results.all_succeeded());
}

#[tokio::test]
async fn test_create_deployment_plans_auto_mode_no_reboot() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Auto, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
    assert!(!plans[0].reboot_check.is_required());
}

#[tokio::test]
async fn test_create_deployment_plans_dry_run() {
    let detector = SystemRebootBackend::default();
    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::DryRun, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::DryActivate);
    assert_eq!(plans[0].reboot_check, RebootCheck::not_required());
}

#[tokio::test]
async fn test_boot_mode_plans_no_reboot_for_any_policy() {
    let detector = SystemRebootBackend::default();

    for policy in [
        RebootPolicy::Auto,
        RebootPolicy::Never,
        RebootPolicy::Always,
    ] {
        let hosts = vec![make_host("boron", None, policy.clone())];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeployStrategy::Boot, false)
                .await
                .unwrap();

        let plan = (plans[0].activation_mode, plans[0].reboot_check.clone());
        assert!(blocked.is_empty(), "{policy:?} blocked");
        assert_eq!(
            plan,
            (ActivationMode::Boot, RebootCheck::not_required()),
            "{policy:?}"
        );
    }
}

#[tokio::test]
async fn test_create_deployment_plans_test_mode() {
    let detector = SystemRebootBackend::default();
    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Test, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Test);
    assert_eq!(plans[0].reboot_check, RebootCheck::not_required());
}

#[tokio::test]
async fn test_create_deployment_plans_switch_with_force() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);
    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Switch, true)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
}

#[tokio::test]
async fn test_create_deployment_plans_multiple_hosts_mixed_results() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![
        make_host("host1", None, RebootPolicy::Auto),
        make_host("host2", None, RebootPolicy::Auto),
        make_host("host3", None, RebootPolicy::Auto),
    ];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Auto, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 3);
    assert_eq!(blocked.len(), 0);
    for plan in plans {
        assert_eq!(plan.activation_mode, ActivationMode::Switch);
    }
}

#[tokio::test]
async fn test_reboot_policy_never_uses_boot_mode() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Never)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Auto, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(!plans[0].reboot_check.is_required());
}

#[tokio::test]
async fn test_reboot_policy_always_uses_boot_mode_with_reboot() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Always)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Auto, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(plans[0].reboot_check.is_required());
}

#[tokio::test]
async fn test_reboot_policy_auto_uses_detection() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Auto)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Auto, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
    assert!(!plans[0].reboot_check.is_required());
}

#[tokio::test]
async fn test_reboot_policy_never_overrides_switch_mode() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Never)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Switch, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(!plans[0].reboot_check.is_required());
}

#[tokio::test]
async fn test_reboot_policy_always_overrides_switch_mode() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Always)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Switch, false)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(plans[0].reboot_check.is_required());
}

#[tokio::test]
async fn test_force_overrides_never_policy_with_switch() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Never)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Switch, true)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
}

#[tokio::test]
async fn test_force_overrides_always_policy_with_switch() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Always)];

    let (plans, blocked) = create_deployment_plans(hosts, &detector, DeployStrategy::Switch, true)
        .await
        .unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
}

#[tokio::test]
async fn test_darwin_hosts_switch_without_reboot_regardless_of_policy() {
    use crate::backends::reboot::MockRebootBackend;

    let kernel_changed = RebootReason::KernelChanged {
        old: "/nix/store/old-kernel".to_string(),
        new: "/nix/store/new-kernel".to_string(),
    };
    let detector = MockRebootBackend::new().with_reboot_required(
        [("ferrum.example.com".to_string(), kernel_changed)]
            .into_iter()
            .collect(),
    );

    for policy in [
        RebootPolicy::Auto,
        RebootPolicy::Never,
        RebootPolicy::Always,
    ] {
        for strategy in [DeployStrategy::Auto, DeployStrategy::Switch] {
            let (mut metadata, target, store_path) = make_host("ferrum", None, policy.clone());
            metadata.system_type = SystemType::Darwin;

            let (plans, blocked) = create_deployment_plans(
                vec![(metadata, target, store_path)],
                &detector,
                strategy,
                false,
            )
            .await
            .unwrap();

            assert!(blocked.is_empty(), "{:?}/{:?} blocked", policy, strategy);
            assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
            assert_eq!(plans[0].reboot_check, RebootCheck::not_required());
        }
    }
}
