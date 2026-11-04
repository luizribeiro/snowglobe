//! Questions asked on the terminal, styled the same in every command

use snowlib::error::{IoError, SnowError};

pub fn prompt_error(error: dialoguer::Error) -> SnowError {
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
