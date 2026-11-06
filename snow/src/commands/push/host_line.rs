//! A single host's row on its own, drawn as push draws each host, for
//! commands that work on one host
//!
//! The row comes and goes with its span; it is hidden while something else
//! needs the terminal, such as a question or sudo asking for a password.

use super::board::HostBoard;
use super::styles::{lone_host_style, SharedBoard};
use parking_lot::Mutex;
use snowlib::{HostEvent, PipelineStep};
use std::sync::Arc;
use std::time::Instant;
use tracing_indicatif::span_ext::IndicatifSpanExt;

pub(in crate::commands) struct HostLine {
    hostname: String,
    board: SharedBoard,
    drawn: Mutex<Drawn>,
}

#[derive(Default)]
struct Drawn {
    wanted: bool,
    /// Pauses may nest, as when sudo asks while a question is open
    pauses: usize,
    step: Option<PipelineStep>,
    span: Option<tracing::Span>,
}

impl HostLine {
    pub fn new(hostname: &str) -> Self {
        let board = HostBoard::new("rebuild", &[hostname.to_string()], Instant::now());
        Self {
            hostname: hostname.to_string(),
            board: Arc::new(Mutex::new(Some(board))),
            drawn: Mutex::default(),
        }
    }

    /// Shows the row, at `step` since now
    pub fn start(&self, step: PipelineStep) {
        if let Some(board) = self.board.lock().as_mut() {
            board.apply(&self.hostname, &HostEvent::Started(step), Instant::now());
        }
        self.redraw(|drawn| {
            drawn.wanted = true;
            drawn.step = Some(step);
        });
    }

    /// The step started last, if any
    pub fn step(&self) -> Option<PipelineStep> {
        self.drawn.lock().step
    }

    pub fn hide(&self) {
        self.redraw(|drawn| drawn.wanted = false);
    }

    pub fn pause(&self) {
        self.redraw(|drawn| drawn.pauses += 1);
    }

    pub fn resume(&self) {
        self.redraw(|drawn| drawn.pauses = drawn.pauses.saturating_sub(1));
    }

    pub fn paused<T>(&self, work: impl FnOnce() -> T) -> T {
        self.pause();
        let done = work();
        self.resume();
        done
    }

    #[cfg(test)]
    fn is_drawn(&self) -> bool {
        self.drawn.lock().span.is_some()
    }

    fn redraw(&self, change: impl FnOnce(&mut Drawn)) {
        let mut drawn = self.drawn.lock();
        change(&mut drawn);
        let visible = drawn.wanted && drawn.pauses == 0;
        if visible && drawn.span.is_none() {
            drawn.span = Some(self.span());
        } else if !visible {
            drawn.span = None;
        }
    }

    fn span(&self) -> tracing::Span {
        let span = tracing::info_span!("host");
        span.pb_set_style(&lone_host_style(self.board.clone()));
        span.pb_start();
        span
    }
}

/// No tracing-indicatif layer runs in tests, so they check which spans
/// exist rather than what the terminal shows
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row_shows_from_the_first_step_until_hidden() {
        let line = HostLine::new("boron");
        assert!(!line.is_drawn());

        line.start(PipelineStep::Build);
        assert!(line.is_drawn());

        line.hide();
        assert!(!line.is_drawn());
    }

    #[test]
    fn test_paused_row_comes_back_only_if_it_was_shown() {
        let line = HostLine::new("boron");
        line.paused(|| ());
        assert!(!line.is_drawn());

        line.start(PipelineStep::Check);
        let drawn_meanwhile = line.paused(|| line.is_drawn());

        assert!(!drawn_meanwhile);
        assert!(line.is_drawn());
    }

    #[test]
    fn test_row_stays_away_until_the_outer_pause_ends() {
        let line = HostLine::new("boron");
        line.start(PipelineStep::Check);

        line.pause();
        line.paused(|| ());
        assert!(!line.is_drawn());

        line.resume();
        assert!(line.is_drawn());
    }

    #[test]
    fn test_step_started_while_paused_shows_once_resumed() {
        let line = HostLine::new("boron");
        line.pause();

        line.start(PipelineStep::Build);
        assert!(!line.is_drawn());

        line.resume();
        assert!(line.is_drawn());
        assert_eq!(line.step(), Some(PipelineStep::Build));
    }
}
