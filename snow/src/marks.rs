//! The mark at the start of a line that says how something went. Every
//! command uses these so an outcome looks the same wherever it shows up.

use console::{style, StyledObject};

pub fn success() -> StyledObject<&'static str> {
    style("✓").green()
}

pub fn failure() -> StyledObject<&'static str> {
    style("✗").red()
}

pub fn warning() -> StyledObject<&'static str> {
    style("!").yellow().bold()
}

/// Later lines sit under the first line's text, past the mark and its space
pub fn warning_lines(text: &str) -> String {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let lines: Vec<String> = std::iter::once(first.to_string())
        .chain(indented(rest, "  "))
        .collect();
    format!("{} {}", warning(), lines.join("\n"))
}

/// Each line of `text` behind `indent`, leaving blank lines empty
pub fn indented(text: &str, indent: &str) -> Vec<String> {
    text.lines()
        .map(|line| match line.is_empty() {
            true => String::new(),
            false => format!("{indent}{line}"),
        })
        .collect()
}

/// Marks what was skipped or cancelled on purpose
pub fn skipped() -> StyledObject<&'static str> {
    style("⊘").dim()
}

/// A line saying what was skipped or cancelled, dim from mark to end
pub fn skipped_line(text: &str) -> String {
    styled_skipped_line(text, console::colors_enabled())
}

// Takes colors as a parameter so tests can force styling without touching
// console's global color state, which other tests depend on
fn styled_skipped_line(text: &str, colors: bool) -> String {
    format!(
        "{} {}",
        skipped().force_styling(colors),
        style(text).dim().force_styling(colors)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(mark: StyledObject<&'static str>) -> String {
        mark.force_styling(true).to_string()
    }

    #[test]
    fn test_marks_render_with_their_colors() {
        assert_eq!(rendered(success()), "\u{1b}[32m✓\u{1b}[0m");
        assert_eq!(rendered(failure()), "\u{1b}[31m✗\u{1b}[0m");
        assert_eq!(rendered(warning()), "\u{1b}[33m\u{1b}[1m!\u{1b}[0m");
    }

    #[test]
    fn test_warning_lines_leaves_blank_lines_empty() {
        assert_eq!(
            console::strip_ansi_codes(&warning_lines("First\n\nThird\n")),
            "! First\n\n  Third"
        );
    }

    #[test]
    fn test_warning_lines_indents_later_lines_under_the_text() {
        let lines = warning_lines("Until you reboot, the system mixes old and new parts,\nand anything that depends on both may misbehave.");
        assert_eq!(
            console::strip_ansi_codes(&lines),
            "! Until you reboot, the system mixes old and new parts,\n  and anything that depends on both may misbehave."
        );
    }

    #[test]
    fn test_skipped_line_is_dim_from_mark_to_end() {
        assert_eq!(
            styled_skipped_line("Cancelled, nothing changed", true),
            "\u{1b}[2m⊘\u{1b}[0m \u{1b}[2mCancelled, nothing changed\u{1b}[0m"
        );
        assert_eq!(
            styled_skipped_line("Cancelled, nothing changed", false),
            "⊘ Cancelled, nothing changed"
        );
    }
}
