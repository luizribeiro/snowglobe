//! The mark at the start of a line that says how something went. Every
//! command uses these so an outcome looks the same wherever it shows up.

use console::{style, StyledObject};

pub fn success() -> StyledObject<&'static str> {
    style("✓").green()
}

pub fn failure() -> StyledObject<&'static str> {
    style("✗").red()
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
    fn test_success_is_a_green_check_and_failure_a_red_cross() {
        assert_eq!(rendered(success()), "\u{1b}[32m✓\u{1b}[0m");
        assert_eq!(rendered(failure()), "\u{1b}[31m✗\u{1b}[0m");
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
