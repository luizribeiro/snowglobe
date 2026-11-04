//! One-line summaries of long error messages

/// The line of an error worth showing on its own
///
/// Nix errors carry a long trace; the final `error:` line names the cause.
/// A failed command's own last words on stderr usually say what went wrong,
/// where its first line only says that it failed.
pub fn error_summary(error: &str) -> &str {
    let lines = || error.lines().map(str::trim);
    lines()
        .filter(|line| line.starts_with("error:"))
        .next_back()
        .or_else(|| last_stderr_line(error))
        .unwrap_or_else(|| lines().next().unwrap_or_default())
}

/// The last non-empty line of a failed command's "Stderr:" section
fn last_stderr_line(error: &str) -> Option<&str> {
    error
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "Stderr:")
        .skip(1)
        .take_while(|line| *line != "Stdout:")
        .filter(|line| !line.is_empty())
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_summary_without_error_line_uses_first_line() {
        assert_eq!(error_summary("worker crashed\nsignal 9"), "worker crashed");
    }

    #[test]
    fn test_error_summary_of_a_failed_command_is_its_last_stderr_line() {
        let error = "Command failed on host 'cerium'\n\n\
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
        let error =
            "  Command failed on host 'radon'\n  Stderr: \n    storage on radon is degraded\n";

        assert_eq!(error_summary(error), "storage on radon is degraded");
    }

    #[test]
    fn test_error_summary_of_a_command_with_empty_stderr_is_its_first_line() {
        let error = "Command failed on host 'radon'\n\nCommand: true\nExit code: 1\n\nStderr:\n";

        assert_eq!(error_summary(error), "Command failed on host 'radon'");
    }

    #[test]
    fn test_error_summary_uses_last_error_line() {
        let error = "error:\n       … while evaluating 'toplevel'\n\n       error: attribute 'wine' missing";

        assert_eq!(error_summary(error), "error: attribute 'wine' missing");
    }
}
