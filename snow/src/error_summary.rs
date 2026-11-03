//! One-line summaries of long error messages

/// The line of an error worth showing on its own
///
/// Nix errors carry a long trace; the final `error:` line names the cause.
pub fn error_summary(error: &str) -> &str {
    error
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("error:"))
        .next_back()
        .unwrap_or_else(|| error.lines().next().unwrap_or_default().trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_summary_without_error_line_uses_first_line() {
        assert_eq!(error_summary("worker crashed\nsignal 9"), "worker crashed");
    }

    #[test]
    fn test_error_summary_uses_last_error_line() {
        let error = "error:\n       … while evaluating 'toplevel'\n\n       error: attribute 'wine' missing";

        assert_eq!(error_summary(error), "error: attribute 'wine' missing");
    }
}
