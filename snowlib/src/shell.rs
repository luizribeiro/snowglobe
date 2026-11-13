//! Quoting for shell command lines: those run on a host over ssh, which joins
//! its arguments with spaces for the host's login shell to parse again, and
//! those snow suggests running.

use std::borrow::Cow;

/// Quotes `word` so the host's shell reads it back unchanged. Words of only
/// safe characters, like store paths, are left bare to keep commands readable
/// in error messages.
pub fn quote(word: &str) -> Cow<'_, str> {
    shell_escape::unix::escape(word.into())
}

/// Quotes each word and joins them into one command line
pub fn join<'a>(words: impl IntoIterator<Item = &'a str>) -> String {
    words.into_iter().map(quote).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_leaves_safe_words_bare() {
        assert_eq!(
            quote("/nix/store/abc123-nixos-system-condor-24.11"),
            "/nix/store/abc123-nixos-system-condor-24.11"
        );
        assert_eq!(quote("dry-activate"), "dry-activate");
    }

    #[test]
    fn test_quote_keeps_spaces_in_one_word() {
        assert_eq!(quote("/var/lib/my disk.raw"), "'/var/lib/my disk.raw'");
    }

    #[test]
    fn test_quote_escapes_single_quotes() {
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn test_quote_stops_variable_and_command_expansion() {
        assert_eq!(quote("$HOME"), "'$HOME'");
        assert_eq!(quote("$(reboot)"), "'$(reboot)'");
    }

    #[test]
    fn test_quote_keeps_an_empty_word() {
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn test_join_quotes_each_word() {
        assert_eq!(
            join(["systemctl", "show", "dev-disk-by\\x2dlabel-nixos.device"]),
            r"systemctl show 'dev-disk-by\x2dlabel-nixos.device'"
        );
    }

    #[tokio::test]
    async fn test_quoted_words_survive_a_real_shell() {
        let words = ["printf", "%s\\n", "it's", "$HOME", "a b", ""];
        let output = tokio::process::Command::new("sh")
            .args(["-c", &join(words)])
            .output()
            .await
            .unwrap();

        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "it's\n$HOME\na b\n\n"
        );
    }
}
