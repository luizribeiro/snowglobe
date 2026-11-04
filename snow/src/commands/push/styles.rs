//! How the push TUI draws a phase header and its lanes of hosts
//!
//! Each line is redrawn from the shared board on every tick, so elapsed
//! times keep moving between events.

use super::board::{format_elapsed, HostBoard, Lane, Line, Row, RowKind};
use super::colors::style;
use console::Color;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing_indicatif::style::ProgressStyle;

pub(super) type SharedBoard = Arc<Mutex<Option<HostBoard>>>;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const LANE_INDENT: &str = "  ";
const ROW_INDENT: &str = "    ";
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

/// "⠋ ▶ Phase p2 · 3 hosts · 1 done  ━━━━━━╸───────────── 33%", green until a
/// host fails, then red. The bar is left out when it would not fit: a header
/// that wraps breaks indicatif's redraws.
pub(super) fn header_text(board: &HostBoard, spinner: &str, width: usize) -> String {
    let (finished, total) = board.progress();
    let color = header_color(board);
    let title = format!("{spinner} ▶ {}", board.title());
    let counts = format!(" · {}", board.counts());
    let (filled, empty) = progress_bar(finished, total, BAR_WIDTH);
    let percent = format!(" {}%", finished * 100 / total.max(1));
    let fits = [&title, &counts, "  ", &filled, &empty, &percent]
        .iter()
        .map(|part| part.chars().count())
        .sum::<usize>()
        + ELAPSED_WIDTH
        < width;
    let heading = format!("{}{}", style(title).fg(color).bold(), style(counts).dim());
    if fits {
        format!(
            "{heading}  {}{}{percent}",
            style(filled).fg(color),
            style(empty).dim()
        )
    } else {
        heading
    }
}

fn header_color(board: &HostBoard) -> Color {
    if board.any_failed() {
        Color::Red
    } else {
        Color::Green
    }
}

/// The bar's filled part, with its head, and the part still to go
pub(super) fn progress_bar(finished: usize, total: usize, width: usize) -> (String, String) {
    let full = finished * width / total.max(1);
    if full >= width {
        ("━".repeat(width), String::new())
    } else {
        (
            format!("{}╸", "━".repeat(full)),
            "─".repeat(width - full - 1),
        )
    }
}

/// The style of the `slot`-th line, whatever is listed there right now
pub(super) fn line_style(board: SharedBoard, slot: usize) -> ProgressStyle {
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

/// The `slot`-th line, coloured, or None past the last one
pub(super) fn render_slot(
    board: &HostBoard,
    slot: usize,
    now: Instant,
    width: usize,
    spinner: &str,
) -> Option<String> {
    Some(match board.lines(now).into_iter().nth(slot)? {
        Line::Lane { lane, hosts } => lane_heading(lane, hosts),
        Line::Host(row) => {
            let text = row_text(&row, board.hostname_width(), width, spinner);
            styled(&row, text)
        }
    })
}

fn lane_color(lane: Lane) -> Color {
    match lane {
        Lane::Queued => Color::White,
        Lane::Build => Color::Yellow,
        Lane::Copy => Color::Blue,
        Lane::Activate => Color::Cyan,
        Lane::Reboot => Color::Magenta,
        Lane::Done => Color::Green,
        Lane::Failed => Color::Red,
    }
}

fn lane_title(lane: Lane) -> &'static str {
    match lane {
        Lane::Queued => "◷ Queued",
        Lane::Build => "⚙ Build",
        Lane::Copy => "⇢ Copy",
        Lane::Activate => "★ Activate",
        Lane::Reboot => "⟳ Reboot",
        Lane::Done => "✓ Done",
        Lane::Failed => "✗ Failed",
    }
}

/// Empty lanes are dimmed, so the eye goes to where the hosts are
fn lane_heading(lane: Lane, hosts: usize) -> String {
    let text = style(format!("{LANE_INDENT}{}", lane_title(lane)));
    if hosts == 0 {
        text.dim().to_string()
    } else {
        text.fg(lane_color(lane)).bold().to_string()
    }
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

/// A host at work takes its lane's colour
fn styled(row: &Row, text: String) -> String {
    match row.kind {
        RowKind::Failed => style(text).red().to_string(),
        RowKind::Active => style(text).fg(lane_color(row.lane)).to_string(),
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
            lane: Lane::Activate,
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
            "    ⠋ cerium     reboot                                3:12"
        );
        assert_eq!(
            neodymium,
            "    ◷ neodymium  queued      serial group 'k8s' (held… 2:40"
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

        assert_eq!(text, "    ✗ lanthanum  plan        hook 'wait-storage-…");
        assert_eq!(text.chars().count(), 49);
    }

    #[test]
    fn test_slots_list_each_lane_then_its_hosts_and_end_with_the_last() {
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

        let slots: Vec<_> = (0..=board.line_count())
            .map(|slot| render_slot(&board, slot, t0, 60, "⠋"))
            .collect();

        assert_eq!(
            slots,
            vec![
                Some("  ◷ Queued".to_string()),
                Some(format!("    ◷ cerium  queued{}0:00", " ".repeat(35))),
                Some("  ⚙ Build".to_string()),
                Some("  ⇢ Copy".to_string()),
                Some("  ★ Activate".to_string()),
                Some("  ⟳ Reboot".to_string()),
                Some("  ✓ Done".to_string()),
                Some("  ✗ Failed".to_string()),
                Some("    ✗ radon   build       error: attribute 'wine' missing".to_string()),
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
            "⠋ ▶ Phase p2 · 3 hosts · 💥 1 failed · 1 running · 1 queued  ━━━━━━╸───────────── 33%"
        );
        assert_eq!(
            header_text(&board, "⠋", 80),
            "⠋ ▶ Phase p2 · 3 hosts · 💥 1 failed · 1 running · 1 queued"
        );
    }

    #[test]
    fn test_each_lane_has_a_fixed_colour() {
        let colors: Vec<_> = Lane::ALL.iter().map(|&lane| lane_color(lane)).collect();

        assert_eq!(
            colors,
            vec![
                Color::White,
                Color::Yellow,
                Color::Blue,
                Color::Cyan,
                Color::Magenta,
                Color::Green,
                Color::Red,
            ]
        );
    }

    #[test]
    fn test_header_turns_red_once_a_host_fails() {
        let t0 = Instant::now();
        let hosts: Vec<String> = ["boron", "cerium"].map(String::from).to_vec();
        let mut board = HostBoard::new("p2", &hosts, t0);
        let before = header_color(&board);

        board.apply(
            "cerium",
            &snowlib::HostEvent::Failed {
                step: snowlib::PipelineStep::Copy,
                reason: "Copy closure failed".to_string(),
            },
            t0,
        );

        assert_eq!((before, header_color(&board)), (Color::Green, Color::Red));
    }

    #[test]
    fn test_progress_bar_fills_as_hosts_finish() {
        let bar = |finished, total| {
            let (filled, empty) = progress_bar(finished, total, 10);
            format!("{filled}|{empty}")
        };
        assert_eq!(bar(0, 3), "╸|─────────");
        assert_eq!(bar(1, 2), "━━━━━╸|────");
        assert_eq!(bar(3, 3), "━━━━━━━━━━|");
        assert_eq!(bar(0, 0), "╸|─────────");
    }

    #[test]
    fn test_short_rows_are_not_padded_without_elapsed() {
        let sodium = row(RowKind::Unchanged, "sodium", "unchanged", "", None);

        assert_eq!(row_text(&sodium, 9, 80, "⠋"), "    = sodium     unchanged");
    }
}
