//! What the push TUI shows for a phase: the steps of a push as lanes, with
//! each host listed under the lane it is in, kept up to date from host events

use super::summary::success_labels;
use snowlib::error::summary::error_summary;
use snowlib::{HookStage, HostDeploymentResult, HostEvent, PipelineStep};
use std::time::{Duration, Instant};

/// How a row is drawn, in the order rows are listed
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RowKind {
    Failed,
    Active,
    Queued,
    Done,
    Unchanged,
}

/// The parts of a push a host moves through, in the order they are listed
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Lane {
    Queued,
    Build,
    Copy,
    Activate,
    Reboot,
    Done,
    Failed,
}

impl Lane {
    pub const ALL: [Lane; 7] = [
        Lane::Queued,
        Lane::Build,
        Lane::Copy,
        Lane::Activate,
        Lane::Reboot,
        Lane::Done,
        Lane::Failed,
    ];
}

/// A line of the board: a lane's heading, or a host listed under it
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Line {
    Lane { lane: Lane, hosts: usize },
    Host(Row),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub lane: Lane,
    pub kind: RowKind,
    pub hostname: String,
    pub step: String,
    pub detail: String,
    pub elapsed: Option<Duration>,
}

#[derive(Debug, Clone)]
enum State {
    Queued,
    Step(PipelineStep),
    Hook {
        stage: HookStage,
        name: String,
    },
    WaitingForSerialGroup {
        group: String,
        holder: Option<String>,
    },
    Failed {
        step: Option<PipelineStep>,
        reason: String,
    },
    Done(HostDeploymentResult),
}

#[derive(Debug)]
struct HostRow {
    hostname: String,
    state: State,
    since: Instant,
}

impl HostRow {
    fn is_finished(&self) -> bool {
        matches!(self.state, State::Failed { .. } | State::Done(_))
    }

    fn set(&mut self, state: State, now: Instant) {
        self.state = state;
        self.since = now;
    }
}

#[derive(Debug)]
pub(super) struct HostBoard {
    phase: String,
    started: Instant,
    hosts: Vec<HostRow>,
}

impl HostBoard {
    pub fn new(phase: &str, hostnames: &[String], now: Instant) -> Self {
        let mut hosts: Vec<HostRow> = hostnames
            .iter()
            .map(|hostname| HostRow {
                hostname: hostname.clone(),
                state: State::Queued,
                since: now,
            })
            .collect();
        hosts.sort_by(|a, b| a.hostname.cmp(&b.hostname));
        Self {
            phase: phase.to_string(),
            started: now,
            hosts,
        }
    }

    pub fn apply(&mut self, hostname: &str, event: &HostEvent, now: Instant) {
        let Some(host) = self.host_mut(hostname) else {
            return;
        };
        let state = match event {
            HostEvent::Started(step) => State::Step(*step),
            HostEvent::Failed { step, reason } => State::Failed {
                step: Some(*step),
                reason: reason.clone(),
            },
            HostEvent::WaitingForSerialGroup { group, holder } => State::WaitingForSerialGroup {
                group: group.clone(),
                holder: holder.clone(),
            },
            HostEvent::Done(result) if result.success => State::Done(result.clone()),
            HostEvent::Done(result) => final_failure(&host.state, result),
            HostEvent::Finished(_) | HostEvent::Unchanged => return,
        };
        let replaces_failure = matches!(state, State::Failed { .. });
        if replaces_failure || !host.is_finished() {
            host.set(state, now);
        }
    }

    /// On-failure hooks run after the host failed; its row keeps the failure
    pub fn hook_started(&mut self, hostname: &str, stage: HookStage, name: &str, now: Instant) {
        if let Some(host) = self.host_mut(hostname).filter(|host| !host.is_finished()) {
            host.set(
                State::Hook {
                    stage,
                    name: name.to_string(),
                },
                now,
            );
        }
    }

    /// Within a lane, hosts keep the name order `new` sorted them into
    pub fn rows(&self, now: Instant) -> Vec<Row> {
        let mut rows: Vec<Row> = self.hosts.iter().map(|host| self.row(host, now)).collect();
        rows.sort_by_key(|row| row.lane);
        rows
    }

    /// Every lane's heading, each followed by the hosts in it
    pub fn lines(&self, now: Instant) -> Vec<Line> {
        let rows = self.rows(now);
        Lane::ALL
            .iter()
            .flat_map(|&lane| {
                let hosts: Vec<Line> = rows
                    .iter()
                    .filter(|row| row.lane == lane)
                    .cloned()
                    .map(Line::Host)
                    .collect();
                std::iter::once(Line::Lane {
                    lane,
                    hosts: hosts.len(),
                })
                .chain(hosts)
            })
            .collect()
    }

    /// Fixed for the whole phase, since the TUI makes its lines once at the
    /// start: hosts only move between lanes
    pub fn line_count(&self) -> usize {
        Lane::ALL.len() + self.hosts.len()
    }

    /// Hosts that are done or failed, out of all of them
    pub fn progress(&self) -> (usize, usize) {
        let finished = self.hosts.iter().filter(|host| host.is_finished()).count();
        (finished, self.hosts.len())
    }

    pub fn title(&self) -> String {
        super::summary::phase_title(&self.phase)
    }

    pub fn any_failed(&self) -> bool {
        self.hosts
            .iter()
            .any(|host| kind_of(&host.state) == RowKind::Failed)
    }

    pub fn counts(&self) -> String {
        let count = |kinds: &[RowKind]| {
            self.hosts
                .iter()
                .filter(|host| kinds.contains(&kind_of(&host.state)))
                .count()
        };
        let mut parts = vec![format!(
            "{} {}",
            self.hosts.len(),
            plural(self.hosts.len(), "host")
        )];
        for (n, label) in [
            (count(&[RowKind::Done, RowKind::Unchanged]), "done"),
            (count(&[RowKind::Failed]), "failed"),
            (count(&[RowKind::Active]), "running"),
            (count(&[RowKind::Queued]), "queued"),
        ] {
            if n > 0 {
                parts.push(format!("{n} {label}"));
            }
        }
        parts.join(" · ")
    }

    pub fn hostname_width(&self) -> usize {
        self.hosts
            .iter()
            .map(|host| host.hostname.len())
            .max()
            .unwrap_or(0)
    }

    fn host_mut(&mut self, hostname: &str) -> Option<&mut HostRow> {
        self.hosts.iter_mut().find(|host| host.hostname == hostname)
    }

    fn row(&self, host: &HostRow, now: Instant) -> Row {
        let (step, detail) = describe(&host.state);
        let elapsed = match host.state {
            State::Failed { .. } => None,
            State::Done(_) => Some(host.since - self.started),
            _ => Some(now.saturating_duration_since(host.since)),
        };
        Row {
            lane: lane_of(&host.state),
            kind: kind_of(&host.state),
            hostname: host.hostname.clone(),
            step,
            detail,
            elapsed,
        }
    }
}

/// A failed result's step and reason, falling back to what the earlier
/// failure event said when the result lacks them
fn final_failure(previous: &State, result: &HostDeploymentResult) -> State {
    let (previous_step, previous_reason) = match previous {
        State::Failed { step, reason } => (*step, reason.as_str()),
        _ => (None, ""),
    };
    State::Failed {
        step: result.failed_at_step.or(previous_step),
        reason: result
            .error_message
            .clone()
            .filter(|reason| !reason.is_empty())
            .unwrap_or_else(|| previous_reason.to_string()),
    }
}

fn lane_of(state: &State) -> Lane {
    match state {
        State::Queued | State::WaitingForSerialGroup { .. } => Lane::Queued,
        State::Step(PipelineStep::Build | PipelineStep::Check) => Lane::Build,
        State::Step(PipelineStep::Copy) => Lane::Copy,
        State::Step(PipelineStep::Reboot)
        | State::Hook {
            stage: HookStage::PostReboot,
            ..
        } => Lane::Reboot,
        State::Step(_) | State::Hook { .. } => Lane::Activate,
        State::Done(_) => Lane::Done,
        State::Failed { .. } => Lane::Failed,
    }
}

fn kind_of(state: &State) -> RowKind {
    match state {
        State::Failed { .. } => RowKind::Failed,
        State::Step(_) | State::Hook { .. } => RowKind::Active,
        State::Queued | State::WaitingForSerialGroup { .. } => RowKind::Queued,
        State::Done(result) if result.unchanged => RowKind::Unchanged,
        State::Done(_) => RowKind::Done,
    }
}

fn describe(state: &State) -> (String, String) {
    match state {
        State::Queued => ("queued".into(), String::new()),
        State::Step(step) => (step.to_string(), String::new()),
        State::Hook { stage, name } => (stage.nix_name().into(), format!("hook '{name}'")),
        State::WaitingForSerialGroup { group, holder } => (
            "queued".into(),
            format!(
                "serial group '{group}' (held by {})",
                holder.as_deref().unwrap_or("another host")
            ),
        ),
        State::Failed {
            step: Some(PipelineStep::SerialGroup),
            reason,
        } => ("skipped".into(), reason.clone()),
        State::Failed { step, reason } => (
            step.map(|s| s.to_string())
                .unwrap_or_else(|| "failed".into()),
            error_summary(reason).to_string(),
        ),
        State::Done(result) if result.unchanged => ("unchanged".into(), String::new()),
        State::Done(result) => ("done".into(), success_labels(result).join(", ")),
    }
}

pub(super) fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

/// "3:07", or "1:02:07" past an hour
pub(super) fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    let (hours, minutes, seconds) = (secs / 3600, secs / 60 % 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowlib::ActivationMode;

    const P3: [&str; 4] = ["vanadium", "cerium", "lanthanum", "sodium"];

    fn board(t0: Instant) -> HostBoard {
        let hosts: Vec<String> = P3.iter().map(|h| h.to_string()).collect();
        HostBoard::new("p3", &hosts, t0)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn deployed(hostname: &str, mode: ActivationMode, rebooted: bool) -> HostDeploymentResult {
        HostDeploymentResult {
            hostname: hostname.to_string(),
            success: true,
            failed_at_step: None,
            activation_mode: Some(mode),
            rebooted,
            unchanged: false,
            error_message: None,
        }
    }

    fn row_of(board: &HostBoard, hostname: &str, now: Instant) -> Row {
        board
            .rows(now)
            .into_iter()
            .find(|row| row.hostname == hostname)
            .unwrap()
    }

    fn brief(row: &Row) -> (RowKind, &str, &str, &str) {
        (
            row.kind,
            row.hostname.as_str(),
            row.step.as_str(),
            row.detail.as_str(),
        )
    }

    fn summary(rows: &[Row]) -> Vec<(RowKind, &str, &str, &str)> {
        rows.iter().map(brief).collect()
    }

    #[test]
    fn test_new_board_lists_every_host_as_queued_in_name_order() {
        let t0 = Instant::now();
        let board = board(t0);

        let rows = board.rows(t0 + secs(5));

        assert_eq!(
            summary(&rows),
            vec![
                (RowKind::Queued, "cerium", "queued", ""),
                (RowKind::Queued, "lanthanum", "queued", ""),
                (RowKind::Queued, "sodium", "queued", ""),
                (RowKind::Queued, "vanadium", "queued", ""),
            ]
        );
        assert_eq!(rows[0].elapsed, Some(secs(5)));
        assert_eq!(board.counts(), "4 hosts · 4 queued");
    }

    #[test]
    fn test_rows_list_hosts_by_lane() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply("vanadium", &HostEvent::Started(PipelineStep::Copy), t0);
        board.apply(
            "lanthanum",
            &HostEvent::Failed {
                step: PipelineStep::Plan,
                reason: "hook 'wait-storage-healthy': /nix/store/715s-kubernetes-storage-healthy missing"
                    .to_string(),
            },
            t0,
        );
        board.apply(
            "cerium",
            &HostEvent::WaitingForSerialGroup {
                group: "k8s".to_string(),
                holder: Some("vanadium".to_string()),
            },
            t0,
        );
        board.apply(
            "sodium",
            &HostEvent::Done(HostDeploymentResult::unchanged("sodium")),
            t0,
        );

        assert_eq!(
            summary(&board.rows(t0)),
            vec![
                (
                    RowKind::Queued,
                    "cerium",
                    "queued",
                    "serial group 'k8s' (held by vanadium)"
                ),
                (RowKind::Active, "vanadium", "copy", ""),
                (RowKind::Unchanged, "sodium", "unchanged", ""),
                (
                    RowKind::Failed,
                    "lanthanum",
                    "plan",
                    "hook 'wait-storage-healthy': /nix/store/715s-kubernetes-storage-healthy missing"
                ),
            ]
        );
        assert_eq!(
            board.counts(),
            "4 hosts · 1 done · 1 failed · 1 running · 1 queued"
        );
    }

    #[test]
    fn test_failed_row_shows_the_last_nix_error_line() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply(
            "cerium",
            &HostEvent::Failed {
                step: PipelineStep::Build,
                reason: "Failed to build host configuration for 'cerium':\n\
                         error: builder for '/nix/store/xyz-cerium.drv' failed with exit code 1\n\
                         error: attribute 'services.nginx.virtualHosts' already defined"
                    .to_string(),
            },
            t0,
        );

        let cerium = row_of(&board, "cerium", t0 + secs(9));
        assert_eq!(
            cerium.detail,
            "attribute 'services.nginx.virtualHosts' already defined"
        );
        assert_eq!(cerium.elapsed, None);
    }

    #[test]
    fn test_step_elapsed_restarts_with_each_step() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply("cerium", &HostEvent::Started(PipelineStep::Build), t0);
        board.apply(
            "cerium",
            &HostEvent::Started(PipelineStep::Copy),
            t0 + secs(100),
        );

        let cerium = row_of(&board, "cerium", t0 + secs(130));
        assert_eq!(
            (cerium.step.as_str(), cerium.elapsed),
            ("copy", Some(secs(30)))
        );
    }

    #[test]
    fn test_done_rows_show_how_the_host_was_deployed_and_when() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply(
            "cerium",
            &HostEvent::Done(deployed("cerium", ActivationMode::Boot, true)),
            t0 + secs(595),
        );
        board.apply(
            "vanadium",
            &HostEvent::Done(deployed("vanadium", ActivationMode::Switch, false)),
            t0 + secs(252),
        );

        let rows = board.rows(t0 + secs(900));
        let done: Vec<_> = rows
            .iter()
            .filter(|r| r.kind == RowKind::Done)
            .map(|r| (r.hostname.as_str(), r.detail.as_str(), r.elapsed))
            .collect();
        assert_eq!(
            done,
            vec![
                ("cerium", "boot, rebooted", Some(secs(595))),
                ("vanadium", "switch", Some(secs(252))),
            ]
        );
    }

    #[test]
    fn test_hooks_show_while_they_run_but_never_hide_a_failure() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.hook_started("cerium", HookStage::PreDeploy, "drain-node", t0);
        let while_draining = row_of(&board, "cerium", t0);
        board.apply(
            "cerium",
            &HostEvent::Failed {
                step: PipelineStep::Activate,
                reason: "activation failed".to_string(),
            },
            t0,
        );
        board.hook_started("cerium", HookStage::OnFailure, "recover-uncordon", t0);
        board.apply("cerium", &HostEvent::Started(PipelineStep::Reboot), t0);

        assert_eq!(
            brief(&while_draining),
            (RowKind::Active, "cerium", "pre-deploy", "hook 'drain-node'")
        );
        assert_eq!(
            brief(&row_of(&board, "cerium", t0)),
            (RowKind::Failed, "cerium", "activate", "activation failed")
        );
    }

    #[test]
    fn test_done_with_a_failure_takes_the_final_reason() {
        let t0 = Instant::now();
        let mut board = board(t0);
        let mut result = deployed("cerium", ActivationMode::Boot, false);
        result.success = false;
        result.failed_at_step = Some(PipelineStep::Reboot);
        result.error_message =
            Some("Deployment succeeded but reboot failed: host did not come back".to_string());

        board.apply(
            "cerium",
            &HostEvent::Failed {
                step: PipelineStep::Reboot,
                reason: "host did not come back".to_string(),
            },
            t0,
        );
        board.apply("cerium", &HostEvent::Done(result), t0);

        assert_eq!(
            brief(&row_of(&board, "cerium", t0)),
            (
                RowKind::Failed,
                "cerium",
                "reboot",
                "Deployment succeeded but reboot failed: host did not come back"
            )
        );
    }

    #[test]
    fn test_hosts_skipped_by_their_serial_group_say_why() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply(
            "vanadium",
            &HostEvent::Done(HostDeploymentResult::skipped_by_serial_group(
                "vanadium", "k8s", "cerium",
            )),
            t0,
        );

        assert_eq!(
            brief(&row_of(&board, "vanadium", t0)),
            (
                RowKind::Failed,
                "vanadium",
                "skipped",
                "serial group 'k8s' halted after cerium failed"
            )
        );
    }

    #[test]
    fn test_events_for_hosts_outside_the_phase_are_ignored() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply("ferrum", &HostEvent::Started(PipelineStep::Build), t0);
        board.hook_started("ferrum", HookStage::PreDeploy, "drain-node", t0);

        assert!(board.rows(t0).iter().all(|r| r.kind == RowKind::Queued));
    }

    #[test]
    fn test_a_failure_is_never_undone() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply(
            "cerium",
            &HostEvent::Failed {
                step: PipelineStep::Copy,
                reason: "Copy closure failed".to_string(),
            },
            t0,
        );
        board.apply("cerium", &HostEvent::Finished(PipelineStep::Copy), t0);
        board.apply(
            "cerium",
            &HostEvent::Done(deployed("cerium", ActivationMode::Switch, false)),
            t0,
        );

        assert_eq!(
            brief(&row_of(&board, "cerium", t0)),
            (RowKind::Failed, "cerium", "copy", "Copy closure failed")
        );
    }

    #[test]
    fn test_failed_done_without_details_keeps_the_earlier_ones() {
        let t0 = Instant::now();
        let mut board = board(t0);
        let mut result = deployed("cerium", ActivationMode::Switch, false);
        result.success = false;

        board.apply(
            "cerium",
            &HostEvent::Failed {
                step: PipelineStep::Copy,
                reason: "Copy closure failed".to_string(),
            },
            t0,
        );
        board.apply("cerium", &HostEvent::Done(result), t0);

        assert_eq!(
            brief(&row_of(&board, "cerium", t0)),
            (RowKind::Failed, "cerium", "copy", "Copy closure failed")
        );
    }

    #[test]
    fn test_serial_group_wait_without_a_known_holder_then_its_turn() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply(
            "cerium",
            &HostEvent::WaitingForSerialGroup {
                group: "k8s".to_string(),
                holder: None,
            },
            t0,
        );
        let waiting = row_of(&board, "cerium", t0);
        let waiting = (waiting.kind, waiting.detail);
        board.apply("cerium", &HostEvent::Started(PipelineStep::Activate), t0);

        assert_eq!(
            waiting,
            (
                RowKind::Queued,
                "serial group 'k8s' (held by another host)".to_string()
            )
        );
        assert_eq!(
            brief(&row_of(&board, "cerium", t0)),
            (RowKind::Active, "cerium", "activate", "")
        );
    }

    #[test]
    fn test_lines_list_every_lane_with_its_hosts_under_it() {
        let t0 = Instant::now();
        let mut board = board(t0);

        board.apply("vanadium", &HostEvent::Started(PipelineStep::Activate), t0);
        board.apply("sodium", &HostEvent::Started(PipelineStep::Activate), t0);
        board.apply(
            "lanthanum",
            &HostEvent::Done(deployed("lanthanum", ActivationMode::Switch, false)),
            t0,
        );

        let lines: Vec<String> = board
            .lines(t0)
            .iter()
            .map(|line| match line {
                Line::Lane { lane, hosts } => format!("{lane:?} ({hosts})"),
                Line::Host(row) => format!("  {}", row.hostname),
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                "Queued (1)",
                "  cerium",
                "Build (0)",
                "Copy (0)",
                "Activate (2)",
                "  sodium",
                "  vanadium",
                "Reboot (0)",
                "Done (1)",
                "  lanthanum",
                "Failed (0)",
            ]
        );
        assert_eq!(lines.len(), board.line_count());
    }

    #[test]
    fn test_each_state_is_listed_in_its_lane() {
        let hook = |stage| State::Hook {
            stage,
            name: "drain-node".to_string(),
        };
        let cases = [
            (State::Queued, Lane::Queued),
            (
                State::WaitingForSerialGroup {
                    group: "k8s".to_string(),
                    holder: None,
                },
                Lane::Queued,
            ),
            (State::Step(PipelineStep::Build), Lane::Build),
            (State::Step(PipelineStep::Check), Lane::Build),
            (State::Step(PipelineStep::Copy), Lane::Copy),
            (hook(HookStage::PreDeploy), Lane::Activate),
            (State::Step(PipelineStep::Plan), Lane::Activate),
            (State::Step(PipelineStep::PreDeploy), Lane::Activate),
            (State::Step(PipelineStep::Activate), Lane::Activate),
            (State::Step(PipelineStep::Reboot), Lane::Reboot),
            (hook(HookStage::PostReboot), Lane::Reboot),
            (State::Step(PipelineStep::PostDeploy), Lane::Activate),
            (hook(HookStage::PostDeploy), Lane::Activate),
            (
                State::Done(deployed("cerium", ActivationMode::Switch, false)),
                Lane::Done,
            ),
            (
                State::Failed {
                    step: Some(PipelineStep::Copy),
                    reason: "Copy closure failed".to_string(),
                },
                Lane::Failed,
            ),
        ];

        for (state, lane) in cases {
            assert_eq!(lane_of(&state), lane, "{state:?}");
        }
    }

    #[test]
    fn test_header_and_width_for_a_single_host() {
        let t0 = Instant::now();
        let board = HostBoard::new("default", &["praseodymium".to_string()], t0);

        assert_eq!(board.title(), "Push");
        assert_eq!(board.counts(), "1 host · 1 queued");
        assert_eq!(board.hostname_width(), 12);
    }

    #[test]
    fn test_elapsed_reads_like_a_clock() {
        assert_eq!(format_elapsed(secs(7)), "0:07");
        assert_eq!(format_elapsed(secs(187)), "3:07");
        assert_eq!(format_elapsed(secs(3727)), "1:02:07");
    }
}
