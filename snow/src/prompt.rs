//! Questions asked on the terminal, styled the same in every command

use crate::cursor::{self, Cursor, KeepsCursor};
use crate::signals;
use dialoguer::console::Term;
use dialoguer::{theme::ColorfulTheme, Confirm, Select};
use snowlib::error::{IoError, SnowError};
use snowlib::Result;

/// Asks a yes/no question; anything but an explicit yes is a no
pub fn confirm(question: &str) -> Result<bool> {
    ask(|term| {
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(question)
            .default(false)
            .interact_on(term)
    })
}

/// Asks to pick one of `items`, the first one by default
pub fn select(question: &str, items: &[&str]) -> Result<usize> {
    ask(|term| {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt(question)
            .items(items)
            .default(0)
            .interact_on(term)
    })
}

fn ask<T>(question: impl FnOnce(&Term) -> dialoguer::Result<T>) -> Result<T> {
    let term = Term::read_write_pair(std::io::stdin(), KeepsCursor(std::io::stderr()));
    let answer = Cursor::stderr().shown_while(|| question(&term));
    if answer.as_ref().is_err_and(is_interrupted) {
        cursor::exit(signals::INTERRUPTED);
    }
    answer.map_err(prompt_error)
}

/// Ctrl-C at a prompt is a key, not a signal, so console raises SIGINT
/// itself and fails the prompt with this error. Exiting here keeps that
/// error from racing the signal handler onto the screen.
fn is_interrupted(error: &dialoguer::Error) -> bool {
    let dialoguer::Error::IO(source) = error;
    source.kind() == std::io::ErrorKind::Interrupted
}

fn prompt_error(error: dialoguer::Error) -> SnowError {
    let dialoguer::Error::IO(source) = error;
    IoError::Prompt { source }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_only_ctrl_c_interrupts_a_prompt() {
        let ctrl_c = std::io::Error::new(std::io::ErrorKind::Interrupted, "read interrupted");
        let closed = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "stdin closed");

        assert!(is_interrupted(&dialoguer::Error::IO(ctrl_c)));
        assert!(!is_interrupted(&dialoguer::Error::IO(closed)));
    }

    #[test]
    fn test_prompt_error_keeps_the_terminal_failure_as_its_cause() {
        let closed = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "stdin closed");

        let error = prompt_error(dialoguer::Error::IO(closed));

        assert!(matches!(error, SnowError::Io(IoError::Prompt { .. })));
        assert_eq!(error.to_string(), "Failed to read your answer");
        assert_eq!(
            std::error::Error::source(&error).unwrap().to_string(),
            "stdin closed"
        );
    }
}
