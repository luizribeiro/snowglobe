//! Tests for the per-host events the pipeline reports to observers

use super::*;
use crate::backends::deploy::HostSystems;
use crate::backends::nix::MockNixBackend;
use crate::backends::reboot::{MockRebootBackend, RebootReason};

/// Sets the harness up so one step fails
type BreakStep = fn(&mut PushHarness);

fn failed_events(run: &HarnessRun) -> Vec<&String> {
    run.events
        .iter()
        .filter(|e| e.starts_with("failed:"))
        .collect()
}

#[tokio::test]
async fn test_deploy_reports_each_step_in_order() {
    let mut harness = PushHarness::new();
    let mut host = kubernetes_node("samarium");
    host.reboot_policy = RebootPolicy::Always;
    harness.config.strategy = PushStrategy::Boot;

    let run = harness.run(vec![host]).await;

    assert!(failed_events(&run).is_empty(), "{:?}", run.events);
    let mut previous = 0;
    for step in ["build", "check", "copy", "plan", "activate", "reboot"] {
        let started = position(&run, &format!("started:samarium:{step}"));
        let finished = position(&run, &format!("finished:samarium:{step}"));
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
            h.nix_backend = MockNixBackend::new().with_build_failures(vec!["cerium".into()])
        }),
        ("check", |h| {
            h.deployer = MockDeployBackend::new()
                .with_host_systems_timeouts(vec!["cerium.example.com".into()])
        }),
        ("copy", |h| {
            h.deployer =
                MockDeployBackend::new().with_copy_failures(vec!["cerium.example.com".into()])
        }),
        ("activate", |h| {
            h.deployer =
                MockDeployBackend::new().with_activate_failures(vec!["cerium.example.com".into()])
        }),
    ];

    for (step, configure) in cases {
        let mut harness = PushHarness::new();
        configure(&mut harness);

        let run = harness.run(vec![kubernetes_node("cerium")]).await;

        let reason = run.host("cerium").error_message.clone().unwrap();
        assert_eq!(
            failed_events(&run),
            vec![&format!("failed:cerium:{step}:{reason}")],
            "{step}"
        );
        assert!(run.has_event(&format!("started:cerium:{step}")), "{step}");
        assert!(!run.has_event(&format!("finished:cerium:{step}")), "{step}");
        assert!(
            position(&run, &format!("failed:cerium:{step}:{reason}"))
                < position(&run, "hooks_start:cerium:OnFailure"),
            "{step}: {:?}",
            run.events
        );
    }
}

#[tokio::test]
async fn test_blocked_host_reports_why_it_was_blocked() {
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

    let reason = run.host("neodymium").error_message.clone().unwrap();
    assert_eq!(
        failed_events(&run),
        vec![&format!("failed:neodymium:plan:{reason}")]
    );
}

#[tokio::test]
async fn test_hook_failures_are_not_reported_as_failed_steps() {
    let mut harness = PushHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_local_failures(vec!["kubectl drain ${SNOW_HOSTNAME}".to_string()]),
    );

    let run = harness.run(vec![kubernetes_node("vanadium")]).await;

    assert_eq!(
        run.host("vanadium").failed_at_step,
        Some(PipelineStep::Activate)
    );
    assert!(failed_events(&run).is_empty(), "{:?}", run.events);
    assert!(!run.has_event_starting_with("activation_complete"));
}

#[tokio::test]
async fn test_failed_post_reboot_hook_leaves_the_reboot_finished() {
    let mut harness = PushHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new().with_local_failures(vec![
            "kubectl wait --for=condition=Ready node/${SNOW_HOSTNAME}".to_string(),
        ]),
    );
    let mut host = kubernetes_node("lanthanum");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    assert_eq!(
        run.host("lanthanum").failed_at_step,
        Some(PipelineStep::Reboot)
    );
    assert!(run.has_event("reboot_complete:lanthanum:ok"));
    assert!(!run.has_event("reboot_complete:lanthanum:fail"));
    assert!(failed_events(&run).is_empty(), "{:?}", run.events);
}

#[tokio::test]
async fn test_failed_reboot_reports_the_reboot_error_itself() {
    let mut harness = PushHarness::new();
    harness.reboot_backend =
        MockRebootBackend::new().with_reboot_failures(vec!["boron.example.com".to_string()]);
    let mut host = kubernetes_node("boron");
    host.reboot_policy = RebootPolicy::Always;

    let run = harness.run(vec![host]).await;

    let failures = failed_events(&run);
    assert_eq!(failures.len(), 1, "{:?}", run.events);
    let reason = failures[0]
        .strip_prefix("failed:boron:reboot:")
        .unwrap_or_else(|| panic!("not a reboot failure: {}", failures[0]));
    assert_eq!(
        run.host("boron").error_message.as_deref(),
        Some(format!("Deployment succeeded but reboot failed: {reason}").as_str())
    );
    assert!(run.has_event("started:boron:reboot"));
    assert!(!run.has_event("finished:boron:reboot"));
}

#[tokio::test]
async fn test_unchanged_host_stops_after_its_check() {
    let mut harness = PushHarness::new();
    harness.deployer = MockDeployBackend::new().with_host_systems(HashMap::from([(
        "sodium.example.com".to_string(),
        HostSystems::both(mock_built_system("sodium")),
    )]));

    let run = harness.run(vec![kubernetes_node("sodium")]).await;

    assert!(run.host("sodium").unchanged);
    assert!(position(&run, "finished:sodium:check") < position(&run, "unchanged:sodium"));
    assert!(!run.has_event("started:sodium:copy"), "{:?}", run.events);
}

#[tokio::test]
async fn test_redeploy_skips_the_check_step() {
    let mut harness = PushHarness::new();
    harness.config.redeploy = true;

    let run = harness.run(vec![kubernetes_node("sodium")]).await;

    assert!(run.host("sodium").success);
    assert!(!run.has_event_starting_with("started:sodium:check"));
    assert!(run.has_event("finished:sodium:copy"));
}

#[tokio::test]
async fn test_failed_post_deploy_hook_is_not_a_failed_step() {
    let mut harness = PushHarness::new();
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_local_failures(vec!["kubectl uncordon ${SNOW_HOSTNAME}".to_string()]),
    );

    let run = harness.run(vec![kubernetes_node("radon")]).await;

    assert_eq!(
        run.host("radon").failed_at_step,
        Some(PipelineStep::PostDeploy)
    );
    assert!(run.has_event("finished:radon:activate"));
    assert!(failed_events(&run).is_empty(), "{:?}", run.events);
}

#[tokio::test]
async fn test_done_carries_each_hosts_final_result() {
    let mut harness = PushHarness::new();
    harness.deployer = MockDeployBackend::new()
        .with_copy_failures(vec!["cerium.example.com".to_string()])
        .with_host_systems(HashMap::from([(
            "sodium.example.com".to_string(),
            HostSystems::both(mock_built_system("sodium")),
        )]));
    harness.reboot_backend =
        MockRebootBackend::new().with_reboot_failures(vec!["boron.example.com".to_string()]);
    harness.hook_backend = Arc::new(
        crate::backends::hook::MockHookBackend::new()
            .with_remote_failures(vec!["vanadium.example.com".to_string()]),
    );
    let mut boron = kubernetes_node("boron");
    boron.reboot_policy = RebootPolicy::Always;
    let mut vanadium = kubernetes_node("vanadium");
    vanadium.hooks.push(crate::hooks::HookConfig::test_fixture(
        "check-disks",
        HookStage::PreDeploy,
        crate::hooks::HookLocation::Remote,
        "smartctl -H /dev/sda",
    ));

    let run = harness
        .run(vec![
            boron,
            kubernetes_node("cerium"),
            kubernetes_node("radon"),
            kubernetes_node("sodium"),
            vanadium,
        ])
        .await;

    assert!(run
        .host("boron")
        .error_message
        .as_deref()
        .is_some_and(|m| m.starts_with("Deployment succeeded but reboot failed")));
    assert!(run.host("sodium").unchanged);
    assert_eq!(
        run.host("vanadium").failed_at_step,
        Some(PipelineStep::Activate)
    );
    assert_done_with_final_results(&run, &["boron", "cerium", "radon", "sodium", "vanadium"]);
}
