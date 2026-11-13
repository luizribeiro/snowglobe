//! Tests for the per-host events the pipeline reports to observers

use super::*;
use crate::backends::deploy::HostSystems;
use crate::backends::nix::MockNixBackend;
use crate::backends::reboot::{MockRebootBackend, RebootReason};

/// Sets the harness up so one step fails
type BreakStep = fn(&mut DeployHarness);

fn failed_events(run: &HarnessRun) -> Vec<&String> {
    run.events
        .iter()
        .filter(|e| e.starts_with("failed:"))
        .collect()
}

#[tokio::test]
async fn test_deploy_reports_each_step_in_order() {
    let harness = DeployHarness::new();
    let mut host = kubernetes_node("seahorse");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    assert!(failed_events(&run).is_empty(), "{:?}", run.events);
    let mut previous = 0;
    for step in ["build", "check", "copy", "plan", "boot", "reboot"] {
        let started = position(&run, &format!("started:seahorse:{step}"));
        let finished = position(&run, &format!("finished:seahorse:{step}"));
        assert!(
            previous <= started && started < finished,
            "{step}: {:?}",
            run.events
        );
        previous = finished;
    }
}

#[tokio::test]
async fn test_failed_step_reports_its_reason_and_never_finishes() {
    let cases: Vec<(&str, BreakStep)> = vec![
        ("build", |h| {
            h.nix_backend = MockNixBackend::new().with_build_failures(vec!["cactus".into()])
        }),
        ("check", |h| {
            h.deployer = MockDeployBackend::new()
                .with_host_systems_timeouts(vec!["cactus.example.com".into()])
        }),
        ("copy", |h| {
            h.deployer =
                MockDeployBackend::new().with_copy_failures(vec!["cactus.example.com".into()])
        }),
        ("switch", |h| {
            h.deployer =
                MockDeployBackend::new().with_activate_failures(vec!["cactus.example.com".into()])
        }),
    ];

    for (step, configure) in cases {
        let mut harness = DeployHarness::new();
        configure(&mut harness);

        let run = harness.run(vec![kubernetes_node("cactus")]).await;

        let reason = run.host("cactus").error_message.clone().unwrap();
        assert_eq!(
            failed_events(&run),
            vec![&format!("failed:cactus:{step}:{reason}")],
            "{step}"
        );
        assert!(run.has_event(&format!("started:cactus:{step}")), "{step}");
        assert!(!run.has_event(&format!("finished:cactus:{step}")), "{step}");
        assert!(
            position(&run, &format!("failed:cactus:{step}:{reason}"))
                < position(&run, "hooks_start:cactus:on-failure"),
            "{step}: {:?}",
            run.events
        );
    }
}

#[tokio::test]
async fn test_blocked_host_reports_why_it_was_blocked() {
    let mut harness = DeployHarness::new();
    harness.config.strategy = DeployStrategy::Switch;
    harness.reboot_backend = MockRebootBackend::new().with_reboot_required(HashMap::from([(
        "nightfall".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    )]));

    let run = harness.run(vec![kubernetes_node("nightfall")]).await;

    let reason = run.host("nightfall").error_message.clone().unwrap();
    assert_eq!(
        failed_events(&run),
        vec![&format!("failed:nightfall:plan:{reason}")]
    );
}

#[tokio::test]
async fn test_hook_failures_are_not_reported_as_failed_steps() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_local_failures(vec!["kubectl drain ${SNOW_HOSTNAME}".to_string()]),
    );

    let run = harness.run(vec![kubernetes_node("vineyard")]).await;

    assert_eq!(
        run.host("vineyard").failed_at_step,
        Some(PipelineStep::PreDeploy)
    );
    assert!(failed_events(&run).is_empty(), "{:?}", run.events);
    assert!(!run.has_event_starting_with("activation_complete"));
}

#[tokio::test]
async fn test_failed_post_reboot_hook_leaves_the_reboot_finished() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new().with_local_failures(vec![
            "kubectl wait --for=condition=Ready node/${SNOW_HOSTNAME}".to_string(),
        ]),
    );
    let mut host = kubernetes_node("limestone");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    assert_eq!(
        run.host("limestone").failed_at_step,
        Some(PipelineStep::Reboot)
    );
    assert!(run.has_event("reboot_complete:limestone:ok"));
    assert!(!run.has_event("reboot_complete:limestone:fail"));
    assert!(failed_events(&run).is_empty(), "{:?}", run.events);
}

#[tokio::test]
async fn test_failed_reboot_reports_the_reboot_error_itself() {
    let mut harness = DeployHarness::new();
    harness.reboot_backend =
        MockRebootBackend::new().with_reboot_failures(vec!["bravo.example.com".to_string()]);
    let mut host = kubernetes_node("bravo");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    let failures = failed_events(&run);
    assert_eq!(failures.len(), 1, "{:?}", run.events);
    let reason = failures[0]
        .strip_prefix("failed:bravo:reboot:")
        .unwrap_or_else(|| panic!("not a reboot failure: {}", failures[0]));
    assert_eq!(
        run.host("bravo").error_message.as_deref(),
        Some(format!("Deployment succeeded but reboot failed: {reason}").as_str())
    );
    assert!(run.has_event("started:bravo:reboot"));
    assert!(!run.has_event("finished:bravo:reboot"));
}

#[tokio::test]
async fn test_unchanged_host_stops_after_its_check() {
    let mut harness = DeployHarness::new();
    harness.deployer = MockDeployBackend::new().with_host_systems(HashMap::from([(
        "spruce.example.com".to_string(),
        HostSystems::both(mock_built_system("spruce")),
    )]));

    let run = harness.run(vec![kubernetes_node("spruce")]).await;

    assert!(run.host("spruce").unchanged);
    assert!(position(&run, "finished:spruce:check") < position(&run, "unchanged:spruce"));
    assert!(!run.has_event("started:spruce:copy"), "{:?}", run.events);
}

#[tokio::test]
async fn test_redeploy_skips_the_check_step() {
    let mut harness = DeployHarness::new();
    harness.config.redeploy = true;

    let run = harness.run(vec![kubernetes_node("spruce")]).await;

    assert!(run.host("spruce").success);
    assert!(!run.has_event_starting_with("started:spruce:check"));
    assert!(run.has_event("finished:spruce:copy"));
}

#[tokio::test]
async fn test_failed_post_deploy_hook_is_not_a_failed_step() {
    let mut harness = DeployHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_local_failures(vec!["kubectl uncordon ${SNOW_HOSTNAME}".to_string()]),
    );

    let run = harness.run(vec![kubernetes_node("raven")]).await;

    assert_eq!(
        run.host("raven").failed_at_step,
        Some(PipelineStep::PostDeploy)
    );
    assert!(run.has_event("finished:raven:switch"));
    assert!(failed_events(&run).is_empty(), "{:?}", run.events);
}

#[tokio::test]
async fn test_done_carries_each_hosts_final_result() {
    let mut harness = DeployHarness::new();
    harness.deployer = MockDeployBackend::new()
        .with_copy_failures(vec!["cactus.example.com".to_string()])
        .with_host_systems(HashMap::from([(
            "spruce.example.com".to_string(),
            HostSystems::both(mock_built_system("spruce")),
        )]));
    harness.reboot_backend =
        MockRebootBackend::new().with_reboot_failures(vec!["bravo.example.com".to_string()]);
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_remote_failures(vec!["vineyard.example.com".to_string()]),
    );
    let mut bravo = kubernetes_node("bravo");
    bravo.reboot_policy = RebootPolicy::Always;
    let mut vineyard = kubernetes_node("vineyard");
    vineyard.hooks.push(crate::hooks::HookConfig::test_fixture(
        "check-disks",
        HookStage::PreDeploy,
        crate::hooks::HookLocation::Remote,
        "smartctl -H /dev/sda",
    ));

    let run = harness
        .run(vec![
            bravo,
            kubernetes_node("cactus"),
            kubernetes_node("raven"),
            kubernetes_node("spruce"),
            vineyard,
        ])
        .await;

    assert!(run
        .host("bravo")
        .error_message
        .as_deref()
        .is_some_and(|m| m.starts_with("Deployment succeeded but reboot failed")));
    assert!(run.host("spruce").unchanged);
    assert_eq!(
        run.host("vineyard").failed_at_step,
        Some(PipelineStep::PreDeploy)
    );
    assert_done_with_final_results(&run, &["bravo", "cactus", "raven", "spruce", "vineyard"]);
}
