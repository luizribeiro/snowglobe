//! Tests for create_deployment_plans and reboot policies

use super::*;
use crate::backends::reboot::{RebootDetectionConfig, SystemRebootBackend};

#[test]
fn test_push_results_counts() {
    let results = PushResults {
        phases: vec![PhaseResult {
            phase: "default".to_string(),
            hosts: vec![
                HostDeploymentResult {
                    hostname: "host1".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    error_message: None,
                },
                HostDeploymentResult {
                    hostname: "host2".to_string(),
                    success: false,
                    failed_at_step: Some(PipelineStep::Activate),
                    activation_mode: None,
                    rebooted: false,
                    error_message: Some("failed".to_string()),
                },
                HostDeploymentResult {
                    hostname: "host3".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
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
fn test_push_results_all_succeeded() {
    let results = PushResults {
        phases: vec![PhaseResult {
            phase: "default".to_string(),
            hosts: vec![
                HostDeploymentResult {
                    hostname: "host1".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    error_message: None,
                },
                HostDeploymentResult {
                    hostname: "host2".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
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

#[test]
fn test_create_deployment_plans_auto_mode_no_reboot() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
    assert!(plans[0].reboot_check.is_some());
    assert!(!plans[0].reboot_check.as_ref().unwrap().required);
}

#[test]
fn test_create_deployment_plans_dry_run() {
    let detector = SystemRebootBackend::default();
    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::DryRun, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::DryActivate);
    assert!(plans[0].reboot_check.is_none());
}

#[test]
fn test_create_deployment_plans_boot_mode() {
    let detector = SystemRebootBackend::default();
    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Boot, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(plans[0].reboot_check.is_none());
}

#[test]
fn test_create_deployment_plans_test_mode() {
    let detector = SystemRebootBackend::default();
    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Test, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Test);
    assert!(plans[0].reboot_check.is_none());
}

#[test]
fn test_create_deployment_plans_switch_with_force() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);
    let hosts = vec![make_host("test", None, RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, true).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
}

#[test]
fn test_create_deployment_plans_multiple_hosts_mixed_results() {
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

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

    assert_eq!(plans.len(), 3);
    assert_eq!(blocked.len(), 0);
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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Never)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(plans[0].reboot_check.is_some());
    assert!(!plans[0].reboot_check.as_ref().unwrap().required);
}

#[test]
fn test_reboot_policy_always_uses_boot_mode_with_reboot() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Always)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(plans[0].reboot_check.is_some());
    assert!(plans[0].reboot_check.as_ref().unwrap().required);
}

#[test]
fn test_reboot_policy_auto_uses_detection() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Never)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(!plans[0].reboot_check.as_ref().unwrap().required);
}

#[test]
fn test_reboot_policy_always_overrides_switch_mode() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Always)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(plans[0].reboot_check.as_ref().unwrap().required);
}

#[test]
fn test_force_overrides_never_policy_with_switch() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Never)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, true).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
}

#[test]
fn test_force_overrides_always_policy_with_switch() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, RebootPolicy::Always)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, true).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
}
