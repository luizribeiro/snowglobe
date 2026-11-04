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

    #[error(transparent)]
    #[diagnostic(transparent)]
    Provisioner(#[from] ProvisionerError),
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

    #[error("Failed to read hooks file {path}")]
    #[diagnostic(code(snow::nix::hooks_file_read_failed))]
    HooksFileReadFailed {
        path: String,
        #[source]
        source: std::io::Error,
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

    #[error(
        "{} of {total} {} failed: {}",
        failed.len(),
        if *total == 1 { "host" } else { "hosts" },
        failed.join(", ")
    )]
    #[diagnostic(
        code(snow::deploy::hosts_failed),
        help("See the per-host errors above")
    )]
    HostsFailed {
        total: usize,
        /// Each failed host with the step it failed at, e.g. "lanthanum (plan)"
        failed: Vec<String>,
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

    #[error(
        "Hook '{hook}' of host '{host}' references {path}, which is not present on this machine"
    )]
    #[diagnostic(
        code(snow::deploy::hook_path_missing),
        help("Local hooks run on the machine running snow. Use `localPkgs` (not `pkgs`) for packages in local hook commands so snow builds them for this machine. The path may also have been garbage-collected.")
    )]
    HookPathMissing {
        host: String,
        hook: String,
        path: String,
    },

    #[error("The ssh-agent did not answer within {seconds}s")]
    #[diagnostic(
        code(snow::deploy::ssh_agent_not_responding),
        help("Every ssh connection would hang on it. Check it with `ssh-add -l`, then restart the agent or unset SSH_AUTH_SOCK.")
    )]
    SshAgentNotResponding { seconds: u64 },

    #[error("Reboot of host '{host}' was not confirmed: {reason}")]
    #[diagnostic(
        code(snow::deploy::reboot_not_confirmed),
        help("Check that the deploy user can run `sudo snowglobe-activate reboot` on the host")
    )]
    RebootNotConfirmed { host: String, reason: String },

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

    #[error("No saved plan at {path}")]
    #[diagnostic(
        code(snow::io::no_saved_plan),
        help("Run `snow plan` first; each run saves its report for `snow plan --last`.")
    )]
    NoSavedPlan { path: PathBuf },

    #[error("Saved plan at {path} is unreadable")]
    #[diagnostic(
        code(snow::io::invalid_saved_plan),
        help("It may be from an older snow version. Run `snow plan` again to replace it.")
    )]
    InvalidSavedPlan {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Failed to create a temporary directory")]
    #[diagnostic(
        code(snow::io::temp_dir_failed),
        help("Check that $TMPDIR (or /tmp) is writable")
    )]
    TempDir {
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to access state file {path}")]
    #[diagnostic(code(snow::io::state_file))]
    StateFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Could not tell this machine's hostname")]
    #[diagnostic(
        code(snow::io::hostname),
        help("Name the host to rebuild with `--host`.")
    )]
    Hostname {
        #[source]
        source: std::io::Error,
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

    #[error("Path contains invalid UTF-8: {path}")]
    #[diagnostic(
        code(snow::bootstrap::invalid_path),
        help("Ensure directory paths contain only valid UTF-8 characters")
    )]
    InvalidPath { path: String },

    #[error("Required tool 'sops' not found")]
    #[diagnostic(
        code(snow::bootstrap::sops_missing),
        help("Install sops using nix-shell -p sops\n\nOr enter the bootstrap environment:\n  nix develop .#bootstrap")
    )]
    SopsMissing,

    #[error("No .sops.yaml found in flake directory: {path}")]
    #[diagnostic(
        code(snow::bootstrap::sops_yaml_not_found),
        help("Create a .sops.yaml file in your flake directory.\nSee https://github.com/getsops/sops for configuration format.")
    )]
    SopsYamlNotFound { path: String },

    #[error("Failed to read .sops.yaml: {path}")]
    #[diagnostic(
        code(snow::bootstrap::sops_yaml_read_failed),
        help("Ensure the file is readable and contains valid YAML")
    )]
    SopsYamlReadFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to write .sops.yaml: {path}")]
    #[diagnostic(
        code(snow::bootstrap::sops_yaml_write_failed),
        help("Ensure the file is writable")
    )]
    SopsYamlWriteFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Invalid .sops.yaml format: {reason}")]
    #[diagnostic(
        code(snow::bootstrap::sops_yaml_invalid),
        help("Ensure .sops.yaml has a valid 'keys:' section.\nSee https://github.com/getsops/sops for format details.")
    )]
    SopsYamlInvalid { reason: String },

    #[error("Host '{hostname}' already exists in .sops.yaml")]
    #[diagnostic(
        code(snow::bootstrap::sops_host_exists),
        help("Remove the existing entry for this host if you want to regenerate its key.\nOr use a different hostname.")
    )]
    SopsHostExists { hostname: String },

    #[error("sops updatekeys command failed:\n{stderr}")]
    #[diagnostic(
        code(snow::bootstrap::sops_updatekeys_failed),
        help("Check that sops is correctly configured and all keys are valid.\nRun manually: sops updatekeys <file>")
    )]
    SopsUpdateKeysFailed { stderr: String },
}

/// Errors related to image provisioning operations
#[derive(Debug, Error, Diagnostic)]
pub enum ProvisionerError {
    #[error("Provisioner '{provisioner}' requires manual key injection")]
    #[diagnostic(
        code(snow::provisioner::manual_required),
        help("Copy the SSH keys to the target's /etc/ssh/ directory manually")
    )]
    ManualRequired { provisioner: String },

    #[error("Unknown provisioner type: {name}")]
    #[diagnostic(
        code(snow::provisioner::unknown),
        help("Available provisioners: manual, sd-image")
    )]
    UnknownProvisioner { name: String },

    #[error("Invalid target '{target}' for {provisioner} provisioner: {reason}")]
    #[diagnostic(
        code(snow::provisioner::invalid_target),
        help("Verify the target matches the provisioner type:\n  - sd-image, raw: /dev/sdX (block device)\n  - proxmox-vma: hostname\n  - remote: user@host")
    )]
    InvalidTarget {
        provisioner: String,
        target: String,
        reason: String,
    },

    #[error("Missing required tool for {provisioner} provisioner: {tool}")]
    #[diagnostic(code(snow::provisioner::missing_dependency), help("{install_hint}"))]
    MissingDependency {
        provisioner: String,
        tool: String,
        install_hint: String,
    },

    #[error("{provisioner} provisioner failed to inject keys: {reason}")]
    #[diagnostic(
        code(snow::provisioner::injection_failed),
        help("Common causes:\n  - Image file not found or corrupted\n  - Insufficient permissions to access the image\n  - Partition not found in image")
    )]
    InjectionFailed { provisioner: String, reason: String },

    #[error("{provisioner} provisioner: {command} command failed:\n{stderr}")]
    #[diagnostic(
        code(snow::provisioner::injection_command_failed),
        help("The key injection tool encountered an error. Common issues:\n  - Partition not found in image\n  - Filesystem corruption\n  - Insufficient space in image")
    )]
    InjectionCommandFailed {
        provisioner: String,
        command: String,
        stderr: String,
    },

    #[error("Failed to build image for host '{host}':\n{stderr}")]
    #[diagnostic(
        code(snow::provisioner::image_build_failed),
        help("Check that the image attribute exists and can be built:\n  nix build .#nixosConfigurations.{host}.config.system.build.<imageAttr>")
    )]
    ImageBuildFailed {
        host: String,
        stderr: String,
        #[source]
        source: std::io::Error,
    },

    #[error("No bootstrap configuration found for host '{host}'")]
    #[diagnostic(
        code(snow::provisioner::no_config),
        help(
            "Add bootstrap configuration to your NixOS config:\n\n  snowglobe.bootstrap = {{\n    provisioner = \"manual\";  # or \"sd-image\", \"raw\", etc.\n  }};"
        )
    )]
    NoBootstrapConfig { host: String },

    #[error("No image attribute configured for host '{host}'")]
    #[diagnostic(
        code(snow::provisioner::no_image_attr),
        help("Specify the image build attribute in your NixOS config:\n\n  snowglobe.bootstrap.imageAttr = \"config.system.build.sdImage\";")
    )]
    NoImageAttr { host: String },

    #[error("{provisioner} provisioner does not support deployment: {reason}")]
    #[diagnostic(
        code(snow::provisioner::deployment_not_supported),
        help("This provisioner only generates images for manual deployment.\nUse a different provisioner or deploy the image manually.")
    )]
    DeploymentNotSupported { provisioner: String, reason: String },

    #[error("{provisioner} deployment failed: {reason}")]
    #[diagnostic(
        code(snow::provisioner::deployment_failed),
        help("Check the target is accessible and has sufficient permissions.\nFor block devices, ensure you have write access (try with sudo).\nFor Proxmox, ensure SSH access is configured.")
    )]
    DeploymentFailed { provisioner: String, reason: String },

    #[error("{provisioner} deployment to {target} was cancelled")]
    #[diagnostic(code(snow::provisioner::deployment_cancelled))]
    DeploymentCancelled { provisioner: String, target: String },
}

/// The error's message followed by each source's, joined with ": ".
///
/// Sources whose text the message already quotes are skipped, since many
/// variants embed their stderr in the message too.
pub fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut chain = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !chain.contains(&text) {
            chain.push_str(": ");
            chain.push_str(&text);
        }
        source = cause.source();
    }
    chain
}

/// Whether `error` or any of its sources is an I/O timeout
pub fn is_timeout(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(err) = current {
        if err
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::TimedOut)
        {
            return true;
        }
        current = err.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_timeout_finds_a_timeout_behind_ssh_failure() {
        let error = SnowError::from(DeployError::SshFailed {
            destination: "pusher@curium.example.com".to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "ssh did not finish within 60s",
            ),
        });

        assert!(is_timeout(&error));
    }

    #[test]
    fn test_is_timeout_ignores_other_failures() {
        let error = SnowError::from(DeployError::SshFailed {
            destination: "pusher@curium.example.com".to_string(),
            source: std::io::Error::other("Connection refused"),
        });

        assert!(!is_timeout(&error));
    }

    #[test]
    fn test_error_chain_includes_the_reason_behind_a_deploy_failure() {
        let error = SnowError::from(DeployError::Failed {
            host: "lanthanum".to_string(),
            source: std::io::Error::other(
                "Reboot required (kernel changed) but host policy is 'never'",
            ),
        });

        assert_eq!(
            error_chain(&error),
            "Failed to deploy to host 'lanthanum': Reboot required (kernel changed) but host policy is 'never'"
        );
    }

    #[test]
    fn test_error_chain_follows_nested_sources() {
        let error = SnowError::from(NixError::MetadataFailed {
            host: "ferrum".to_string(),
            source: Box::new(NixError::CommandFailed {
                source: std::io::Error::other("nix: command not found"),
            }),
        });

        assert_eq!(
            error_chain(&error),
            "Failed to get metadata for host 'ferrum': Nix command execution failed: nix: command not found"
        );
    }

    #[test]
    fn test_error_chain_skips_sources_already_in_the_message() {
        let error = NixError::BuildFailed {
            host: "radon".to_string(),
            stderr: "error: builder for wine failed".to_string(),
            source: std::io::Error::other("error: builder for wine failed"),
        };

        assert_eq!(
            error_chain(&error),
            "Failed to build host configuration for 'radon':\nerror: builder for wine failed"
        );
    }

    #[test]
    fn test_error_chain_of_error_without_source() {
        let error = DeployError::HookPathMissing {
            host: "samarium".to_string(),
            hook: "wait-storage-healthy".to_string(),
            path: "/nix/store/715sqm-kubernetes-storage-healthy".to_string(),
        };

        assert_eq!(error_chain(&error), error.to_string());
    }

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

    #[test]
    fn test_provisioner_error_converts_to_snow_error() {
        let prov_err = ProvisionerError::ManualRequired {
            provisioner: "manual".to_string(),
        };
        let snow_err: SnowError = prov_err.into();
        assert!(snow_err.to_string().contains("manual"));
    }

    #[test]
    fn test_unknown_provisioner_error() {
        let error = ProvisionerError::UnknownProvisioner {
            name: "foobar".to_string(),
        };
        assert!(error.to_string().contains("Unknown provisioner"));
        assert!(error.to_string().contains("foobar"));
    }

    #[test]
    fn test_invalid_target_error() {
        let error = ProvisionerError::InvalidTarget {
            provisioner: "proxmox-vma".to_string(),
            target: "/dev/sdc".to_string(),
            reason: "expects a hostname".to_string(),
        };
        assert!(error.to_string().contains("/dev/sdc"));
        assert!(error.to_string().contains("proxmox-vma"));
        assert!(error.to_string().contains("expects a hostname"));
    }

    #[test]
    fn test_injection_failed_includes_provisioner_and_reason() {
        let error = ProvisionerError::InjectionFailed {
            provisioner: "sd-image".to_string(),
            reason: "permission denied".to_string(),
        };
        assert!(error.to_string().contains("sd-image"));
        assert!(error.to_string().contains("permission denied"));
    }

    #[test]
    fn test_injection_command_failed_includes_details() {
        let error = ProvisionerError::InjectionCommandFailed {
            provisioner: "sd-image".to_string(),
            command: "guestfish".to_string(),
            stderr: "guestfish: error: no root device found".to_string(),
        };
        assert!(error.to_string().contains("sd-image"));
        assert!(error.to_string().contains("guestfish"));
        assert!(error.to_string().contains("no root device found"));
    }

    #[test]
    fn test_missing_dependency_error() {
        let error = ProvisionerError::MissingDependency {
            provisioner: "sd-image".to_string(),
            tool: "guestfish".to_string(),
            install_hint: "nix-shell -p libguestfs".to_string(),
        };
        assert!(error.to_string().contains("sd-image"));
        assert!(error.to_string().contains("guestfish"));
    }

    #[test]
    fn test_no_image_attr_error() {
        let error = ProvisionerError::NoImageAttr {
            host: "hydrogen".to_string(),
        };
        assert!(error.to_string().contains("hydrogen"));
        assert!(error.to_string().contains("No image attribute"));
    }

    #[test]
    fn test_image_build_failed_error() {
        let io_error = std::io::Error::other("build failed");
        let error = ProvisionerError::ImageBuildFailed {
            host: "carbon".to_string(),
            stderr: "error: attribute not found".to_string(),
            source: io_error,
        };
        assert!(error.to_string().contains("carbon"));
        assert!(error.to_string().contains("attribute not found"));
    }

    #[test]
    fn test_no_bootstrap_config_error() {
        let error = ProvisionerError::NoBootstrapConfig {
            host: "myhost".to_string(),
        };
        assert!(error.to_string().contains("myhost"));
        assert!(error.to_string().contains("No bootstrap configuration"));
    }
}
