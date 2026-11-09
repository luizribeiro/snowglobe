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
    let mut harness = DeployHarness::new();
    harness.deployer = host_running(
        "pomegranates.example.com",
        HostSystems::both(mock_built_system("pomegranates")),
    );
    // Booted kernel differs from the running system, as on a host that
    // switched since its last reboot; that alone must not make it changed
    harness.reboot_backend = MockRebootBackend::new().with_reboot_required(HashMap::from([(
        "pomegranates".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    )]));

    let run = harness.run(vec![kubernetes_node("pomegranates")]).await;

    let host = run.host("pomegranates");
    assert!(host.success);
    assert!(host.unchanged);
    assert!(!host.rebooted);
    assert_eq!(host.activation_mode, None);
    assert!(
        run.executed_hooks.is_empty(),
        "hooks ran: {:?}",
        run.executed_hooks
    );
    assert!(run.has_event("unchanged:pomegranates"));
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
    let mut harness = DeployHarness::new();
    harness.deployer = host_running(
        "pufferfish.example.com",
        HostSystems {
            running: mock_built_system("pufferfish"),
            next_boot: StorePath::new("/nix/store/staged-by-hand-nixos-system".to_string()),
        },
    );

    let run = harness.run(vec![kubernetes_node("pufferfish")]).await;

    let host = run.host("pufferfish");
    assert!(host.success);
    assert!(!host.unchanged);
    assert!(host.rebooted, "rebootPolicy always reboots a changed host");
    assert!(run.has_event_starting_with("activation_start:pufferfish"));
    assert!(run
        .executed_hooks
        .iter()
        .any(|(_, stage, _, _)| *stage == HookStage::PreDeploy));
}

#[tokio::test]
async fn test_host_with_unreadable_systems_is_deployed() {
    let mut harness = DeployHarness::new();
    harness.deployer =
        MockDeployBackend::new().with_host_systems_failures(vec!["alpha.example.com".to_string()]);

    let run = harness
        .run(vec![make_host_metadata(
            "alpha",
            Some("p1"),
            RebootPolicy::Auto,
        )])
        .await;

    let host = run.host("alpha");
    assert!(host.success);
    assert!(!host.unchanged);
    assert!(run.has_event_starting_with("activation_start:alpha"));
}

#[tokio::test]
async fn test_host_whose_check_times_out_fails_before_the_copy() {
    let mut harness = DeployHarness::new();
    harness.deployer =
        MockDeployBackend::new().with_host_systems_timeouts(vec!["condor.example.com".to_string()]);

    let run = harness
        .run(vec![
            make_host_metadata("alpha", Some("p2"), RebootPolicy::Auto),
            make_host_metadata("condor", Some("p2"), RebootPolicy::Auto),
        ])
        .await;

    let condor = run.host("condor");
    assert_eq!(condor.failed_at_step, Some(PipelineStep::Check));
    assert!(
        condor
            .error_message
            .as_deref()
            .is_some_and(|m| m.contains("did not finish within 60s")),
        "{:?}",
        condor.error_message
    );
    assert!(!run.has_event_starting_with("copy_start:condor"));
    assert!(run.host("alpha").success);
}

#[tokio::test]
async fn test_only_unchanged_hosts_are_skipped_in_a_phase() {
    let mut harness = DeployHarness::new();
    harness.deployer = host_running(
        "bravo.example.com",
        HostSystems::both(mock_built_system("bravo")),
    );

    let run = harness
        .run(vec![
            make_host_metadata("bravo", Some("p1"), RebootPolicy::Auto),
            make_host_metadata("cactus", Some("p1"), RebootPolicy::Auto),
        ])
        .await;

    assert!(run.results.all_succeeded());
    assert!(run.host("bravo").unchanged);
    assert!(!run.host("cactus").unchanged);
    assert!(run.has_event_starting_with("activation_start:cactus"));
    assert!(!run.has_event_starting_with("activation_start:bravo"));
}

#[tokio::test]
async fn test_redeploy_deploys_unchanged_host() {
    let mut harness = DeployHarness::new();
    harness.deployer = host_running(
        "kestrel.example.com",
        HostSystems::both(mock_built_system("kestrel")),
    );
    harness.config.redeploy = true;

    let run = harness.run(vec![kubernetes_node("kestrel")]).await;

    let host = run.host("kestrel");
    assert!(host.success);
    assert!(!host.unchanged);
    assert!(run.has_event_starting_with("activation_start:kestrel"));
    assert!(run
        .executed_hooks
        .iter()
        .any(|(_, stage, _, _)| *stage == HookStage::PreDeploy));
}
