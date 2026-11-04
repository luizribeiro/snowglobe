//! How the push TUI draws a phase header and its host rows
//!
//! Each line is redrawn from the shared board on every tick, so elapsed
//! times keep moving between events.

use super::board::{format_elapsed, HostBoard, Row, RowKind};
use super::colors::style;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing_indicatif::style::ProgressStyle;

pub(super) type SharedBoard = Arc<Mutex<Option<HostBoard>>>;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ROW_INDENT: &str = "  ";
const BAR_WIDTH: usize = 20;

pub(super) fn header_style(board: SharedBoard) -> ProgressStyle {
    ProgressStyle::with_template("{header} {elapsed_precise:.dim}")
        .expect("valid template")
        .with_key(
            "header",
            move |_: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                if let Some(board) = board.lock().as_ref() {
                    let _ = write!(w, "{}", header_text(board, spinner(), terminal_width()));
                }
            },
        )
}

/// The template's elapsed time, "00:03:07", and the space before it
const ELAPSED_WIDTH: usize = 9;

/// "⠋ ▶ p2 · 3 hosts · 1 done  ━━━━━━╸───────────── 33%". The bar is left out
/// when it would not fit: a header that wraps breaks indicatif's redraws.
pub(super) fn header_text(board: &HostBoard, spinner: &str, width: usize) -> String {
    let (finished, total) = board.progress();
    let title = format!("{spinner} ▶ {}", board.header());
    let bar = format!(
        "  {} {}%",
        progress_bar(finished, total, BAR_WIDTH),
        finished * 100 / total.max(1)
    );
    let fits = title.chars().count() + bar.chars().count() + ELAPSED_WIDTH < width;
    let title = style(title).bold();
    if fits {
        format!("{title}{bar}")
    } else {
        title.to_string()
    }
}

pub(super) fn progress_bar(finished: usize, total: usize, width: usize) -> String {
    let full = finished * width / total.max(1);
    if full >= width {
        "━".repeat(width)
    } else {
        format!("{}╸{}", "━".repeat(full), "─".repeat(width - full - 1))
    }
}

/// The style of the `slot`-th row, whichever host is listed there right now
pub(super) fn row_style(board: SharedBoard, slot: usize) -> ProgressStyle {
    ProgressStyle::with_template("{row}")
        .expect("valid template")
        .with_key(
            "row",
            move |_: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                let line = board.lock().as_ref().and_then(|board| {
                    render_slot(board, slot, Instant::now(), terminal_width(), spinner())
                });
                if let Some(line) = line {
                    let _ = write!(w, "{line}");
                }
            },
        )
}

/// The `slot`-th row, coloured, or None past the last host
pub(super) fn render_slot(
    board: &HostBoard,
    slot: usize,
    now: Instant,
    width: usize,
    spinner: &str,
) -> Option<String> {
    let row = board.rows(now).into_iter().nth(slot)?;
    let text = row_text(&row, board.hostname_width(), width, spinner);
    Some(styled(row.kind, text))
}

/// One row as plain text, cut to fit `width` columns
pub(super) fn row_text(
    row: &Row,
    hostname_width: usize,
    max_width: usize,
    spinner: &str,
) -> String {
    let icon = match row.kind {
        RowKind::Failed => "✗",
        RowKind::Active => spinner,
        RowKind::Queued => "◷",
        RowKind::Done => "✓",
        RowKind::Unchanged => "=",
    };
    let elapsed = row.elapsed.map(format_elapsed).unwrap_or_default();
    let line = format!(
        "{ROW_INDENT}{icon} {:<hostname_width$}  {:<11} {}",
        row.hostname, row.step, row.detail
    );
    let line = line.trim_end();
    // Some terminals wrap a line that reaches the last column
    let usable = max_width.saturating_sub(1);
    let room = if elapsed.is_empty() {
        usable
    } else {
        usable.saturating_sub(elapsed.chars().count() + 1)
    };
    let line = truncate(line, room);
    if elapsed.is_empty() {
        line
    } else {
        format!("{line:<room$} {elapsed}")
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let kept: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

fn styled(kind: RowKind, text: String) -> String {
    match kind {
        RowKind::Failed => style(text).red().to_string(),
        RowKind::Active => style(text).cyan().to_string(),
        RowKind::Queued | RowKind::Unchanged => style(text).dim().to_string(),
        RowKind::Done => style(text).green().to_string(),
    }
}

fn terminal_width() -> usize {
    console::Term::stderr().size().1 as usize
}

fn spinner() -> &'static str {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    SPINNER_FRAMES[(millis / 100) as usize % SPINNER_FRAMES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn row(kind: RowKind, hostname: &str, step: &str, detail: &str, secs: Option<u64>) -> Row {
        Row {
            kind,
            hostname: hostname.to_string(),
            step: step.to_string(),
            detail: detail.to_string(),
            elapsed: secs.map(Duration::from_secs),
        }
    }

    #[test]
    fn test_rows_line_up_with_elapsed_at_the_right_edge() {
        let cerium = row(RowKind::Active, "cerium", "reboot", "", Some(192));
        let neodymium = row(
            RowKind::Queued,
            "neodymium",
            "queued",
            "serial group 'k8s' (held by cerium)",
            Some(160),
        );

        let cerium = row_text(&cerium, 9, 60, "⠋");
        let neodymium = row_text(&neodymium, 9, 60, "⠋");

        assert_eq!(
            cerium,
            "  ⠋ cerium     reboot                                  3:12"
        );
        assert_eq!(
            neodymium,
            "  ◷ neodymium  queued      serial group 'k8s' (held b… 2:40"
        );
        assert_eq!(cerium.chars().count(), 59);
        assert_eq!(neodymium.chars().count(), 59);
    }

    #[test]
    fn test_failed_row_without_elapsed_is_cut_to_the_terminal() {
        let lanthanum = row(
            RowKind::Failed,
            "lanthanum",
            "plan",
            "hook 'wait-storage-healthy': /nix/store/715s-kubernetes-storage-healthy missing",
            None,
        );

        let text = row_text(&lanthanum, 9, 50, "⠋");

        assert_eq!(text, "  ✗ lanthanum  plan        hook 'wait-storage-he…");
        assert_eq!(text.chars().count(), 49);
    }

    #[test]
    fn test_slots_follow_the_board_order_and_end_with_the_hosts() {
        console::set_colors_enabled(false);
        let t0 = Instant::now();
        let hosts = vec!["radon".to_string(), "cerium".to_string()];
        let mut board = HostBoard::new("p3", &hosts, t0);
        board.apply(
            "radon",
            &snowlib::HostEvent::Failed {
                step: snowlib::PipelineStep::Build,
                reason: "error: attribute 'wine' missing".to_string(),
            },
            t0,
        );

        let slots: Vec<_> = (0..3)
            .map(|slot| render_slot(&board, slot, t0, 60, "⠋"))
            .collect();

        assert_eq!(
            slots,
            vec![
                Some("  ✗ radon   build       error: attribute 'wine' missing".to_string()),
                Some(format!("  ◷ cerium  queued{}0:00", " ".repeat(37))),
                None,
            ]
        );
    }

    #[test]
    fn test_header_shows_how_much_of_the_phase_is_finished() {
        console::set_colors_enabled(false);
        let t0 = Instant::now();
        let hosts: Vec<String> = ["boron", "cerium", "radon"].map(String::from).to_vec();
        let mut board = HostBoard::new("p2", &hosts, t0);
        board.apply(
            "cerium",
            &snowlib::HostEvent::Started(snowlib::PipelineStep::Copy),
            t0,
        );
        board.apply(
            "radon",
            &snowlib::HostEvent::Failed {
                step: snowlib::PipelineStep::Activate,
                reason: "storage-agent.service failed".to_string(),
            },
            t0,
        );

        assert_eq!(
            header_text(&board, "⠋", 100),
            "⠋ ▶ p2 · 3 hosts · 💥 1 failed · 1 running · 1 queued  ━━━━━━╸───────────── 33%"
        );
        assert_eq!(
            header_text(&board, "⠋", 80),
            "⠋ ▶ p2 · 3 hosts · 💥 1 failed · 1 running · 1 queued"
        );
    }

    #[test]
    fn test_progress_bar_fills_as_hosts_finish() {
        assert_eq!(progress_bar(0, 3, 10), "╸─────────");
        assert_eq!(progress_bar(1, 2, 10), "━━━━━╸────");
        assert_eq!(progress_bar(3, 3, 10), "━━━━━━━━━━");
        assert_eq!(progress_bar(0, 0, 10), "╸─────────");
    }

    #[test]
    fn test_short_rows_are_not_padded_without_elapsed() {
        let sodium = row(RowKind::Unchanged, "sodium", "unchanged", "", None);

        assert_eq!(row_text(&sodium, 9, 80, "⠋"), "  = sodium     unchanged");
    }
}
