//! Tests for hosts in a serial group deploying one at a time within a phase

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
    find(format!("hooks_start:{}:PreDeploy", hostname))
        ..=find(format!("hooks_complete:{}:PostDeploy:ok", hostname))
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
        Some(
            format!(
                "skipped: serial group 'kubernetes' halted after {} failed",
                failed
            )
            .as_str()
        )
    );
    assert!(
        !ran_hooks(run, hostname),
        "{} ran hooks: {:?}",
        hostname,
        run.executed_hooks
    );
}

#[tokio::test]
async fn test_same_group_deploys_never_overlap() {
    let mut harness = PushHarness::new();
    harness.hook_backend = Arc::new(hooks_taking(Duration::from_millis(30)));

    let run = harness
        .run(vec![
            node("praseodymium", Some("kubernetes")),
            node("promethium", Some("kubernetes")),
            node("samarium", Some("kubernetes")),
        ])
        .await;

    assert!(run.results.all_succeeded(), "{:?}", run.results);
    let spans: Vec<_> = ["praseodymium", "promethium", "samarium"]
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

#[tokio::test]
async fn test_different_groups_and_ungrouped_hosts_overlap() {
    let mut harness = PushHarness::new();
    harness.hook_backend = Arc::new(hooks_taking(Duration::from_millis(30)));

    let run = harness
        .run(vec![
            node("praseodymium", Some("kubernetes")),
            node("argon", Some("consul")),
            node("hydrogen", None),
        ])
        .await;

    assert!(run.results.all_succeeded());
    let kubernetes = deploy_span(&run, "praseodymium");
    assert!(
        overlap(&kubernetes, &deploy_span(&run, "argon")),
        "{:?}",
        run.events
    );
    assert!(
        overlap(&kubernetes, &deploy_span(&run, "hydrogen")),
        "{:?}",
        run.events
    );
    assert!(!run.has_event_starting_with("serial_wait:"));
}

#[tokio::test]
async fn test_unchanged_host_never_takes_the_group_slot() {
    let mut harness = PushHarness::new();
    harness.hook_backend = Arc::new(hooks_taking(Duration::from_millis(30)));
    harness.deployer = MockDeployBackend::new().with_host_systems(HashMap::from([(
        "neodymium.example.com".to_string(),
        HostSystems::both(mock_built_system("neodymium")),
    )]));

    let run = harness
        .run(vec![
            node("neodymium", Some("kubernetes")),
            node("praseodymium", Some("kubernetes")),
        ])
        .await;

    assert!(run.host("neodymium").unchanged);
    assert!(run.results.all_succeeded());
    assert!(
        !run.has_event_starting_with("serial_wait:"),
        "{:?}",
        run.events
    );
}

#[tokio::test]
async fn test_failed_deploy_skips_the_rest_of_its_group_only() {
    let mut harness = PushHarness::new();
    harness.hook_backend = Arc::new(
        hooks_taking(Duration::from_millis(30))
            .with_local_failures(vec!["--simulate-failure".to_string()]),
    );
    harness.nix_backend = builds_delayed(&["promethium", "samarium"], Duration::from_millis(20));

    let run = harness
        .run(vec![
            node_with_failing_drain("praseodymium", Some("kubernetes")),
            node("promethium", Some("kubernetes")),
            node("samarium", Some("kubernetes")),
            node("hydrogen", None),
        ])
        .await;

    let failed = run.host("praseodymium");
    assert_eq!(failed.failed_at_step, Some(PipelineStep::PreDeploy));
    assert!(run.has_event("hooks_complete:praseodymium:OnFailure:ok"));
    assert_skipped_after(&run, "promethium", "praseodymium");
    assert_skipped_after(&run, "samarium", "praseodymium");
    assert!(run.host("hydrogen").success);
    assert_done_with_final_results(
        &run,
        &["hydrogen", "praseodymium", "promethium", "samarium"],
    );
}

#[tokio::test]
async fn test_failure_before_deploy_halts_group_before_on_failure_hooks_finish() {
    let mut harness = PushHarness::new();
    // on-failure hooks of the failed build outlast the other members' builds,
    // so they would start deploying if the halt waited for those hooks
    harness.hook_backend = Arc::new(hooks_taking(Duration::from_millis(80)));
    harness.nix_backend = builds_delayed(&["promethium", "samarium"], Duration::from_millis(20))
        .with_build_failures(vec!["praseodymium".to_string()]);

    let run = harness
        .run(vec![
            node("praseodymium", Some("kubernetes")),
            node("promethium", Some("kubernetes")),
            node("samarium", Some("kubernetes")),
        ])
        .await;

    assert_eq!(
        run.host("praseodymium").failed_at_step,
        Some(PipelineStep::Build)
    );
    assert_skipped_after(&run, "promethium", "praseodymium");
    assert_skipped_after(&run, "samarium", "praseodymium");
}

#[tokio::test]
async fn test_waiting_for_the_group_does_not_consume_hook_timeouts() {
    use crate::backends::hook::SystemHookBackend;
    use crate::hooks::{HookConfig, HookLocation};

    let mut harness = PushHarness::new();
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
    harness.nix_backend = builds_delayed(&["promethium"], Duration::from_millis(500));

    let run = harness
        .run(vec![
            host("praseodymium", "sleep 1.5", 10),
            host("promethium", "sleep 0.1", 1),
        ])
        .await;

    // promethium queues ~1.5s behind praseodymium, past its own 1s timeout,
    // so it only succeeds if the timeout starts when its hook does
    assert!(run.results.all_succeeded(), "{:?}", run.results);
    assert!(
        run.has_event_starting_with("serial_wait:promethium:"),
        "{:?}",
        run.events
    );
}
