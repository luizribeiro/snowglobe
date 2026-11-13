//! The error that ends a command, printed last on stderr

use crate::marks::indented;
use console::style;
use miette::Diagnostic;
use snowlib::error::SnowError;

pub fn render(error: &SnowError) -> String {
    styled(error, console::colors_enabled_stderr())
}

fn styled(error: &SnowError, colors: bool) -> String {
    let message = error.to_string();
    let (headline, rest) = message.split_once('\n').unwrap_or((&message, ""));
    let mut lines = vec![format!(
        "{} {}",
        crate::marks::failure().bold().force_styling(colors),
        style(headline).red().bold().force_styling(colors)
    )];

    lines.extend(indented(rest, "    "));
    for cause in snowlib::error::error_causes(error) {
        lines.extend(indented(&cause, "    "));
    }

    if let Some(help) = error.help() {
        let help = help.to_string();
        let (first, rest) = help.split_once('\n').unwrap_or((&help, ""));
        lines.push(format!("  help: {first}"));
        lines.extend(indented(rest, "    "));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use snowlib::error::{DeployError, FlakeError, NixError};

    #[test]
    fn test_render_shows_the_help_of_a_wrapped_error() {
        let error = SnowError::from(DeployError::SshAgentNotResponding { seconds: 10 });

        assert_eq!(
            styled(&error, false),
            "✗ The ssh-agent did not answer within 10s\n  \
             help: Every ssh connection would hang on it. Check it with `ssh-add -l`, then restart the agent or unset `SSH_AUTH_SOCK`."
        );
    }

    #[test]
    fn test_render_makes_the_headline_red_and_bold() {
        let error = SnowError::from(DeployError::SshAgentNotResponding { seconds: 10 });

        let headline = styled(&error, true).lines().next().unwrap().to_string();

        assert_eq!(
            headline,
            "\u{1b}[31m\u{1b}[1m✗\u{1b}[0m \u{1b}[31m\u{1b}[1mThe ssh-agent did not answer within 10s\u{1b}[0m"
        );
    }

    #[test]
    fn test_render_without_help_or_causes_is_the_headline_alone() {
        let error = SnowError::from(FlakeError::InvalidPath {
            path: "/srv/ops/flake.lock".into(),
        });

        assert_eq!(
            styled(&error, false),
            "✗ Path '/srv/ops/flake.lock' is neither a directory containing flake.nix nor a flake.nix file"
        );
    }

    #[test]
    fn test_render_indents_the_causes_under_the_headline() {
        let error = SnowError::from(NixError::MetadataFailed {
            host: "falcon".to_string(),
            source: Box::new(NixError::CommandFailed {
                source: std::io::Error::other("nix: command not found\n\nis nix installed?"),
            }),
        });

        assert_eq!(
            styled(&error, false),
            "✗ Failed to get metadata for host 'falcon'\n    \
             Nix command execution failed\n    \
             nix: command not found\n\n    \
             is nix installed?\n  \
             help: snow reads networking.hostName, networking.domain and the snowglobe options from the host's configuration. Trace the failing attribute (darwinConfigurations for nix-darwin hosts):\n    \
             \x20 nix eval --show-trace .#nixosConfigurations.falcon.config.<attribute>"
        );
    }

    #[test]
    fn test_render_indents_later_message_and_help_lines() {
        let error = SnowError::from(NixError::BuildFailed {
            host: "raven".to_string(),
            stderr: "error: builder for wine failed".to_string(),
        });

        assert_eq!(
            styled(&error, false),
            "✗ Failed to build host configuration for 'raven':\n    \
             error: builder for wine failed\n  \
             help: Check that the host exists and builds on its own (darwinConfigurations for nix-darwin hosts):\n    \
             \x20 nix build .#nixosConfigurations.raven.config.system.build.toplevel"
        );
    }
}
