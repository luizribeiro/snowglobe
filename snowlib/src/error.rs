use miette::Diagnostic;
use std::path::PathBuf;
use thiserror::Error;

/// Result type alias for snow operations
pub type Result<T> = std::result::Result<T, SnowError>;

/// Main error type for the snow CLI
#[derive(Debug, Error, Diagnostic)]
pub enum SnowError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Nix(#[from] NixError),

    #[error(transparent)]
    #[diagnostic(transparent)]
    Flake(#[from] FlakeError),

    #[error(transparent)]
    #[diagnostic(transparent)]
    Deploy(#[from] DeployError),

    #[error(transparent)]
    #[diagnostic(transparent)]
    Io(#[from] IoError),

    #[error(transparent)]
    #[diagnostic(transparent)]
    Bootstrap(#[from] BootstrapError),
}

/// Errors related to Nix evaluation and building
#[derive(Debug, Error, Diagnostic)]
pub enum NixError {
    #[error("Failed to evaluate Nix expression:\n{stderr}")]
    #[diagnostic(
        code(snow::nix::eval_failed),
        help("Check your flake.nix syntax and ensure the expression is valid.\nCommon issues:\n  - Missing 'or' defaults for optional attributes\n  - Syntax errors in filter expressions\n  - Undefined attributes")
    )]
    EvalFailed {
        stderr: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse Nix command output as JSON")]
    #[diagnostic(
        code(snow::nix::output_parse_failed),
        help("This is likely a bug in snow. The Nix command returned invalid JSON.")
    )]
    OutputParseFailed {
        output: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("Failed to build host configuration for '{host}':\n{stderr}")]
    #[diagnostic(
        code(snow::nix::build_failed),
        help("Check that the host exists in your flake's nixosConfigurations and can be built:\n  nix build .#nixosConfigurations.{host}.config.system.build.toplevel")
    )]
    BuildFailed {
        host: String,
        stderr: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to get metadata for host '{host}'")]
    #[diagnostic(
        code(snow::nix::metadata_failed),
        help("Ensure the host has the required configuration attributes")
    )]
    MetadataFailed {
        host: String,
        #[source]
        source: Box<NixError>,
    },

    #[error("Nix command execution failed")]
    #[diagnostic(code(snow::nix::command_failed))]
    CommandFailed {
        #[source]
        source: std::io::Error,
    },
}

/// Errors related to flake detection and validation
#[derive(Debug, Error, Diagnostic)]
pub enum FlakeError {
    #[error("Could not find flake.nix in {start_dir} or any parent directory")]
    #[diagnostic(
        code(snow::flake::not_found),
        help("snowglobe requires a flake.nix with nixosConfigurations to operate.\nRun this command from within a NixOS flake directory, or use --flake-path to specify the location.")
    )]
    NotFound { start_dir: PathBuf },

    #[error("Flake path does not exist: {path}")]
    #[diagnostic(
        code(snow::flake::path_not_found),
        help("Provide a valid path to a directory containing flake.nix or to a flake.nix file")
    )]
    PathNotFound { path: PathBuf },

    #[error("No flake.nix found in specified directory: {path}")]
    #[diagnostic(
        code(snow::flake::missing_in_dir),
        help("The directory exists but doesn't contain a flake.nix file.\nProvide a path to a directory containing flake.nix or the flake.nix file itself")
    )]
    MissingInDir { path: PathBuf },

    #[error(
        "Path must be either a directory containing flake.nix or the flake.nix file itself: {path}"
    )]
    #[diagnostic(code(snow::flake::invalid_path))]
    InvalidPath { path: PathBuf },

    #[error("Invalid host(s): {invalid_hosts}\nAvailable hosts: {available_hosts}")]
    #[diagnostic(
        code(snow::flake::invalid_hosts),
        help("Check your flake's nixosConfigurations for the list of available hosts.\nYou can run 'snow list' to see all configured hosts.")
    )]
    InvalidHosts {
        invalid_hosts: String,
        available_hosts: String,
    },
}

/// Errors related to deployment operations
#[derive(Debug, Error, Diagnostic)]
pub enum DeployError {
    #[error("Failed to deploy to host '{host}'")]
    #[diagnostic(code(snow::deploy::failed))]
    Failed {
        host: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Command failed on host '{host}'\n\nCommand: {command}\nExit code: {exit_code}\n\nStderr:\n{stderr}{}", if stdout.is_empty() { String::new() } else { format!("\n\nStdout:\n{}", stdout) })]
    #[diagnostic(code(snow::deploy::command_failed))]
    CommandFailed {
        host: String,
        command: String,
        exit_code: i32,
        stderr: String,
        stdout: String,
    },

    #[error("SSH connection failed to {destination}")]
    #[diagnostic(
        code(snow::deploy::ssh_failed),
        help("Ensure the host is reachable and SSH is configured correctly")
    )]
    SshFailed {
        destination: String,
        #[source]
        source: std::io::Error,
    },
}

/// Errors related to I/O operations
#[derive(Debug, Error, Diagnostic)]
pub enum IoError {
    #[error("Failed to get current directory")]
    #[diagnostic(code(snow::io::current_dir))]
    CurrentDir {
        #[source]
        source: std::io::Error,
    },

    #[error("JSON serialization failed")]
    #[diagnostic(code(snow::io::json_serialize))]
    JsonSerialize {
        #[source]
        source: serde_json::Error,
    },
}

/// Errors related to bootstrap operations (key generation, provisioning)
#[derive(Debug, Error, Diagnostic)]
pub enum BootstrapError {
    #[error("Required tool '{tool}' not found")]
    #[diagnostic(
        code(snow::bootstrap::missing_dependency),
        help("Install the missing tool using nix-shell -p <tool_name>\n\nOr enter the bootstrap environment:\n  nix develop .#bootstrap")
    )]
    MissingDependency { tool: String },

    #[error("Failed to generate SSH host key")]
    #[diagnostic(
        code(snow::bootstrap::keygen_failed),
        help("Ensure ssh-keygen is available and the output directory is writable")
    )]
    KeyGenFailed {
        #[source]
        source: std::io::Error,
    },

    #[error("SSH key generation failed:\n{stderr}")]
    #[diagnostic(
        code(snow::bootstrap::keygen_command_failed),
        help("Check that ssh-keygen is working correctly:\n  ssh-keygen -t ed25519 -f /tmp/test_key -N ''")
    )]
    KeyGenCommandFailed { stderr: String },

    #[error("Failed to derive age public key from SSH key")]
    #[diagnostic(
        code(snow::bootstrap::age_derivation_failed),
        help("Ensure ssh-to-age is installed and the SSH public key is valid:\n  ssh-to-age -i <public_key_file>")
    )]
    AgeDerivationFailed {
        #[source]
        source: std::io::Error,
    },

    #[error("Age key derivation failed:\n{stderr}")]
    #[diagnostic(
        code(snow::bootstrap::age_command_failed),
        help("Check that ssh-to-age can process the key:\n  ssh-to-age -i <public_key_file>")
    )]
    AgeCommandFailed { stderr: String },

    #[error("Failed to create secure temporary directory")]
    #[diagnostic(
        code(snow::bootstrap::temp_dir_failed),
        help("Ensure /tmp is writable and has sufficient space.\nCheck permissions with: ls -ld /tmp")
    )]
    TempDirFailed {
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_dependency_error_message() {
        let error = BootstrapError::MissingDependency {
            tool: "ssh-keygen".to_string(),
        };
        assert!(error.to_string().contains("ssh-keygen"));
        assert!(error.to_string().contains("not found"));
    }

    #[test]
    fn test_bootstrap_error_converts_to_snow_error() {
        let bootstrap_err = BootstrapError::MissingDependency {
            tool: "ssh-to-age".to_string(),
        };
        let snow_err: SnowError = bootstrap_err.into();
        assert!(snow_err.to_string().contains("ssh-to-age"));
    }

    #[test]
    fn test_keygen_command_failed_includes_stderr() {
        let error = BootstrapError::KeyGenCommandFailed {
            stderr: "Permission denied".to_string(),
        };
        assert!(error.to_string().contains("Permission denied"));
    }

    #[test]
    fn test_age_command_failed_includes_stderr() {
        let error = BootstrapError::AgeCommandFailed {
            stderr: "Invalid key format".to_string(),
        };
        assert!(error.to_string().contains("Invalid key format"));
    }

    #[test]
    fn test_keygen_failed_with_source() {
        let io_error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let error = BootstrapError::KeyGenFailed { source: io_error };
        assert!(error
            .to_string()
            .contains("Failed to generate SSH host key"));
    }

    #[test]
    fn test_age_derivation_failed_with_source() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "command not found");
        let error = BootstrapError::AgeDerivationFailed { source: io_error };
        assert!(error
            .to_string()
            .contains("Failed to derive age public key"));
    }

    #[test]
    fn test_temp_dir_failed_with_source() {
        let io_error = std::io::Error::other("disk full");
        let error = BootstrapError::TempDirFailed { source: io_error };
        assert!(error
            .to_string()
            .contains("Failed to create secure temporary directory"));
    }
}
