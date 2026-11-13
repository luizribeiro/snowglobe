//! What a deployment cut short by Ctrl-C leaves behind on its hosts. snow
//! exits without waiting for them, so it says which ones need a redeploy.

use super::board::plural;
use parking_lot::Mutex;
use snowlib::{ActivationMode, DeployObserver, HookStage, HostEvent, PipelineStep};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Default)]
struct HostProgress {
    /// Hook stages run only when they have hooks, so a stage the host lacks
    /// never starts and must not be awaited
    configured_hooks: HashSet<HookStage>,
    started_hooks: HashSet<HookStage>,
    running_hooks: Option<HookStage>,
    step: Option<PipelineStep>,
    failed: bool,
}

impl HostProgress {
    /// None while nothing has changed on the host yet: building, copying and
    /// planning leave it as it was
    fn left_behind(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(stage) = self.running_hooks {
            parts.push(format!("{stage} hooks interrupted"));
        } else if let Some(step) = self.step.and_then(still_running) {
            parts.push(step);
        }
        if let Some(stage) = self.undoing_stage_not_run() {
            parts.push(format!("pre-deploy hooks ran, {stage} hooks didn't"));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    /// The host's hooks that would undo what its pre-deploy hooks did, such
    /// as undraining it, when they never started
    fn undoing_stage_not_run(&self) -> Option<HookStage> {
        let undoing = if self.failed {
            HookStage::OnFailure
        } else {
            HookStage::PostDeploy
        };
        let pre_deploy_done = self.started_hooks.contains(&HookStage::PreDeploy)
            && self.running_hooks != Some(HookStage::PreDeploy);
        (pre_deploy_done
            && self.configured_hooks.contains(&undoing)
            && !self.started_hooks.contains(&undoing))
        .then_some(undoing)
    }
}

/// What carries on without snow; a dry run's activation changes nothing
fn still_running(step: PipelineStep) -> Option<String> {
    match step {
        PipelineStep::Activate(mode) if mode != ActivationMode::DryActivate => {
            Some(format!("{step} (still running on the host)"))
        }
        PipelineStep::Reboot => Some("rebooting".to_string()),
        _ => None,
    }
}

#[derive(Clone)]
pub(super) struct LeftBehind {
    hosts: Arc<Mutex<BTreeMap<String, HostProgress>>>,
    redeploy: Arc<[String]>,
}

impl LeftBehind {
    /// `redeploy`: the command that finishes the hosts, up to their names
    pub fn new(redeploy: Vec<String>) -> Self {
        Self {
            hosts: Arc::default(),
            redeploy: redeploy.into(),
        }
    }

    pub fn has_hooks(&self, hostname: &str, stages: impl IntoIterator<Item = HookStage>) {
        self.update(hostname, |host| host.configured_hooks.extend(stages));
    }

    pub fn report(&self) -> Vec<String> {
        report(&self.hosts.lock(), &self.redeploy)
    }

    fn update(&self, hostname: &str, change: impl FnOnce(&mut HostProgress)) {
        change(self.hosts.lock().entry(hostname.to_string()).or_default());
    }
}

impl DeployObserver for LeftBehind {
    fn on_host_event(&self, hostname: &str, event: &HostEvent) {
        match event {
            HostEvent::Done(_) => {
                self.hosts.lock().remove(hostname);
            }
            HostEvent::Started(step) => self.update(hostname, |host| host.step = Some(*step)),
            HostEvent::Finished(_) => self.update(hostname, |host| host.step = None),
            HostEvent::Failed { .. } => self.update(hostname, |host| {
                host.step = None;
                host.failed = true;
            }),
            HostEvent::Unchanged | HostEvent::WaitingForSerialGroup { .. } => {}
        }
    }

    fn on_hooks_start(&self, hostname: &str, stage: HookStage) {
        self.update(hostname, |host| {
            host.running_hooks = Some(stage);
            host.started_hooks.insert(stage);
        });
    }

    fn on_hooks_complete(&self, hostname: &str, _stage: HookStage, _success: bool) {
        self.update(hostname, |host| host.running_hooks = None);
    }
}

/// ```text
/// Interrupted. Hosts left mid-deploy:
///   web1  switch (still running on the host), pre-deploy hooks ran, post-deploy hooks didn't
///   web2  pre-deploy hooks ran, post-deploy hooks didn't
/// To finish them: snow deploy --redeploy web1 web2
/// ```
fn report(hosts: &BTreeMap<String, HostProgress>, redeploy: &[String]) -> Vec<String> {
    let left: Vec<(&str, String)> = hosts
        .iter()
        .filter_map(|(hostname, host)| Some((hostname.as_str(), host.left_behind()?)))
        .collect();
    if left.is_empty() {
        return vec!["Interrupted.".to_string()];
    }

    let width = left
        .iter()
        .map(|(hostname, _)| hostname.len())
        .max()
        .unwrap_or_default();
    let command = snowlib::shell::join(
        redeploy
            .iter()
            .map(String::as_str)
            .chain(left.iter().map(|(hostname, _)| *hostname)),
    );
    let mut lines = vec![format!(
        "Interrupted. {} left mid-deploy:",
        plural(left.len(), "Host")
    )];
    lines.extend(
        left.iter()
            .map(|(hostname, what)| format!("  {hostname:<width$}  {what}")),
    );
    lines.push(format!(
        "To finish {}: {command}",
        if left.len() == 1 { "it" } else { "them" },
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use PipelineStep::*;

    const SWITCH: PipelineStep = Activate(ActivationMode::Switch);
    const DRAINED: [HookStage; 3] = [
        HookStage::PreDeploy,
        HookStage::PostDeploy,
        HookStage::OnFailure,
    ];

    fn left_behind() -> LeftBehind {
        LeftBehind::new(words(&["snow", "deploy", "--redeploy"]))
    }

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    fn start(left: &LeftBehind, hostname: &str, step: PipelineStep) {
        left.on_host_event(hostname, &HostEvent::Started(step));
    }

    fn finish(left: &LeftBehind, hostname: &str, step: PipelineStep) {
        left.on_host_event(hostname, &HostEvent::Finished(step));
    }

    fn prepare(left: &LeftBehind, hostname: &str) {
        for step in [Build, Check, Copy, Plan] {
            start(left, hostname, step);
            finish(left, hostname, step);
        }
    }

    fn run_hooks(left: &LeftBehind, hostname: &str, stage: HookStage) {
        left.on_hooks_start(hostname, stage);
        left.on_hooks_complete(hostname, stage, true);
    }

    /// Prepared, with its pre-deploy hooks run
    fn drained(left: &LeftBehind, hostname: &str, stages: &[HookStage]) {
        left.has_hooks(hostname, stages.iter().copied());
        prepare(left, hostname);
        run_hooks(left, hostname, HookStage::PreDeploy);
    }

    fn listed(left: &LeftBehind) -> Vec<String> {
        let report = left.report();
        report[1..report.len() - 1].to_vec()
    }

    #[test]
    fn test_nothing_started_says_only_interrupted() {
        assert_eq!(left_behind().report(), vec!["Interrupted."]);
    }

    #[test]
    fn test_host_building_copying_or_planning_is_not_listed() {
        let left = left_behind();
        start(&left, "cactus", Build);
        prepare(&left, "raven");
        start(&left, "raven", Copy);
        prepare(&left, "spruce");
        start(&left, "spruce", Plan);

        assert_eq!(left.report(), vec!["Interrupted."]);
    }

    #[test]
    fn test_host_mid_switch_is_listed_with_its_mode() {
        let left = left_behind();
        prepare(&left, "web1");
        start(&left, "web1", SWITCH);

        assert_eq!(
            left.report(),
            vec![
                "Interrupted. Host left mid-deploy:",
                "  web1  switch (still running on the host)",
                "To finish it: snow deploy --redeploy web1",
            ]
        );
    }

    #[test]
    fn test_rebooting_host_is_listed() {
        let left = left_behind();
        prepare(&left, "web1");
        start(&left, "web1", SWITCH);
        finish(&left, "web1", SWITCH);
        start(&left, "web1", Reboot);

        assert_eq!(listed(&left), ["  web1  rebooting"]);
    }

    #[test]
    fn test_redeploy_command_is_quoted_for_the_shell() {
        let left = LeftBehind::new(words(&[
            "snow",
            "-f",
            "/srv/my flake",
            "deploy",
            "--redeploy",
            "--boot",
        ]));
        prepare(&left, "web1");
        start(&left, "web1", Activate(ActivationMode::Boot));

        assert_eq!(
            left.report()[2],
            "To finish it: snow -f '/srv/my flake' deploy --redeploy --boot web1"
        );
    }

    #[test]
    fn test_drained_host_mid_boot_says_its_post_deploy_hooks_did_not_run() {
        let left = left_behind();
        drained(&left, "web1", &DRAINED);
        start(&left, "web1", Activate(ActivationMode::Boot));

        assert_eq!(
            listed(&left),
            ["  web1  boot (still running on the host), pre-deploy hooks ran, post-deploy hooks didn't"]
        );
    }

    #[test]
    fn test_drained_host_interrupted_before_activation_is_listed() {
        let left = left_behind();
        drained(&left, "web2", &DRAINED);

        assert_eq!(
            listed(&left),
            ["  web2  pre-deploy hooks ran, post-deploy hooks didn't"]
        );
    }

    #[test]
    fn test_host_with_only_pre_deploy_hooks_mid_switch_is_listed_for_the_switch() {
        let left = left_behind();
        drained(&left, "web1", &[HookStage::PreDeploy]);
        start(&left, "web1", SWITCH);

        assert_eq!(
            listed(&left),
            ["  web1  switch (still running on the host)"]
        );
    }

    #[test]
    fn test_host_with_only_pre_deploy_hooks_after_its_switch_is_not_listed() {
        let left = left_behind();
        drained(&left, "web1", &[HookStage::PreDeploy]);
        start(&left, "web1", SWITCH);
        finish(&left, "web1", SWITCH);

        assert_eq!(left.report(), vec!["Interrupted."]);
    }

    /// No hook of its own would undo its pre-deploy hooks, and a plain
    /// re-run deploys it, as it doesn't run the new system yet
    #[test]
    fn test_host_with_only_pre_deploy_hooks_before_activation_is_not_listed() {
        let left = left_behind();
        drained(&left, "web1", &[HookStage::PreDeploy]);

        assert_eq!(left.report(), vec!["Interrupted."]);
    }

    #[test]
    fn test_pre_deploy_hooks_cut_short_are_named() {
        let left = left_behind();
        left.has_hooks("web1", DRAINED);
        prepare(&left, "web1");
        left.on_hooks_start("web1", HookStage::PreDeploy);

        assert_eq!(listed(&left), ["  web1  pre-deploy hooks interrupted"]);
    }

    #[test]
    fn test_post_deploy_hooks_cut_short_are_named() {
        let left = left_behind();
        drained(&left, "web1", &DRAINED);
        start(&left, "web1", SWITCH);
        finish(&left, "web1", SWITCH);
        left.on_hooks_start("web1", HookStage::PostDeploy);

        assert_eq!(listed(&left), ["  web1  post-deploy hooks interrupted"]);
    }

    #[test]
    fn test_post_reboot_hooks_cut_short_are_named_with_what_is_still_undone() {
        let left = left_behind();
        drained(&left, "web1", &DRAINED);
        start(&left, "web1", SWITCH);
        finish(&left, "web1", SWITCH);
        start(&left, "web1", Reboot);
        finish(&left, "web1", Reboot);
        left.on_hooks_start("web1", HookStage::PostReboot);

        assert_eq!(
            listed(&left),
            ["  web1  post-reboot hooks interrupted, pre-deploy hooks ran, post-deploy hooks didn't"]
        );
    }

    #[test]
    fn test_waiting_or_unchanged_host_is_not_listed() {
        let left = left_behind();
        start(&left, "web1", Build);
        finish(&left, "web1", Build);
        start(&left, "web1", Check);
        finish(&left, "web1", Check);
        left.on_host_event("web1", &HostEvent::Unchanged);
        prepare(&left, "web2");
        left.on_host_event(
            "web2",
            &HostEvent::WaitingForSerialGroup {
                group: "db".to_string(),
                holder: Some("web3".to_string()),
            },
        );

        assert_eq!(left.report(), vec!["Interrupted."]);
    }

    fn fail_switch(left: &LeftBehind, hostname: &str) {
        start(left, hostname, SWITCH);
        left.on_host_event(
            hostname,
            &HostEvent::Failed {
                step: SWITCH,
                reason: "activation failed".to_string(),
            },
        );
    }

    #[test]
    fn test_failed_drained_host_awaiting_its_on_failure_hooks_is_listed() {
        let left = left_behind();
        drained(&left, "web1", &DRAINED);
        fail_switch(&left, "web1");

        assert_eq!(
            listed(&left),
            ["  web1  pre-deploy hooks ran, on-failure hooks didn't"]
        );
    }

    #[test]
    fn test_failed_host_whose_on_failure_hooks_ran_is_not_listed() {
        let left = left_behind();
        drained(&left, "web1", &DRAINED);
        fail_switch(&left, "web1");
        run_hooks(&left, "web1", HookStage::OnFailure);

        assert_eq!(left.report(), vec!["Interrupted."]);
    }

    #[test]
    fn test_finished_host_is_not_listed() {
        let left = left_behind();
        drained(&left, "web1", &DRAINED);
        start(&left, "web1", SWITCH);
        finish(&left, "web1", SWITCH);
        run_hooks(&left, "web1", HookStage::PostDeploy);
        left.on_host_event(
            "web1",
            &HostEvent::Done(snowlib::HostDeploymentResult::unchanged("web1")),
        );

        assert_eq!(left.report(), vec!["Interrupted."]);
    }

    #[test]
    fn test_dry_run_leaves_nothing_behind() {
        let left = left_behind();
        prepare(&left, "web1");
        start(&left, "web1", Activate(ActivationMode::DryActivate));

        assert_eq!(left.report(), vec!["Interrupted."]);
    }

    /// What the report says each time a host finishes its activation
    #[derive(Clone)]
    struct ReportAfterActivation {
        left: LeftBehind,
        reports: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl DeployObserver for ReportAfterActivation {
        fn on_host_event(&self, _hostname: &str, event: &HostEvent) {
            if let HostEvent::Finished(Activate(_)) = event {
                self.reports.lock().push(self.left.report());
            }
        }
    }

    #[tokio::test]
    async fn test_a_deploy_feeds_the_report_until_its_host_is_done() {
        use snowlib::NixBackend;
        let flake =
            snowlib::Flake::new(Some(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")))
                .unwrap();
        let nix_backend = snowlib::MockNixBackend::new();
        let mut host = nix_backend
            .get_host_metadata(&flake, "web1", snowlib::SystemType::NixOS)
            .await
            .unwrap();
        host.hooks = serde_json::from_value(serde_json::json!([
            {"name": "drain", "stage": "pre-deploy", "location": "remote", "command": "touch /root/drained"},
            {"name": "undrain", "stage": "post-deploy", "location": "remote", "command": "rm /root/drained"},
        ]))
        .unwrap();
        let left = left_behind();
        left.has_hooks("web1", host.hooks.iter().map(|hook| hook.stage));
        let probe = ReportAfterActivation {
            left: left.clone(),
            reports: Arc::default(),
        };
        let observer = snowlib::observer::CompositeObserver::new(vec![
            Box::new(left.clone()),
            Box::new(probe.clone()),
        ]);
        let hook_executor = snowlib::HookExecutor::new(
            Arc::new(snowlib::MockHookBackend::new()),
            [("web1".to_string(), host.hooks.clone())].into(),
        );
        let config = snowlib::DeployConfiguration {
            strategy: snowlib::DeployStrategy::Switch,
            output_mode: snowlib::OutputMode::Capture,
            ..Default::default()
        };
        let deploy_backend = snowlib::MockDeployBackend::new();
        let reboot_backend = snowlib::MockRebootBackend::new();
        let deploy_ctx = snowlib::DeployContext::new(
            &flake,
            &nix_backend,
            &deploy_backend,
            &reboot_backend,
            &config,
            &observer,
            Some(&hook_executor),
        );

        let results = snowlib::deploy(&deploy_ctx, vec![host]).await.unwrap();

        assert!(results.all_succeeded());
        assert_eq!(
            *probe.reports.lock(),
            [[
                "Interrupted. Host left mid-deploy:",
                "  web1  pre-deploy hooks ran, post-deploy hooks didn't",
                "To finish it: snow deploy --redeploy web1",
            ]]
        );
        assert_eq!(left.report(), ["Interrupted."]);
    }

    #[test]
    fn test_several_hosts_are_listed_in_name_order_and_aligned() {
        let left = left_behind();
        prepare(&left, "web10");
        start(&left, "web10", Reboot);
        start(&left, "db", Build);
        drained(&left, "cache", &DRAINED);
        prepare(&left, "app");
        start(&left, "app", SWITCH);

        assert_eq!(
            left.report(),
            vec![
                "Interrupted. Hosts left mid-deploy:",
                "  app    switch (still running on the host)",
                "  cache  pre-deploy hooks ran, post-deploy hooks didn't",
                "  web10  rebooting",
                "To finish them: snow deploy --redeploy app cache web10",
            ]
        );
    }
}
