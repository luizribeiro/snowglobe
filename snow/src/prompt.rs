//! Questions asked on the terminal, styled the same in every command

use crate::cursor::{Cursor, KeepsCursor};
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
    Cursor::stderr()
        .shown_while(|| question(&term))
        .map_err(prompt_error)
}

fn prompt_error(error: dialoguer::Error) -> SnowError {
    let dialoguer::Error::IO(source) = error;
    IoError::Prompt { source }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

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
