//! One-line summaries of long error messages

/// The line of an error worth showing on its own
///
/// Nix errors carry a long trace; the final `error:` line names the cause.
/// A failed command's own last words on stderr usually say what went wrong,
/// where its first line only says that it failed. The line is shown as the
/// cause, so it drops Nix's `error:` prefix; a bare `error:` only heads a trace.
pub fn error_summary(error: &str) -> &str {
    let lines = || error.lines().map(str::trim);
    lines()
        .filter_map(|line| line.strip_prefix("error:"))
        .map(str::trim_start)
        .filter(|cause| !cause.is_empty())
        .next_back()
        .or_else(|| last_stderr_line(error))
        .unwrap_or_else(|| lines().next().unwrap_or_default())
}

/// `text` cut to at most `max` characters, ending in "…" when cut
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let kept: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

/// The lines of an error that lead up to its cause, ending at it
#[derive(Debug, PartialEq, Eq)]
pub struct ErrorContext<'a> {
    pub lines: Vec<&'a str>,
    /// The error's lines left out of `lines`
    pub hidden: usize,
}

/// At most `max` non-empty lines ending at the cause. They come from a
/// failed command's stderr when the cause is there: its stdout ends in
/// service logs that rarely explain the failure. The failed command's
/// `Command:` line then comes first and counts toward `max`, since stderr
/// alone doesn't say what ran, such as which hook.
pub fn error_context(error: &str, max: usize) -> ErrorContext<'_> {
    let cause = error_summary(error);
    let all: Vec<&str> = non_empty(error.lines().map(str::trim)).collect();
    let stderr: Vec<&str> = non_empty(stderr_lines(error)).collect();
    // The cause may be its line without Nix's `error:` prefix
    let holds_cause = |line: &&str| line.ends_with(cause);
    let from_stderr = stderr.iter().any(holds_cause);
    let source = if from_stderr { &stderr } else { &all };
    let end = source
        .iter()
        .rposition(holds_cause)
        .map_or(source.len(), |at| at + 1);
    let command = error
        .lines()
        .map(str::trim)
        .take_while(|line| *line != "Stderr:")
        .find(|line| line.starts_with("Command:"))
        .filter(|_| from_stderr && max >= 2);
    let room = max.saturating_sub(usize::from(command.is_some()));
    let lines: Vec<&str> = command
        .into_iter()
        .chain(source[end.saturating_sub(room)..end].iter().copied())
        .collect();
    ErrorContext {
        hidden: all.len() - lines.len(),
        lines,
    }
}

fn non_empty<'a>(lines: impl Iterator<Item = &'a str>) -> impl Iterator<Item = &'a str> {
    lines.filter(|line| !line.is_empty())
}

/// The trimmed lines of a failed command's "Stderr:" section
fn stderr_lines(error: &str) -> impl Iterator<Item = &str> {
    error
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "Stderr:")
        .skip(1)
        .take_while(|line| *line != "Stdout:")
}

fn last_stderr_line(error: &str) -> Option<&str> {
    non_empty(stderr_lines(error)).last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_marks_a_cut_with_an_ellipsis() {
        let cause = "warning: the following units failed: storage-agent.service";

        assert_eq!(truncate(cause, 80), cause);
        assert_eq!(truncate(cause, cause.len()), cause);
        assert_eq!(truncate(cause, 20), "warning: the follow…");
        assert_eq!(truncate(cause, 0), "…");
    }

    #[test]
    fn test_error_summary_without_error_line_uses_first_line() {
        assert_eq!(error_summary("worker crashed\nsignal 9"), "worker crashed");
    }

    #[test]
    fn test_error_summary_of_a_failed_command_is_its_last_stderr_line() {
        let error = "Command failed\n\n\
                     Command: ssh sudo snowglobe-activate /nix/store/yzxw-cerium switch\n\
                     Exit code: 4\n\n\
                     Stderr:\n\
                     updating GRUB 2 menu...\n\
                     activating the configuration...\n\
                     warning: the following units failed: storage-agent.service\n\n\
                     Stdout:\n\
                     setting up /etc...\n\
                     × storage-agent.service";

        assert_eq!(
            error_summary(error),
            "warning: the following units failed: storage-agent.service"
        );
    }

    #[test]
    fn test_error_summary_finds_stderr_in_an_indented_error() {
        let error = "  Command failed\n  Stderr: \n    storage on radon is degraded\n";

        assert_eq!(error_summary(error), "storage on radon is degraded");
    }

    #[test]
    fn test_error_summary_of_a_command_with_empty_stderr_is_its_first_line() {
        let error = "Command failed\n\nCommand: true\nExit code: 1\n\nStderr:\n";

        assert_eq!(error_summary(error), "Command failed");
    }

    const ACTIVATION_FAILURE: &str = "Command failed\n\n\
        Command: ssh sudo snowglobe-activate /nix/store/yzxw-cerium switch\n\
        Exit code: 4\n\n\
        Stderr:\n\
        updating GRUB 2 menu...\n\
        activating the configuration...\n\
        setting up /etc...\n\
        warning: the following units failed: storage-agent.service\n\n\
        Stdout:\n\
        Nov 04 10:12:03 cerium systemd[1]: Starting storage-agent.service...\n\
        Nov 04 10:12:04 cerium storage-agent[812]: disk sdb missing";

    #[test]
    fn test_error_context_of_a_failed_command_is_its_stderr_up_to_the_cause() {
        assert_eq!(
            error_context(ACTIVATION_FAILURE, 10),
            ErrorContext {
                lines: vec![
                    "Command: ssh sudo snowglobe-activate /nix/store/yzxw-cerium switch",
                    "updating GRUB 2 menu...",
                    "activating the configuration...",
                    "setting up /etc...",
                    "warning: the following units failed: storage-agent.service",
                ],
                hidden: 6,
            }
        );
    }

    #[test]
    fn test_error_context_keeps_the_lines_nearest_the_cause() {
        assert_eq!(
            error_context(ACTIVATION_FAILURE, 3),
            ErrorContext {
                lines: vec![
                    "Command: ssh sudo snowglobe-activate /nix/store/yzxw-cerium switch",
                    "setting up /etc...",
                    "warning: the following units failed: storage-agent.service",
                ],
                hidden: 8,
            }
        );
    }

    #[test]
    fn test_error_context_of_a_failed_hook_says_which_hook_ran() {
        let error = "Command failed\n\n\
            Command: Hook 'wait-storage-healthy' (pre-deploy): /nix/store/abc-wait-storage-healthy\n\
            Exit code: 1\n\n\
            Stderr:\n\
            storage on radon is degraded";

        assert_eq!(
            error_context(error, 10),
            ErrorContext {
                lines: vec![
                    "Command: Hook 'wait-storage-healthy' (pre-deploy): /nix/store/abc-wait-storage-healthy",
                    "storage on radon is degraded",
                ],
                hidden: 3,
            }
        );
    }

    #[test]
    fn test_error_context_too_small_for_the_command_keeps_only_the_cause() {
        let cause = "warning: the following units failed: storage-agent.service";
        assert_eq!(error_context(ACTIVATION_FAILURE, 1).lines, vec![cause]);
        assert!(error_context(ACTIVATION_FAILURE, 0).lines.is_empty());
    }

    #[test]
    fn test_error_context_of_a_nix_error_ends_at_its_last_error_line() {
        let error = "error:\n       … while evaluating 'toplevel'\n\n       \
                     error: attribute 'wine' missing\n       at /nix/store/abc-source/hosts/cerium.nix:12:5";

        assert_eq!(
            error_context(error, 10),
            ErrorContext {
                lines: vec![
                    "error:",
                    "… while evaluating 'toplevel'",
                    "error: attribute 'wine' missing",
                ],
                hidden: 1,
            }
        );
    }

    #[test]
    fn test_error_context_of_a_one_line_error_is_that_line() {
        assert_eq!(
            error_context("kernel linux-6.12.58 → linux-6.12.60", 10),
            ErrorContext {
                lines: vec!["kernel linux-6.12.58 → linux-6.12.60"],
                hidden: 0,
            }
        );
    }

    #[test]
    fn test_error_summary_uses_last_error_line() {
        let error = "error:\n       … while evaluating 'toplevel'\n\n       error: attribute 'wine' missing";

        assert_eq!(error_summary(error), "attribute 'wine' missing");
    }

    #[test]
    fn test_error_summary_skips_a_bare_error_header() {
        let error =
            "Failed to evaluate Nix expression:\nerror:\n       … while evaluating 'toplevel'";

        assert_eq!(error_summary(error), "Failed to evaluate Nix expression:");
    }
}
