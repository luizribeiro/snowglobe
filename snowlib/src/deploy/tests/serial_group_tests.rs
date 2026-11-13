//! Tests for hosts in a serial group deploying one at a time within a phase
//!
//! Mock builds and hooks sleep on tokio's paused clock, so their delays order
//! the hosts the same way however loaded the machine is.

use super::*;
use crate::backends::deploy::HostSystems;
use crate::backends::hook::MockHookBackend;
use std::time::Duration;

fn node(name: &str, group: Option<&str>) -> HostMetadata {
    let mut host = make_host_metadata(name, Some("p2"), RebootPolicy::Always);
    host.hooks = kubernetes_node_hooks();
    host.serial_group = group.map(str::to_string);
    host
}

/// A node whose drain hook fails, while other nodes' drains succeed
fn node_with_failing_drain(name: &str, group: Option<&str>) -> HostMetadata {
    let mut host = node(name, group);
    for hook in host
        .hooks
        .iter_mut()
        .filter(|h| h.stage == HookStage::PreDeploy)
    {
        hook.command = format!("kubectl drain {} --simulate-failure", name);
    }
    host
}

fn hooks_taking(delay: Duration) -> MockHookBackend {
    MockHookBackend::new().with_delay(delay)
}

/// Makes the listed hosts finish building later, so the others go first
fn builds_delayed(hosts: &[&str], delay: Duration) -> MockNixBackend {
    MockNixBackend::new()
        .with_build_delays(hosts.iter().map(|host| (host.to_string(), delay)).collect())
}

/// Event indexes from the host's first pre-deploy hook to its last post-deploy hook
fn deploy_span(run: &HarnessRun, hostname: &str) -> std::ops::RangeInclusive<usize> {
    let find = |event: String| {
        run.events
            .iter()
            .position(|e| *e == event)
            .unwrap_or_else(|| panic!("missing {}: {:?}", event, run.events))
    };
    find(format!("hooks_start:{}:pre-deploy", hostname))
        ..=find(format!("hooks_complete:{}:post-deploy:ok", hostname))
}

fn overlap(a: &std::ops::RangeInclusive<usize>, b: &std::ops::RangeInclusive<usize>) -> bool {
    a.start() < b.end() && b.start() < a.end()
}

fn ran_hooks(run: &HarnessRun, hostname: &str) -> bool {
    run.executed_hooks
        .iter()
        .any(|(host, _, _, _)| host == hostname)
}

fn assert_skipped_after(run: &HarnessRun, hostname: &str, failed: &str) {
    let host = run.host(hostname);
    assert_eq!(
        host.failed_at_step,
        Some(PipelineStep::SerialGroup),
        "{:?}",
        host
    );
    assert_eq!(
        host.error_message.as_deref(),
        Some(format!("serial group 'kubernetes' halted after {} failed", failed).as_str())
    );
    assert!(
        !ran_hooks(run, hostname),
        "{} ran hooks: {:?}",
        hostname,
        run.executed_hooks
    );
}

#[tokio::test(start_paused = true)]
async fn test_same_group_deploys_never_overlap() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(hooks_taking(Duration::from_millis(30)));

    let run = harness
        .run(vec![
            node("pomegranates", Some("kubernetes")),
            node("pufferfish", Some("kubernetes")),
            node("seahorse", Some("kubernetes")),
        ])
        .await;

    assert!(run.results.all_succeeded(), "{:?}", run.results);
    let spans: Vec<_> = ["pomegranates", "pufferfish", "seahorse"]
        .iter()
        .map(|host| deploy_span(&run, host))
        .collect();
    for (i, a) in spans.iter().enumerate() {
        for b in &spans[i + 1..] {
            assert!(!overlap(a, b), "deploys overlap: {:?}", run.events);
        }
    }
    assert!(
        run.has_event_starting_with("serial_wait:"),
        "{:?}",
        run.events
    );
}

#[tokio::test(start_paused = true)]
async fn test_different_groups_and_ungrouped_hosts_overlap() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(hooks_taking(Duration::from_millis(30)));

    let run = harness
        .run(vec![
            node("pomegranates", Some("kubernetes")),
            node("alpha", Some("consul")),
            node("hedgehog", None),
        ])
        .await;

    assert!(run.results.all_succeeded());
    let kubernetes = deploy_span(&run, "pomegranates");
    assert!(
        overlap(&kubernetes, &deploy_span(&run, "alpha")),
        "{:?}",
        run.events
    );
    assert!(
        overlap(&kubernetes, &deploy_span(&run, "hedgehog")),
        "{:?}",
        run.events
    );
    assert!(!run.has_event_starting_with("serial_wait:"));
}

#[tokio::test(start_paused = true)]
async fn test_unchanged_host_never_takes_the_group_slot() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(hooks_taking(Duration::from_millis(30)));
    harness.deployer = MockDeployBackend::new().with_host_systems(HashMap::from([(
        "nightfall.example.com".to_string(),
        HostSystems::both(mock_built_system("nightfall")),
    )]));

    let run = harness
        .run(vec![
            node("nightfall", Some("kubernetes")),
            node("pomegranates", Some("kubernetes")),
        ])
        .await;

    assert!(run.host("nightfall").unchanged);
    assert!(run.results.all_succeeded());
    assert!(
        !run.has_event_starting_with("serial_wait:"),
        "{:?}",
        run.events
    );
}

#[tokio::test(start_paused = true)]
async fn test_failed_deploy_skips_the_rest_of_its_group_only() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(
        hooks_taking(Duration::from_millis(30))
            .with_local_failures(vec!["--simulate-failure".to_string()]),
    );
    harness.nix_backend = builds_delayed(&["pufferfish", "seahorse"], Duration::from_millis(20));

    let run = harness
        .run(vec![
            node_with_failing_drain("pomegranates", Some("kubernetes")),
            node("pufferfish", Some("kubernetes")),
            node("seahorse", Some("kubernetes")),
            node("hedgehog", None),
        ])
        .await;

    let failed = run.host("pomegranates");
    assert_eq!(failed.failed_at_step, Some(PipelineStep::PreDeploy));
    assert!(run.has_event("hooks_complete:pomegranates:on-failure:ok"));
    assert_skipped_after(&run, "pufferfish", "pomegranates");
    assert_skipped_after(&run, "seahorse", "pomegranates");
    assert!(run.host("hedgehog").success);
    assert_done_with_final_results(
        &run,
        &["hedgehog", "pomegranates", "pufferfish", "seahorse"],
    );
}

#[tokio::test(start_paused = true)]
async fn test_failure_before_deploy_halts_group_before_on_failure_hooks_finish() {
    let mut harness = DeployHarness::new();
    // on-failure hooks of the failed build outlast the other members' builds,
    // so they would start deploying if the halt waited for those hooks
    harness.hook_backend = Arc::new(hooks_taking(Duration::from_millis(80)));
    harness.nix_backend = builds_delayed(&["pufferfish", "seahorse"], Duration::from_millis(20))
        .with_build_failures(vec!["pomegranates".to_string()]);

    let run = harness
        .run(vec![
            node("pomegranates", Some("kubernetes")),
            node("pufferfish", Some("kubernetes")),
            node("seahorse", Some("kubernetes")),
        ])
        .await;

    assert_eq!(
        run.host("pomegranates").failed_at_step,
        Some(PipelineStep::Build)
    );
    assert_skipped_after(&run, "pufferfish", "pomegranates");
    assert_skipped_after(&run, "seahorse", "pomegranates");
}

#[tokio::test]
async fn test_waiting_for_the_group_does_not_consume_hook_timeouts() {
    use crate::backends::hook::SystemHookBackend;
    use crate::hooks::{HookConfig, HookLocation};

    let mut harness = DeployHarness::new();
    harness.real_hook_backend = Some(Arc::new(SystemHookBackend::new(
        None,
        crate::process::OutputMode::Capture,
    )));
    let host = |name: &str, drain: &str, timeout: u64| {
        let mut hook = HookConfig::test_fixture(
            "drain-node",
            HookStage::PreDeploy,
            HookLocation::Local,
            drain,
        );
        hook.timeout = timeout;
        let mut host = make_host_metadata(name, Some("p2"), RebootPolicy::Auto);
        host.hooks = vec![hook];
        host.serial_group = Some("kubernetes".to_string());
        host
    };
    harness.nix_backend = builds_delayed(&["pufferfish"], Duration::from_millis(500));

    let run = harness
        .run(vec![
            host("pomegranates", "sleep 1.5", 10),
            host("pufferfish", "sleep 0.1", 1),
        ])
        .await;

    // pufferfish queues ~1.5s behind pomegranates, past its own 1s timeout,
    // so it only succeeds if the timeout starts when its hook does
    assert!(run.results.all_succeeded(), "{:?}", run.results);
    assert!(
        run.has_event_starting_with("serial_wait:pufferfish:"),
        "{:?}",
        run.events
    );
}
