//! CLI observer: a header per phase with its lanes of hosts underneath
//!
//! Events update a shared HostBoard; the tracing-indicatif spans only hold
//! styles that read the board whenever the terminal is redrawn.

use super::board::HostBoard;
use super::styles::{header_style, line_style, SharedBoard};
use parking_lot::Mutex;
use snowlib::{HookStage, HostEvent, PhaseResult, PushObserver};
use std::time::Instant;
use tracing::info_span;
use tracing_indicatif::span_ext::IndicatifSpanExt;

pub struct CliObserver {
    board: SharedBoard,
    /// Closing them removes the phase's lines
    spans: Mutex<Vec<tracing::Span>>,
}

impl CliObserver {
    pub fn new() -> Self {
        Self {
            board: SharedBoard::default(),
            spans: Mutex::new(Vec::new()),
        }
    }

    /// `change` must not touch the spans: their styles lock the board to draw
    fn update(&self, change: impl FnOnce(&mut HostBoard, Instant)) {
        if let Some(board) = self.board.lock().as_mut() {
            change(board, Instant::now());
        }
    }
}

impl PushObserver for CliObserver {
    fn on_phase_start(&self, phase: &str, hosts: &[String]) {
        let board = HostBoard::new(phase, hosts, Instant::now());
        let lines = board.line_count();
        *self.board.lock() = Some(board);

        let header = info_span!("phase");
        header.pb_set_style(&header_style(self.board.clone()));
        header.pb_start();

        // indicatif lists the most recently started line first
        let mut spans: Vec<tracing::Span> = (0..lines)
            .rev()
            .map(|slot| {
                let row = info_span!(parent: &header, "line");
                row.pb_set_style(&line_style(self.board.clone(), slot));
                row.pb_start();
                row
            })
            .collect();
        spans.push(header);
        *self.spans.lock() = spans;
    }

    fn on_phase_complete(&self, _phase: &str, result: Option<&PhaseResult>) {
        // Closing the spans removes the phase's lines, so the report takes their place
        self.spans.lock().clear();
        *self.board.lock() = None;
        if let Some(result) = result {
            // Without suspending, indicatif's next redraw would draw over the report
            tracing_indicatif::suspend_tracing_indicatif(|| {
                println!("{}\n", super::summary::phase_report(result, report_width()));
            });
        }
    }

    fn on_host_event(&self, hostname: &str, event: &HostEvent) {
        self.update(|board, now| board.apply(hostname, event, now));
    }

    fn on_hook_start(&self, hostname: &str, stage: HookStage, hook_name: &str) {
        self.update(|board, now| board.hook_started(hostname, stage, hook_name, now));
    }
}

/// Output that isn't a terminal is read in a log or a pager, which never wrap
fn report_width() -> usize {
    let stdout = console::Term::stdout();
    if stdout.is_term() {
        stdout.size().1 as usize
    } else {
        usize::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::push::board::Lane;
    use snowlib::PipelineStep;

    fn hosts() -> Vec<String> {
        vec!["radon".to_string(), "cerium".to_string()]
    }

    fn rows(observer: &CliObserver) -> Vec<(String, String)> {
        observer
            .board
            .lock()
            .as_ref()
            .unwrap()
            .rows(Instant::now())
            .into_iter()
            .map(|row| (row.hostname, row.step))
            .collect()
    }

    #[test]
    fn test_phase_start_draws_a_header_and_a_line_per_lane_and_host() {
        let observer = CliObserver::new();

        observer.on_phase_start("p3", &hosts());

        assert_eq!(observer.spans.lock().len(), 1 + Lane::ALL.len() + 2);
        assert_eq!(observer.board.lock().as_ref().unwrap().title(), "Phase p3");
    }

    #[test]
    fn test_default_phase_is_shown_as_push() {
        let observer = CliObserver::new();

        observer.on_phase_start("default", &hosts());

        assert_eq!(observer.board.lock().as_ref().unwrap().title(), "Push");
    }

    #[test]
    fn test_events_and_hooks_update_the_rows() {
        let observer = CliObserver::new();
        observer.on_phase_start("p3", &hosts());

        observer.on_host_event("radon", &HostEvent::Started(PipelineStep::Copy));
        observer.on_hook_start("cerium", HookStage::PreDeploy, "drain-node");

        assert_eq!(
            rows(&observer),
            vec![
                ("radon".to_string(), "copy".to_string()),
                ("cerium".to_string(), "pre-deploy".to_string()),
            ]
        );
    }

    #[test]
    fn test_phase_complete_with_results_clears_the_board_and_spans() {
        let observer = CliObserver::new();
        observer.on_phase_start("p3", &hosts());
        let result = PhaseResult {
            phase: "p3".to_string(),
            hosts: vec![snowlib::HostDeploymentResult::unchanged("radon")],
            duration: std::time::Duration::from_secs(5),
        };

        observer.on_phase_complete("p3", Some(&result));

        assert!(observer.spans.lock().is_empty());
        assert!(observer.board.lock().is_none());
    }

    #[test]
    fn test_phase_complete_clears_the_board_and_spans() {
        let observer = CliObserver::new();
        observer.on_phase_start("p3", &hosts());

        observer.on_phase_complete("p3", None);
        observer.on_host_event("radon", &HostEvent::Started(PipelineStep::Copy));

        assert!(observer.spans.lock().is_empty());
        assert!(observer.board.lock().is_none());
    }
}
