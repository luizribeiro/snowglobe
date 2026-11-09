//! Tests for the order of planning, hooks, activation and reboot

use super::*;
use crate::backends::reboot::{MockRebootBackend, RebootReason};

#[tokio::test]
async fn test_host_blocked_by_planning_is_never_drained() {
    let mut harness = DeployHarness::new();
    harness.config.strategy = DeployStrategy::Switch;
    harness.reboot_backend = MockRebootBackend::new().with_reboot_required(HashMap::from([(
        "neodymium".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    )]));

    let run = harness.run(vec![kubernetes_node("neodymium")]).await;

    let host = run.host("neodymium");
    assert!(!host.success);
    assert_eq!(host.failed_at_step, Some(PipelineStep::Plan));
    let stages: Vec<HookStage> = run.executed_hooks.iter().map(|(_, s, _, _)| *s).collect();
    assert_eq!(stages, vec![HookStage::OnFailure]);
    assert!(
        !run.has_event_starting_with("activation_start"),
        "{:?}",
        run.events
    );
}

#[tokio::test]
async fn test_blocked_host_keeps_the_reason_it_was_blocked() {
    let mut harness = DeployHarness::new();
    harness.config.strategy = DeployStrategy::Switch;
    harness.reboot_backend = MockRebootBackend::new().with_reboot_required(HashMap::from([(
        "neodymium".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/0n6kcz8q4lw1-linux-6.12.58/bzImage".to_string(),
            new: "/nix/store/9s2mf0yh7l3a-linux-6.12.60/bzImage".to_string(),
        },
    )]));

    let run = harness.run(vec![kubernetes_node("neodymium")]).await;

    let message = run.host("neodymium").error_message.clone().unwrap();
    assert_eq!(message, "kernel linux-6.12.58 → linux-6.12.60");
}

#[tokio::test]
async fn test_hooks_wrap_activation_in_order() {
    let run = DeployHarness::new()
        .run(vec![kubernetes_node("samarium")])
        .await;

    assert!(run.results.all_succeeded());
    let pre = position(&run, "hooks_complete:samarium:pre-deploy:ok");
    let activate = position(&run, "activation_start:samarium");
    let activated = position(&run, "activation_complete:samarium:ok");
    let post = position(&run, "hooks_start:samarium:post-deploy");
    assert!(
        pre < activate && activate < activated && activated < post,
        "{:?}",
        run.events
    );
}

#[tokio::test]
async fn test_failed_drain_does_not_start_activation() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_local_failures(vec!["kubectl drain ${SNOW_HOSTNAME}".to_string()]),
    );

    let run = harness.run(vec![kubernetes_node("vanadium")]).await;

    let host = run.host("vanadium");
    assert!(!host.success);
    assert_eq!(host.failed_at_step, Some(PipelineStep::PreDeploy));
    assert!(
        !run.has_event_starting_with("activation_start"),
        "{:?}",
        run.events
    );
}

fn executed_stages(run: &HarnessRun) -> Vec<HookStage> {
    run.executed_hooks
        .iter()
        .map(|(_, stage, _, _)| *stage)
        .collect()
}

#[tokio::test]
async fn test_post_deploy_runs_last_after_a_reboot() {
    let harness = DeployHarness::new();
    let mut host = kubernetes_node("lanthanum");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    assert!(run.host("lanthanum").rebooted);
    assert_eq!(
        executed_stages(&run),
        vec![
            HookStage::PreDeploy,
            HookStage::PostReboot,
            HookStage::PostDeploy
        ]
    );
    let rebooted = position(&run, "reboot_complete:lanthanum:ok");
    let post_deploy = position(&run, "hooks_start:lanthanum:post-deploy");
    assert!(rebooted < post_deploy, "{:?}", run.events);
}

#[tokio::test]
async fn test_post_deploy_runs_without_a_reboot() {
    let run = DeployHarness::new()
        .run(vec![kubernetes_node("cerium")])
        .await;

    assert!(!run.host("cerium").rebooted);
    assert_eq!(
        executed_stages(&run),
        vec![HookStage::PreDeploy, HookStage::PostDeploy]
    );
}

#[tokio::test]
async fn test_post_deploy_skipped_when_reboot_fails() {
    let mut harness = DeployHarness::new();
    harness.reboot_backend =
        MockRebootBackend::new().with_reboot_failures(vec!["curium.example.com".to_string()]);
    let mut host = kubernetes_node("curium");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    assert!(!run.host("curium").success);
    assert!(!executed_stages(&run).contains(&HookStage::PostDeploy));
}

#[tokio::test]
async fn test_failed_post_deploy_hook_runs_on_failure_hooks() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_local_failures(vec!["kubectl uncordon ${SNOW_HOSTNAME}".to_string()]),
    );
    let mut host = kubernetes_node("oxygen");
    // Keep the on-failure hook from matching the failing uncordon command
    for hook in host
        .hooks
        .iter_mut()
        .filter(|h| h.stage == HookStage::OnFailure)
    {
        hook.command = "notify-oncall ${SNOW_HOSTNAME}".to_string();
    }

    let run = harness.run(vec![host]).await;

    let host = run.host("oxygen");
    assert!(!host.success);
    assert_eq!(host.failed_at_step, Some(PipelineStep::PostDeploy));
    assert_eq!(
        executed_stages(&run),
        vec![
            HookStage::PreDeploy,
            HookStage::PostDeploy,
            HookStage::OnFailure
        ]
    );
}

#[tokio::test]
async fn test_failed_reboot_runs_on_failure_hooks() {
    let mut harness = DeployHarness::new();
    harness.reboot_backend =
        MockRebootBackend::new().with_reboot_failures(vec!["boron.example.com".to_string()]);
    let mut host = kubernetes_node("boron");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    let host = run.host("boron");
    assert!(!host.success);
    assert_eq!(host.failed_at_step, Some(PipelineStep::Reboot));
    assert_eq!(host.activation_mode, Some(ActivationMode::Boot));
    assert!(!host.rebooted);
    assert!(host
        .error_message
        .as_deref()
        .is_some_and(|msg| msg.starts_with("Deployment succeeded but reboot failed")));
    assert_eq!(
        executed_stages(&run),
        vec![HookStage::PreDeploy, HookStage::OnFailure]
    );
}

#[tokio::test]
async fn test_dry_run_runs_no_hooks() {
    let mut harness = DeployHarness::new();
    harness.config.strategy = DeployStrategy::DryRun;
    let mut host = kubernetes_node("hydrogen");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    assert!(run.host("hydrogen").success);
    assert_eq!(
        run.host("hydrogen").activation_mode,
        Some(ActivationMode::DryActivate)
    );
    assert!(run.executed_hooks.is_empty(), "{:?}", run.executed_hooks);
}

#[tokio::test]
async fn test_failed_dry_run_runs_no_on_failure_hooks() {
    let mut harness = DeployHarness::new();
    harness.config.strategy = DeployStrategy::DryRun;
    harness.deployer =
        MockDeployBackend::new().with_activate_failures(vec!["oxygen.example.com".to_string()]);

    let run = harness.run(vec![kubernetes_node("oxygen")]).await;

    assert!(!run.host("oxygen").success);
    assert!(run.executed_hooks.is_empty(), "{:?}", run.executed_hooks);
}

const MISSING_GATE: &str =
    "/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6y-snowglobe-test-missing-gate";

fn node_with_gate(name: &str, location: crate::hooks::HookLocation) -> HostMetadata {
    let mut host = kubernetes_node(name);
    host.hooks.push(crate::hooks::HookConfig::test_fixture(
        "storage-gate",
        HookStage::PreDeploy,
        location,
        &format!("{}/bin/gate --wait", MISSING_GATE),
    ));
    host
}

#[tokio::test]
async fn test_local_hook_with_missing_store_path_fails_before_any_build() {
    let hosts = vec![
        make_host_metadata("boron", Some("p1"), RebootPolicy::Auto),
        node_with_gate("promethium", crate::hooks::HookLocation::Local),
    ];

    let (result, observer) = DeployHarness::new().run_deploy(hosts).await;
    let events = observer.get_events();

    assert!(
        matches!(
            result,
            Err(crate::error::SnowError::Deploy(
                crate::error::DeployError::HookPathMissing { ref host, ref path, .. }
            )) if host == "promethium" && path == MISSING_GATE
        ),
        "{result:?}"
    );
    assert!(events.is_empty(), "{events:?}");
}

#[tokio::test]
async fn test_remote_hook_store_paths_are_not_checked_locally() {
    let run = DeployHarness::new()
        .run(vec![node_with_gate(
            "promethium",
            crate::hooks::HookLocation::Remote,
        )])
        .await;

    assert!(run.host("promethium").success);
}

#[tokio::test]
async fn test_dry_run_skips_the_hook_path_check() {
    let mut harness = DeployHarness::new();
    harness.config.strategy = DeployStrategy::DryRun;

    let run = harness
        .run(vec![node_with_gate(
            "promethium",
            crate::hooks::HookLocation::Local,
        )])
        .await;

    assert!(run.host("promethium").success);
}

#[tokio::test]
async fn test_hung_ssh_agent_stops_the_deploy_before_any_build() {
    let mut harness = DeployHarness::new();
    harness.deployer = MockDeployBackend::new().with_hung_agent();

    let (result, observer) = harness.run_deploy(vec![kubernetes_node("cerium")]).await;

    assert!(matches!(
        result,
        Err(crate::error::SnowError::Deploy(
            crate::error::DeployError::SshAgentNotResponding { .. }
        ))
    ));
    assert!(
        observer.get_events().is_empty(),
        "{:?}",
        observer.get_events()
    );
    assert_eq!(harness.deployer.copy_count("cerium.example.com"), 0);
}

fn darwin_host(name: &str) -> HostMetadata {
    let mut host = make_host_metadata(name, Some("p1"), RebootPolicy::Auto);
    host.system_type = SystemType::Darwin;
    host.hooks = kubernetes_node_hooks();
    host
}

#[tokio::test]
async fn test_darwin_hosts_stop_a_deploy_that_does_not_switch_before_any_build() {
    for strategy in [
        DeployStrategy::Boot,
        DeployStrategy::Test,
        DeployStrategy::DryRun,
    ] {
        let mut harness = DeployHarness::new();
        harness.config.strategy = strategy;

        let (result, observer) = harness
            .run_deploy(vec![
                darwin_host("ferrum"),
                kubernetes_node("boron"),
                darwin_host("cobalt"),
            ])
            .await;

        assert!(
            matches!(
                result,
                Err(crate::error::SnowError::Deploy(
                    crate::error::DeployError::DarwinOnlySwitches { ref hosts, .. }
                )) if hosts == &["ferrum", "cobalt"]
            ),
            "{strategy:?}: {result:?}"
        );
        assert!(observer.get_events().is_empty(), "{strategy:?}");
        assert!(harness.hook_backend.get_executed_hooks().is_empty());
        assert_eq!(harness.deployer.copy_count("boron.example.com"), 0);
    }
}

#[tokio::test]
async fn test_darwin_hosts_join_a_deploy_that_switches() {
    for strategy in [DeployStrategy::Auto, DeployStrategy::Switch] {
        let mut harness = DeployHarness::new();
        harness.config.strategy = strategy;

        let run = harness
            .run(vec![darwin_host("ferrum"), kubernetes_node("boron")])
            .await;

        assert!(run.host("ferrum").success, "{strategy:?}");
        assert!(run.host("boron").success, "{strategy:?}");
    }
}
