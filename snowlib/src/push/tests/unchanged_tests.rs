//! Tests for skipping hosts that already run and would boot the built system

use super::*;
use crate::backends::deploy::HostSystems;
use crate::backends::reboot::{MockRebootBackend, RebootReason};

fn kubernetes_node(name: &str) -> HostMetadata {
    let mut host = make_host_metadata(name, Some("p1"), RebootPolicy::Always);
    host.hooks = kubernetes_node_hooks();
    host
}

fn host_running(fqdn: &str, systems: HostSystems) -> MockDeployBackend {
    MockDeployBackend::new().with_host_systems(HashMap::from([(fqdn.to_string(), systems)]))
}

#[tokio::test]
async fn test_unchanged_host_skips_hooks_activation_and_reboot() {
    let mut harness = PushHarness::new();
    harness.deployer = host_running(
        "praseodymium.example.com",
        HostSystems::both(mock_built_system("praseodymium")),
    );
    // Booted kernel differs from the running system, as on a host that
    // switched since its last reboot; that alone must not make it changed
    harness.reboot_backend = MockRebootBackend::new().with_reboot_required(HashMap::from([(
        "praseodymium".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    )]));

    let run = harness.run(vec![kubernetes_node("praseodymium")]).await;

    let host = run.host("praseodymium");
    assert!(host.success);
    assert!(host.unchanged);
    assert!(!host.rebooted);
    assert_eq!(host.activation_mode, None);
    assert!(
        run.executed_hooks.is_empty(),
        "hooks ran: {:?}",
        run.executed_hooks
    );
    assert!(run.has_event("unchanged:praseodymium"));
    for step in ["copy_start", "activation_start", "reboot_start"] {
        assert!(
            !run.has_event_starting_with(step),
            "{} happened: {:?}",
            step,
            run.events
        );
    }
}

#[tokio::test]
async fn test_host_booting_a_different_system_is_deployed() {
    let mut harness = PushHarness::new();
    harness.deployer = host_running(
        "promethium.example.com",
        HostSystems {
            running: mock_built_system("promethium"),
            next_boot: StorePath::new("/nix/store/staged-by-hand-nixos-system".to_string()),
        },
    );

    let run = harness.run(vec![kubernetes_node("promethium")]).await;

    let host = run.host("promethium");
    assert!(host.success);
    assert!(!host.unchanged);
    assert!(host.rebooted, "rebootPolicy always reboots a changed host");
    assert!(run.has_event_starting_with("activation_start:promethium"));
    assert!(run
        .executed_hooks
        .iter()
        .any(|(_, stage, _, _)| *stage == HookStage::PreDeploy));
}

#[tokio::test]
async fn test_host_with_unreadable_systems_is_deployed() {
    let mut harness = PushHarness::new();
    harness.deployer =
        MockDeployBackend::new().with_host_systems_failures(vec!["argon.example.com".to_string()]);

    let run = harness
        .run(vec![make_host_metadata(
            "argon",
            Some("p1"),
            RebootPolicy::Auto,
        )])
        .await;

    let host = run.host("argon");
    assert!(host.success);
    assert!(!host.unchanged);
    assert!(run.has_event_starting_with("activation_start:argon"));
}

#[tokio::test]
async fn test_only_unchanged_hosts_are_skipped_in_a_phase() {
    let mut harness = PushHarness::new();
    harness.deployer = host_running(
        "boron.example.com",
        HostSystems::both(mock_built_system("boron")),
    );

    let run = harness
        .run(vec![
            make_host_metadata("boron", Some("p1"), RebootPolicy::Auto),
            make_host_metadata("cerium", Some("p1"), RebootPolicy::Auto),
        ])
        .await;

    assert!(run.results.all_succeeded());
    assert!(run.host("boron").unchanged);
    assert!(!run.host("cerium").unchanged);
    assert!(run.has_event_starting_with("activation_start:cerium"));
    assert!(!run.has_event_starting_with("activation_start:boron"));
}

#[tokio::test]
async fn test_redeploy_deploys_unchanged_host() {
    let mut harness = PushHarness::new();
    harness.deployer = host_running(
        "krypton.example.com",
        HostSystems::both(mock_built_system("krypton")),
    );
    harness.config.redeploy = true;

    let run = harness.run(vec![kubernetes_node("krypton")]).await;

    let host = run.host("krypton");
    assert!(host.success);
    assert!(!host.unchanged);
    assert!(run.has_event_starting_with("activation_start:krypton"));
    assert!(run
        .executed_hooks
        .iter()
        .any(|(_, stage, _, _)| *stage == HookStage::PreDeploy));
}
