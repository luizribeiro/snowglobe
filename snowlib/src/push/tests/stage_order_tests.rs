//! Tests for the order of planning, hooks, activation and reboot

use super::*;
use crate::backends::reboot::{MockRebootBackend, RebootReason};

fn kubernetes_node(name: &str) -> HostMetadata {
    let mut host = make_host_metadata(name, Some("p1"), RebootPolicy::Auto);
    host.hooks = kubernetes_node_hooks();
    host
}

#[tokio::test]
async fn test_host_blocked_by_planning_is_never_drained() {
    let mut harness = PushHarness::new();
    harness.config.strategy = PushStrategy::Switch;
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

fn position(run: &HarnessRun, event: &str) -> usize {
    run.events
        .iter()
        .position(|e| e == event)
        .unwrap_or_else(|| panic!("missing {}: {:?}", event, run.events))
}

#[tokio::test]
async fn test_hooks_wrap_activation_in_order() {
    let run = PushHarness::new()
        .run(vec![kubernetes_node("samarium")])
        .await;

    assert!(run.results.all_succeeded());
    let pre = position(&run, "hooks_complete:samarium:PreActivate:ok");
    let activate = position(&run, "activation_start:samarium");
    let activated = position(&run, "activation_complete:samarium:ok");
    let post = position(&run, "hooks_start:samarium:PostActivate");
    assert!(
        pre < activate && activate < activated && activated < post,
        "{:?}",
        run.events
    );
}

#[tokio::test]
async fn test_failed_drain_does_not_start_activation() {
    let mut harness = PushHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_local_failures(vec!["kubectl drain ${SNOW_HOSTNAME}".to_string()]),
    );

    let run = harness.run(vec![kubernetes_node("vanadium")]).await;

    let host = run.host("vanadium");
    assert!(!host.success);
    assert_eq!(host.failed_at_step, Some(PipelineStep::Activate));
    assert!(
        !run.has_event_starting_with("activation_start"),
        "{:?}",
        run.events
    );
}
