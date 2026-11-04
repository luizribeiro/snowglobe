//! The mark at the start of a line that says how something went. Every
//! command uses these so an outcome looks the same wherever it shows up.

use console::{style, StyledObject};

pub fn success() -> StyledObject<&'static str> {
    style("✓").green()
}

pub fn failure() -> StyledObject<&'static str> {
    style("✗").red()
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
}
