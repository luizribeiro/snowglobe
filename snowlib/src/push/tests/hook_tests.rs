//! Hook integration tests

use super::*;
use crate::backends::hook::MockHookBackend;
use crate::hooks::{HookConfig, HookLocation, HookStage};

// ============================================================================
// Hook Integration Tests
// ============================================================================

#[tokio::test]
async fn test_push_with_pre_activate_hooks() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Auto,
        system_type: SystemType::NixOS,
        hooks: vec![
            HookConfig::test_fixture(
                "test-hook",
                HookStage::PreActivate,
                HookLocation::Local,
                "kubectl drain ${SNOW_HOSTNAME} --timeout=300s",
            ),
            HookConfig::test_fixture(
                "test-hook",
                HookStage::PreActivate,
                HookLocation::Remote,
                "systemctl stop important-service",
            ),
        ],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 2);
    assert_eq!(executed_hooks[0].1, HookStage::PreActivate);
    assert_eq!(executed_hooks[0].2, HookLocation::Local);
    assert!(executed_hooks[0].3.contains("kubectl drain"));
    assert_eq!(executed_hooks[1].1, HookStage::PreActivate);
    assert_eq!(executed_hooks[1].2, HookLocation::Remote);
    assert!(executed_hooks[1].3.contains("systemctl stop"));
}

#[tokio::test]
async fn test_push_with_post_activate_hooks() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Auto,
        system_type: SystemType::NixOS,
        hooks: vec![HookConfig::test_fixture(
            "test-hook",
            HookStage::PostActivate,
            HookLocation::Remote,
            "systemctl start important-service",
        )],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 1);
    assert_eq!(executed_hooks[0].1, HookStage::PostActivate);
    assert_eq!(executed_hooks[0].2, HookLocation::Remote);
    assert!(executed_hooks[0].3.contains("systemctl start"));
}

#[tokio::test]
async fn test_push_with_post_reboot_hooks() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    use crate::backends::reboot::MockRebootBackend;
    let reboot_backend = MockRebootBackend::new();

    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Always,
        system_type: SystemType::NixOS,
        hooks: vec![HookConfig::test_fixture(
            "test-hook",
            HookStage::PostReboot,
            HookLocation::Local,
            "kubectl uncordon ${SNOW_HOSTNAME}",
        )],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let phase_result = &results.phases[0];
    assert!(
        phase_result.hosts[0].rebooted,
        "Host should have rebooted due to reboot_policy=Always"
    );

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 1);
    assert_eq!(executed_hooks[0].1, HookStage::PostReboot);
    assert_eq!(executed_hooks[0].2, HookLocation::Local);
    assert!(executed_hooks[0].3.contains("kubectl uncordon"));
}

#[tokio::test]
async fn test_push_post_reboot_hooks_not_executed_when_reboot_fails() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    use crate::backends::reboot::MockRebootBackend;
    let reboot_backend =
        MockRebootBackend::new().with_reboot_failures(vec!["test-host".to_string()]);

    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Always,
        system_type: SystemType::NixOS,
        hooks: vec![HookConfig::test_fixture(
            "test-hook",
            HookStage::PostReboot,
            HookLocation::Local,
            "kubectl uncordon ${SNOW_HOSTNAME}",
        )],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(
        !results.all_succeeded(),
        "Push should fail when reboot fails"
    );

    let phase_result = &results.phases[0];
    assert!(
        !phase_result.hosts[0].rebooted,
        "Host should not be marked as rebooted when reboot fails"
    );
    assert!(
        !phase_result.hosts[0].success,
        "Host deployment should fail when reboot fails"
    );

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(
        executed_hooks.len(),
        0,
        "Post-reboot hooks should not execute when reboot fails"
    );
}

#[tokio::test]
async fn test_push_pre_activate_hook_failure_aborts_deployment() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hook_backend =
        Arc::new(MockHookBackend::new().with_local_failures(vec!["kubectl".to_string()]));

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Auto,
        system_type: SystemType::NixOS,
        hooks: vec![HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "kubectl drain ${SNOW_HOSTNAME}",
        )],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);

    let phase_result = &results.phases[0];
    assert_eq!(
        phase_result.hosts[0].failed_at_step,
        Some(PipelineStep::Activate),
        "Pre-activate hook failure should be reported as Activate step failure"
    );

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 1);
}

#[tokio::test]
async fn test_push_hook_failure_with_continue_on_failure() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hook_backend =
        Arc::new(MockHookBackend::new().with_local_failures(vec!["kubectl".to_string()]));

    let mut hook = HookConfig::test_fixture(
        "test-hook",
        HookStage::PreActivate,
        HookLocation::Local,
        "kubectl drain ${SNOW_HOSTNAME}",
    );
    hook.continue_on_failure = true;

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Auto,
        system_type: SystemType::NixOS,
        hooks: vec![hook],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 1);
}

#[tokio::test]
async fn test_push_on_failure_hooks_execute_when_build_fails() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new().with_build_failures(vec!["test-host".to_string()]);
    let deployer = MockDeployBackend::new();
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Auto,
        system_type: SystemType::NixOS,
        hooks: vec![
            HookConfig::test_fixture(
                "test-hook",
                HookStage::PreActivate,
                HookLocation::Local,
                "pre-activate hook",
            ),
            HookConfig::test_fixture(
                "test-hook",
                HookStage::OnFailure,
                HookLocation::Local,
                "cleanup on failure",
            ),
        ],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);

    let executed_hooks = hook_backend.get_executed_hooks();
    assert!(
        executed_hooks.iter().any(|h| h.1 == HookStage::OnFailure),
        "OnFailure hook should execute when build fails"
    );
    assert!(
        !executed_hooks.iter().any(|h| h.1 == HookStage::PreActivate),
        "PreActivate hook should not execute when build fails"
    );
}

#[tokio::test]
async fn test_push_on_failure_hooks_execute_when_copy_fails() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer =
        MockDeployBackend::new().with_copy_failures(vec!["test-host.example.com".to_string()]);
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::default();
    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Auto,
        system_type: SystemType::NixOS,
        hooks: vec![HookConfig::test_fixture(
            "test-hook",
            HookStage::OnFailure,
            HookLocation::Local,
            "rollback on failure",
        )],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(!results.all_succeeded());

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 1);
    assert_eq!(executed_hooks[0].1, HookStage::OnFailure);
}

#[tokio::test]
async fn test_push_environment_variables_passed_to_hooks() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.example.com".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Auto,
        system_type: SystemType::NixOS,
        hooks: vec![HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo $SNOW_HOSTNAME $SNOW_PHASE $SNOW_STORE_PATH",
        )],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 1);
    assert_eq!(executed_hooks[0].0, "test-host");
}

#[tokio::test]
async fn test_push_hooks_execute_in_parallel_for_multiple_hosts() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![
        HostMetadata {
            name: "host1".to_string(),
            fqdn: "host1.example.com".to_string(),
            phase: Some("production".to_string()),
            tags: vec![],
            reboot_policy: RebootPolicy::Auto,
            system_type: SystemType::NixOS,
            hooks: vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PreActivate,
                HookLocation::Local,
                "hook for host1",
            )],
        },
        HostMetadata {
            name: "host2".to_string(),
            fqdn: "host2.example.com".to_string(),
            phase: Some("production".to_string()),
            tags: vec![],
            reboot_policy: RebootPolicy::Auto,
            system_type: SystemType::NixOS,
            hooks: vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PreActivate,
                HookLocation::Local,
                "hook for host2",
            )],
        },
    ];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 2);
    assert!(executed_hooks.iter().any(|h| h.0 == "host1"));
    assert!(executed_hooks.iter().any(|h| h.0 == "host2"));
}

#[tokio::test]
async fn test_push_hook_failure_in_one_host_doesnt_affect_others() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hook_backend = Arc::new(
        MockHookBackend::new().with_remote_failures(vec!["host1.example.com".to_string()]),
    );

    let hosts = vec![
        HostMetadata {
            name: "host1".to_string(),
            fqdn: "host1.example.com".to_string(),
            phase: Some("production".to_string()),
            tags: vec![],
            reboot_policy: RebootPolicy::Auto,
            system_type: SystemType::NixOS,
            hooks: vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PreActivate,
                HookLocation::Remote,
                "will fail",
            )],
        },
        HostMetadata {
            name: "host2".to_string(),
            fqdn: "host2.example.com".to_string(),
            phase: Some("production".to_string()),
            tags: vec![],
            reboot_policy: RebootPolicy::Auto,
            system_type: SystemType::NixOS,
            hooks: vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PreActivate,
                HookLocation::Remote,
                "will succeed",
            )],
        },
    ];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert_eq!(results.success_count(), 1);
    assert_eq!(results.failure_count(), 1);

    let phase_result = &results.phases[0];
    let host1_result = phase_result
        .hosts
        .iter()
        .find(|h| h.hostname == "host1")
        .unwrap();
    let host2_result = phase_result
        .hosts
        .iter()
        .find(|h| h.hostname == "host2")
        .unwrap();

    assert!(!host1_result.success);
    assert!(host2_result.success);
}

#[tokio::test]
async fn test_push_hook_with_explicit_environment() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();

    let hook_backend = Arc::new(MockHookBackend::new());

    let mut hook = HookConfig::test_fixture(
        "env-test",
        HookStage::PreActivate,
        HookLocation::Local,
        "echo $CUSTOM_VAR",
    );
    hook.environment
        .insert("CUSTOM_VAR".to_string(), "custom_value".to_string());

    let hosts = vec![HostMetadata {
        name: "test-host".to_string(),
        fqdn: "test-host.local".to_string(),
        phase: Some("test".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Never,
        system_type: SystemType::NixOS,
        hooks: vec![hook],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: false,
    };

    let _results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    // Verify hook was executed (environment variable merging is tested in unit tests)
    let executed = hook_backend.get_executed_hooks();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].0, "test-host");
    assert_eq!(executed[0].1, HookStage::PreActivate);
}

// ============================================================================
// Integration Tests: Full Pipeline with Hooks
// ============================================================================

#[tokio::test]
async fn test_full_pipeline_all_hook_stages_execute_in_order() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    use crate::backends::reboot::MockRebootBackend;
    let reboot_backend = MockRebootBackend::new();

    let observer = TestObserver::new();
    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "k8s-node-01".to_string(),
        fqdn: "k8s-node-01.cluster.local".to_string(),
        phase: Some("production".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Always,
        system_type: SystemType::NixOS,
        hooks: vec![
            HookConfig::test_fixture(
                "drain-node",
                HookStage::PreActivate,
                HookLocation::Local,
                "kubectl drain k8s-node-01 --ignore-daemonsets --timeout=5m",
            ),
            HookConfig::test_fixture(
                "stop-services",
                HookStage::PreActivate,
                HookLocation::Remote,
                "systemctl stop kubelet containerd",
            ),
            HookConfig::test_fixture(
                "health-check",
                HookStage::PostActivate,
                HookLocation::Remote,
                "systemctl is-active kubelet containerd",
            ),
            HookConfig::test_fixture(
                "uncordon-node",
                HookStage::PostReboot,
                HookLocation::Local,
                "kubectl uncordon k8s-node-01",
            ),
            HookConfig::test_fixture(
                "slack-notify",
                HookStage::PostReboot,
                HookLocation::Local,
                "curl -X POST https://hooks.slack.com/services/XXX -d '{\"text\":\"Deploy complete\"}'",
            ),
        ],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let phase_result = &results.phases[0];
    assert!(
        phase_result.hosts[0].rebooted,
        "Host should have rebooted due to reboot_policy=Always"
    );

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 5, "All 5 hooks should have executed");

    // Verify hooks executed in correct order
    // Within each stage, hooks execute alphabetically by name
    assert_eq!(executed_hooks[0].1, HookStage::PreActivate);
    assert!(executed_hooks[0].3.contains("kubectl drain"));

    assert_eq!(executed_hooks[1].1, HookStage::PreActivate);
    assert!(executed_hooks[1].3.contains("systemctl stop"));

    assert_eq!(executed_hooks[2].1, HookStage::PostActivate);
    assert!(executed_hooks[2].3.contains("systemctl is-active"));

    // PostReboot hooks execute alphabetically: "slack-notify" < "uncordon-node"
    assert_eq!(executed_hooks[3].1, HookStage::PostReboot);
    assert!(executed_hooks[3].3.contains("curl"));

    assert_eq!(executed_hooks[4].1, HookStage::PostReboot);
    assert!(executed_hooks[4].3.contains("kubectl uncordon"));
}

#[tokio::test]
async fn test_pipeline_environment_variables_propagated_correctly() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    use crate::backends::reboot::MockRebootBackend;
    let reboot_backend = MockRebootBackend::new();

    let observer = TestObserver::new();

    // Use SystemHookBackend to actually execute commands and verify env vars
    use crate::backends::hook::SystemHookBackend;
    let real_hook_backend = Arc::new(SystemHookBackend::new(
        None,
        crate::process::OutputMode::Capture,
    ));

    let hosts = vec![HostMetadata {
        name: "web-server-01".to_string(),
        fqdn: "web-server-01.example.com".to_string(),
        phase: Some("staging".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Always,
        system_type: SystemType::NixOS,
        hooks: vec![
            HookConfig::test_fixture(
                "check-env-vars",
                HookStage::PreActivate,
                HookLocation::Local,
                "test -n \"$SNOW_HOSTNAME\" && test -n \"$SNOW_PHASE\" && test \"$SNOW_HOSTNAME\" = \"web-server-01\" && test \"$SNOW_PHASE\" = \"staging\"",
            ),
            HookConfig::test_fixture(
                "check-store-path",
                HookStage::PostActivate,
                HookLocation::Local,
                "test -n \"$SNOW_STORE_PATH\" && echo \"$SNOW_STORE_PATH\" | grep -q '^/nix/store/'",
            ),
        ],
    }];

    // Create hook executor manually since we're using real backend
    let mut hooks_by_host = std::collections::HashMap::new();
    for host in &hosts {
        hooks_by_host.insert(host.name.clone(), host.hooks.clone());
    }
    let hook_executor = crate::push::hooks::HookExecutor::new(real_hook_backend, hooks_by_host);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(
        results.all_succeeded(),
        "All environment variables should be set correctly in hooks"
    );
}

#[tokio::test]
async fn test_pipeline_hooks_execute_by_priority_order() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();
    let hook_backend = Arc::new(MockHookBackend::new());

    let mut hook1 = HookConfig::test_fixture(
        "third-priority",
        HookStage::PreActivate,
        HookLocation::Local,
        "echo third",
    );
    hook1.priority = 30;

    let mut hook2 = HookConfig::test_fixture(
        "first-priority",
        HookStage::PreActivate,
        HookLocation::Local,
        "echo first",
    );
    hook2.priority = 10;

    let mut hook3 = HookConfig::test_fixture(
        "second-priority",
        HookStage::PreActivate,
        HookLocation::Local,
        "echo second",
    );
    hook3.priority = 20;

    let hosts = vec![HostMetadata {
        name: "priority-test".to_string(),
        fqdn: "priority-test.local".to_string(),
        phase: Some("test".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Never,
        system_type: SystemType::NixOS,
        hooks: vec![hook1, hook2, hook3],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: false,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(executed_hooks.len(), 3);

    // Verify execution order matches priority (lower priority number = executes first)
    assert!(
        executed_hooks[0].3.contains("first"),
        "Hook with priority=10 should execute first"
    );
    assert!(
        executed_hooks[1].3.contains("second"),
        "Hook with priority=20 should execute second"
    );
    assert!(
        executed_hooks[2].3.contains("third"),
        "Hook with priority=30 should execute third"
    );
}

#[tokio::test]
async fn test_post_reboot_hooks_not_executed_when_no_reboot_needed() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();
    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "no-reboot-host".to_string(),
        fqdn: "no-reboot-host.local".to_string(),
        phase: Some("test".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Never,
        system_type: SystemType::NixOS,
        hooks: vec![
            HookConfig::test_fixture(
                "pre-activate-hook",
                HookStage::PreActivate,
                HookLocation::Local,
                "echo pre-activate",
            ),
            HookConfig::test_fixture(
                "post-activate-hook",
                HookStage::PostActivate,
                HookLocation::Local,
                "echo post-activate",
            ),
            HookConfig::test_fixture(
                "post-reboot-hook",
                HookStage::PostReboot,
                HookLocation::Local,
                "kubectl uncordon no-reboot-host",
            ),
        ],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: false,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let phase_result = &results.phases[0];
    assert!(
        !phase_result.hosts[0].rebooted,
        "Host should not have rebooted with reboot_policy=Never"
    );

    let executed_hooks = hook_backend.get_executed_hooks();
    assert_eq!(
        executed_hooks.len(),
        2,
        "Only pre-activate and post-activate hooks should execute"
    );

    assert_eq!(executed_hooks[0].1, HookStage::PreActivate);
    assert_eq!(executed_hooks[1].1, HookStage::PostActivate);

    // Verify post-reboot hook did NOT execute
    assert!(
        !executed_hooks.iter().any(|h| h.1 == HookStage::PostReboot),
        "Post-reboot hooks should not execute when host doesn't reboot"
    );
}

#[tokio::test]
async fn test_hook_observer_callbacks_invoked_correctly() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();
    let hook_backend = Arc::new(MockHookBackend::new());

    let hosts = vec![HostMetadata {
        name: "observer-test".to_string(),
        fqdn: "observer-test.local".to_string(),
        phase: Some("test".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Never,
        system_type: SystemType::NixOS,
        hooks: vec![
            HookConfig::test_fixture(
                "first-hook",
                HookStage::PreActivate,
                HookLocation::Local,
                "echo first",
            ),
            HookConfig::test_fixture(
                "second-hook",
                HookStage::PreActivate,
                HookLocation::Local,
                "echo second",
            ),
            HookConfig::test_fixture(
                "post-hook",
                HookStage::PostActivate,
                HookLocation::Local,
                "echo post",
            ),
        ],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: false,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(results.all_succeeded());

    let events = observer.get_events();

    // Verify hooks_start called for PreActivate stage
    assert!(
        events
            .iter()
            .any(|e| e == "hooks_start:observer-test:PreActivate"),
        "hooks_start should be called for PreActivate stage. Events: {:?}",
        events
    );

    // Verify individual hook_start/hook_complete called for each hook
    assert!(
        events
            .iter()
            .any(|e| e == "hook_start:observer-test:PreActivate:first-hook"),
        "hook_start should be called for first-hook. Events: {:?}",
        events
    );
    assert!(
        events
            .iter()
            .any(|e| e == "hook_complete:observer-test:PreActivate:first-hook:ok"),
        "hook_complete should be called for first-hook. Events: {:?}",
        events
    );
    assert!(
        events
            .iter()
            .any(|e| e == "hook_start:observer-test:PreActivate:second-hook"),
        "hook_start should be called for second-hook. Events: {:?}",
        events
    );
    assert!(
        events
            .iter()
            .any(|e| e == "hook_complete:observer-test:PreActivate:second-hook:ok"),
        "hook_complete should be called for second-hook. Events: {:?}",
        events
    );

    // Verify hooks_complete called for PreActivate stage
    assert!(
        events
            .iter()
            .any(|e| e == "hooks_complete:observer-test:PreActivate:ok"),
        "hooks_complete should be called for PreActivate stage. Events: {:?}",
        events
    );

    // Verify PostActivate hooks also trigger observer callbacks
    assert!(
        events
            .iter()
            .any(|e| e == "hooks_start:observer-test:PostActivate"),
        "hooks_start should be called for PostActivate stage. Events: {:?}",
        events
    );
    assert!(
        events
            .iter()
            .any(|e| e == "hook_start:observer-test:PostActivate:post-hook"),
        "hook_start should be called for post-hook. Events: {:?}",
        events
    );
    assert!(
        events
            .iter()
            .any(|e| e == "hooks_complete:observer-test:PostActivate:ok"),
        "hooks_complete should be called for PostActivate stage. Events: {:?}",
        events
    );
}

#[tokio::test]
async fn test_hook_observer_callbacks_on_failure() {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_backend = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();
    let hook_backend =
        Arc::new(MockHookBackend::new().with_local_failures(vec!["fail".to_string()]));

    let hosts = vec![HostMetadata {
        name: "fail-test".to_string(),
        fqdn: "fail-test.local".to_string(),
        phase: Some("test".to_string()),
        tags: vec![],
        reboot_policy: RebootPolicy::Never,
        system_type: SystemType::NixOS,
        hooks: vec![HookConfig::test_fixture(
            "failing-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "fail command",
        )],
    }];

    let hook_executor = make_hook_executor(hook_backend.clone(), &hosts);

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: false,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );
        push(&push_ctx, hosts).await
    }
    .unwrap();

    assert!(!results.all_succeeded());

    let events = observer.get_events();

    // Verify hook_start was called
    assert!(
        events
            .iter()
            .any(|e| e == "hook_start:fail-test:PreActivate:failing-hook"),
        "hook_start should be called even for failing hooks. Events: {:?}",
        events
    );

    // Verify hook_complete called with failure status
    assert!(
        events
            .iter()
            .any(|e| e == "hook_complete:fail-test:PreActivate:failing-hook:fail"),
        "hook_complete should be called with fail status. Events: {:?}",
        events
    );

    // Verify hooks_complete called with failure status
    assert!(
        events
            .iter()
            .any(|e| e == "hooks_complete:fail-test:PreActivate:fail"),
        "hooks_complete should be called with fail status. Events: {:?}",
        events
    );
}
