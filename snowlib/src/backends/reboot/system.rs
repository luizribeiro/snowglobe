use super::{BootComponent, RebootBackend, RebootCheck, RebootDetectionConfig, RebootReason};
use crate::backends::deploy::{helper_found, DeployTarget, HELPER_CHECK};
use crate::error::{DeployError, Result, SnowError};
use crate::nix::StorePath;
use crate::process::CommandResult;
use crate::shell::quote;
use crate::ssh_options::{run_bounded, ssh_options, Keepalive};
use async_trait::async_trait;
use std::time::Duration;

/// Prefers the snowglobe-activate helper because deploy users are only granted
/// passwordless sudo for it; hosts without the snowglobe module fall back to systemctl.
const REBOOT_COMMAND: &str = "if command -v snowglobe-activate >/dev/null; \
     then sudo snowglobe-activate reboot; \
     else sudo systemctl reboot; fi";

const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

/// ssh exits with 255 when the connection drops, which is expected while the host goes down
const SSH_CONNECTION_LOST: i32 = 255;

/// System reboot detection backend
pub struct SystemRebootBackend {
    config: RebootDetectionConfig,
    ssh_user: Option<String>,
    ssh_command: Vec<String>,
}

impl SystemRebootBackend {
    /// Create a new backend with the given configuration
    pub fn new(config: RebootDetectionConfig) -> Self {
        Self {
            config,
            ssh_user: None,
            ssh_command: vec!["ssh".to_string()],
        }
    }

    /// Create a backend with default configuration
    pub fn with_defaults() -> Self {
        Self::new(RebootDetectionConfig::default())
    }

    /// Set the SSH user for connections
    pub fn with_ssh_user(mut self, user: impl Into<String>) -> Self {
        self.ssh_user = Some(user.into());
        self
    }

    /// How long to wait for a rebooted host to answer again
    pub fn with_reboot_max_wait(mut self, seconds: u32) -> Self {
        self.config.reboot_max_wait = seconds;
        self
    }

    fn get_destination(&self, target: &DeployTarget) -> String {
        target.ssh_destination_as(self.ssh_user.as_deref())
    }

    /// Check if kernel or initrd changed (fast check)
    async fn check_hard_requirements(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<Vec<RebootReason>> {
        let mut reasons = Vec::new();
        for component in BootComponent::checked(&self.config) {
            let booted_path = format!("/run/booted-system/{}", component.link());
            let new_path = format!("{}/{}", new_system.as_str(), component.link());
            if self
                .compare_symlinks(target, &booted_path, &new_path)
                .await?
            {
                continue;
            }
            let old_target = self.read_symlink_target(target, &booted_path).await?;
            let new_target = self.read_symlink_target(target, &new_path).await?;
            reasons.extend(component.reason(old_target, new_target));
        }

        Ok(reasons)
    }

    /// Read the target of a symlink on the remote host
    async fn read_symlink_target(
        &self,
        target: &DeployTarget,
        symlink_path: &str,
    ) -> Result<String> {
        let cmd = format!("readlink -f {}", quote(symlink_path));
        let result = self.ssh_exec_full(target, &cmd).await?;

        if !result.status.success() {
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(format!(
                    "Failed to resolve path {}: {}",
                    symlink_path,
                    String::from_utf8_lossy(&result.stderr)
                )),
            }
            .into());
        }

        Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
    }

    /// Compare two symlinks on the remote host
    async fn compare_symlinks(
        &self,
        target: &DeployTarget,
        path1: &str,
        path2: &str,
    ) -> Result<bool> {
        let cmd = format!(
            r#"[ "$(readlink {})" = "$(readlink {})" ]"#,
            quote(path1),
            quote(path2)
        );

        let status = self.ssh_exec(target, &cmd).await?;
        Ok(status.success())
    }

    /// Through snowglobe-activate when the host has it, as that is all a
    /// deploy user may sudo
    async fn dry_activate_command(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<String> {
        let system = quote(new_system.as_str());
        let helper_check = self.ssh_exec_full(target, &HELPER_CHECK.join(" ")).await?;
        if helper_found(&self.get_destination(target), &helper_check)? {
            Ok(format!("sudo snowglobe-activate {system} dry-activate"))
        } else {
            Ok(format!(
                "sudo {system}/bin/switch-to-configuration dry-activate"
            ))
        }
    }

    /// Check if any critical services would be restarted
    async fn check_critical_services(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<Vec<RebootReason>> {
        let dry_activate_cmd = self.dry_activate_command(target, new_system).await?;

        let timeout = Duration::from_secs(self.config.dry_activate_timeout.into());
        let result = self
            .ssh_exec_with_timeout(target, &dry_activate_cmd, timeout)
            .await?;
        if !result.status.success() {
            return Err(dry_activate_failed(target, &dry_activate_cmd, &result));
        }

        Ok(super::critical_restarts(
            &String::from_utf8_lossy(&result.stderr),
            &self.config.critical_services,
        ))
    }

    /// Build SSH command arguments
    fn build_ssh_args(&self, target: &DeployTarget, command: &str) -> Vec<String> {
        let mut args = ssh_options(
            self.config.ssh_connect_timeout.into(),
            Keepalive::Probe,
            target.port,
        );
        args.push(self.get_destination(target));
        args.push(command.to_string());
        args
    }

    /// Execute an SSH command and return the status
    async fn ssh_exec(
        &self,
        target: &DeployTarget,
        command: &str,
    ) -> Result<std::process::ExitStatus> {
        Ok(self.ssh_exec_full(target, command).await?.status)
    }

    /// Execute an SSH probe and return the full result
    async fn ssh_exec_full(&self, target: &DeployTarget, command: &str) -> Result<CommandResult> {
        let timeout = Duration::from_secs(self.config.probe_timeout.into());
        self.ssh_exec_with_timeout(target, command, timeout).await
    }

    async fn ssh_exec_with_timeout(
        &self,
        target: &DeployTarget,
        command: &str,
        timeout: Duration,
    ) -> Result<CommandResult> {
        let (program, leading_args) = self
            .ssh_command
            .split_first()
            .expect("ssh_command is never empty");
        let args = self.build_ssh_args(target, command);
        let args_refs: Vec<&str> = leading_args
            .iter()
            .chain(args.iter())
            .map(|s| s.as_str())
            .collect();

        run_bounded(program, &args_refs, timeout)
            .await
            .map_err(|e| {
                DeployError::SshFailed {
                    destination: self.get_destination(target),
                    source: e,
                }
                .into()
            })
    }

    /// Read a file from the remote host via SSH
    async fn ssh_read_file(&self, target: &DeployTarget, remote_path: &str) -> Result<String> {
        let cmd = format!("cat {}", quote(remote_path));
        let result = self.ssh_exec_full(target, &cmd).await?;

        if !result.status.success() {
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(format!(
                    "Failed to read file {}: {}",
                    remote_path,
                    String::from_utf8_lossy(&result.stderr)
                )),
            }
            .into());
        }

        Ok(String::from_utf8_lossy(&result.stdout).into_owned())
    }

    async fn read_boot_id(&self, target: &DeployTarget) -> Result<String> {
        Ok(self
            .ssh_read_file(target, BOOT_ID_PATH)
            .await?
            .trim()
            .to_string())
    }

    /// Ask the host to reboot, failing if the command was refused (e.g. by sudo)
    async fn request_reboot(&self, target: &DeployTarget) -> Result<()> {
        let result = self.ssh_exec_full(target, REBOOT_COMMAND).await?;

        match result.status.code() {
            Some(0) | Some(SSH_CONNECTION_LOST) | None => Ok(()),
            Some(exit_code) => Err(DeployError::CommandFailed {
                host: target.hostname.clone(),
                command: REBOOT_COMMAND.to_string(),
                exit_code,
                stderr: String::from_utf8_lossy(&result.stderr).trim().to_string(),
                stdout: String::from_utf8_lossy(&result.stdout).trim().to_string(),
            }
            .into()),
        }
    }

    /// Poll until the host answers with a boot ID different from `previous_boot_id`
    async fn wait_for_new_boot(&self, target: &DeployTarget, previous_boot_id: &str) -> Result<()> {
        let max_wait = Duration::from_secs(self.config.reboot_max_wait.into());
        let deadline = tokio::time::Instant::now() + max_wait;
        let retry_interval = Duration::from_secs(self.config.reboot_retry_interval.max(1).into());
        let mut still_on_previous_boot = false;
        let mut last_error = None;

        let mut probed = false;
        loop {
            tokio::time::sleep_until((tokio::time::Instant::now() + retry_interval).min(deadline))
                .await;
            if probed && tokio::time::Instant::now() >= deadline {
                break;
            }
            probed = true;
            match tokio::time::timeout_at(deadline, self.read_boot_id(target)).await {
                Ok(Ok(boot_id)) if boot_id != previous_boot_id => return Ok(()),
                Ok(Ok(_)) => still_on_previous_boot = true,
                Ok(Err(e)) => {
                    still_on_previous_boot = false;
                    tracing::debug!("{}: not back yet: {}", target.hostname, e);
                    last_error = Some(crate::error::error_chain(&e).trim_end().to_string());
                }
                // The deadline cut this check short; an earlier check's
                // error says more about why the host is not back
                Err(_elapsed) => {
                    still_on_previous_boot = false;
                    last_error.get_or_insert_with(|| "the host never answered".to_string());
                    break;
                }
            }
        }

        let outcome = if still_on_previous_boot {
            "boot ID is unchanged, so the host never rebooted"
        } else {
            "host did not come back online"
        };
        let detail = last_error
            .filter(|_| !still_on_previous_boot)
            .map(|error| format!(" (last error: {error})"))
            .unwrap_or_default();

        Err(DeployError::RebootNotConfirmed {
            host: target.hostname.clone(),
            reason: format!("{outcome} within {}s{detail}", max_wait.as_secs()),
        }
        .into())
    }
}

impl Default for SystemRebootBackend {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[async_trait]
impl RebootBackend for SystemRebootBackend {
    async fn check_reboot_required(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<RebootCheck> {
        let mut reasons = self.check_hard_requirements(target, new_system).await?;
        // dry-activate is slow; skip it when a reboot is needed anyway
        if reasons.is_empty() && !self.config.critical_services.is_empty() {
            reasons = self.check_critical_services(target, new_system).await?;
        }
        Ok(RebootCheck::from_reasons(reasons))
    }

    async fn reboot_and_wait(&self, target: &DeployTarget) -> Result<()> {
        let boot_id = self.read_boot_id(target).await?;
        self.request_reboot(target).await?;
        self.wait_for_new_boot(target, &boot_id).await
    }
}

/// Without dry-activate's restart list, snow can't tell whether the new
/// system needs a reboot, so it must not guess that it doesn't. Stderr gets
/// its own section, whose last line a deployment shows as the cause.
fn dry_activate_failed(target: &DeployTarget, command: &str, result: &CommandResult) -> SnowError {
    DeployError::Failed {
        host: target.hostname.clone(),
        source: std::io::Error::other(format!(
            "Could not tell whether the new system needs a reboot: `{command}` failed with {}. \
             If sudo refused it, import the snowglobe module on the host and add the SSH \
             user to snowglobe.deploy.allowedUsers.\
             \n\nStderr:\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        )),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SnowError;

    /// A host simulated by a script that stands in for `ssh`. The host starts on
    /// boot "boot-1"; `on_reboot` is the shell snippet run for the reboot command.
    /// Writing `$state/next_boot_id` makes the host come back on that boot after
    /// one unreachable poll, and `$state/stale_polls` delays that by N polls that
    /// still answer with the old boot. `$state/refuse` names a command that
    /// sudo refuses, `$state/helper_status` is how the check for
    /// snowglobe-activate exits (0, it's installed, by default), and
    /// `$state/dry_activate_output` is what dry-activate prints.
    struct FakeHost {
        state: tempfile::TempDir,
    }

    impl FakeHost {
        fn new(on_reboot: &str) -> Self {
            let state = tempfile::tempdir().unwrap();
            let state_path = state.path().display();
            std::fs::write(state.path().join("boot_id"), "boot-1\n").unwrap();
            std::fs::write(
                state.path().join("ssh.sh"),
                format!(
                    r#"
state='{state_path}'
cmd="${{@: -1}}"
echo "$cmd" >> "$state/commands"
if [ -e "$state/hang_on" ]; then
  case "$cmd" in *"$(cat "$state/hang_on")"*) exec sleep 30 ;; esac
fi
if [ -e "$state/refuse" ]; then
  case "$cmd" in *"$(cat "$state/refuse")"*) echo "sudo: a password is required" >&2; exit 1 ;; esac
fi
case "$cmd" in
  "command -v snowglobe-activate")
    exit "$(cat "$state/helper_status" 2>/dev/null || echo 0)"
    ;;
  *dry-activate)
    if [ -e "$state/dry_activate_output" ]; then cat "$state/dry_activate_output" >&2; fi
    ;;
  *readlink*)
    eval "${{cmd//\/run\/booted-system/$state/booted}}"
    ;;
  *boot_id*)
    if [ -e "$state/unreachable" ]; then cat "$state/unreachable" >&2; exit 255; fi
    stale=$(cat "$state/stale_polls" 2>/dev/null || echo 0)
    if [ "$stale" -gt 0 ]; then
      echo $((stale - 1)) > "$state/stale_polls"
      cat "$state/boot_id"
      exit 0
    fi
    if [ -e "$state/next_boot_id" ]; then
      mv "$state/next_boot_id" "$state/boot_id"
      exit 255
    fi
    cat "$state/boot_id"
    ;;
  *reboot*)
    {on_reboot}
    ;;
esac
"#
                ),
            )
            .unwrap();
            Self { state }
        }

        fn backend(&self, max_wait_secs: u32) -> SystemRebootBackend {
            let mut backend = SystemRebootBackend::new(RebootDetectionConfig {
                reboot_max_wait: max_wait_secs,
                reboot_retry_interval: 1,
                probe_timeout: 2,
                dry_activate_timeout: 1,
                ..Default::default()
            });
            backend.ssh_command = vec![
                "bash".to_string(),
                self.state.path().join("ssh.sh").display().to_string(),
            ];
            backend
        }

        fn commands(&self) -> Vec<String> {
            std::fs::read_to_string(self.state.path().join("commands"))
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        }

        fn reboot_requested(&self) -> bool {
            self.commands().iter().any(|c| c == REBOOT_COMMAND)
        }
    }

    fn fake_target() -> DeployTarget {
        DeployTarget {
            user: Some("pusher".to_string()),
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        }
    }

    fn reboot_not_confirmed_reason(err: SnowError) -> String {
        match err {
            SnowError::Deploy(DeployError::RebootNotConfirmed { reason, .. }) => reason,
            other => panic!("expected RebootNotConfirmed, got {:?}", other),
        }
    }

    /// Runs REBOOT_COMMAND with `sudo` stubbed to print what it would run
    fn sudo_invocation_of_reboot_command(helper_installed: bool) -> String {
        use std::os::unix::fs::PermissionsExt;

        // A stub executable rather than a shell function: POSIX sh rejects
        // function names with hyphens
        let bin = tempfile::tempdir().unwrap();
        if helper_installed {
            let helper = bin.path().join("snowglobe-activate");
            std::fs::write(&helper, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let script = format!("sudo() {{ echo \"$@\"; }}\n{}", REBOOT_COMMAND);
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &script])
            .env("PATH", bin.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn test_reboot_command_uses_activation_helper_when_installed() {
        assert_eq!(
            sudo_invocation_of_reboot_command(true),
            "snowglobe-activate reboot"
        );
    }

    #[test]
    fn test_reboot_command_falls_back_to_systemctl_without_helper() {
        assert_eq!(sudo_invocation_of_reboot_command(false), "systemctl reboot");
    }

    #[tokio::test]
    async fn test_reboot_confirmed_when_connection_drops_and_boot_id_changes() {
        let host = FakeHost::new(r#"echo boot-2 > "$state/next_boot_id"; exit 255"#);

        host.backend(5)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap();

        assert!(host.reboot_requested());
    }

    #[tokio::test]
    async fn test_reboot_confirmed_when_host_is_slow_to_go_down() {
        let host = FakeHost::new(
            r#"echo 2 > "$state/stale_polls"; echo boot-2 > "$state/next_boot_id"; exit 0"#,
        );

        host.backend(6)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_reboot_fails_fast_when_sudo_refuses() {
        let host = FakeHost::new(
            r#"echo "sudo: a password is required" >&2; echo boot-2 > "$state/next_boot_id"; exit 1"#,
        );

        let err = host
            .backend(5)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        match err {
            SnowError::Deploy(DeployError::CommandFailed {
                exit_code, stderr, ..
            }) => {
                assert_eq!(exit_code, 1);
                assert!(stderr.contains("a password is required"));
            }
            other => panic!("expected CommandFailed, got {:?}", other),
        }
        assert_eq!(
            host.commands().len(),
            2,
            "should not poll for a new boot after the reboot command failed"
        );
    }

    #[tokio::test]
    async fn test_reboot_fails_when_host_keeps_same_boot_id() {
        let host = FakeHost::new("exit 0");

        let err = host
            .backend(2)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        assert!(reboot_not_confirmed_reason(err).contains("boot ID is unchanged"));
    }

    #[tokio::test]
    async fn test_reboot_fails_when_host_never_comes_back() {
        let host = FakeHost::new(r#"rm "$state/boot_id"; exit 255"#);

        let err = host
            .backend(2)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        assert!(reboot_not_confirmed_reason(err).contains("did not come back online"));
    }

    #[tokio::test]
    async fn test_hung_probe_fails_instead_of_blocking_the_deploy() {
        let host = FakeHost::new("exit 0");
        std::fs::write(host.state.path().join("hang_on"), "boot_id").unwrap();

        let err = host
            .backend(5)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        let reason = crate::error::error_chain(&err);
        assert!(reason.contains("did not finish within 2s"), "{reason}");
        assert!(!host.reboot_requested());
    }

    #[tokio::test]
    async fn test_dry_activate_has_its_own_time_limit() {
        let host = FakeHost::new("exit 0");
        std::fs::write(host.state.path().join("hang_on"), "dry-activate").unwrap();
        let new_system =
            StorePath::new("/nix/store/6c3x8d0yq2l1-nixos-system-condor-26.05".to_string());

        let err = host
            .backend(5)
            .check_reboot_required(&fake_target(), &new_system)
            .await
            .unwrap_err();

        let reason = crate::error::error_chain(&err);
        assert!(reason.contains("did not finish within 1s"), "{reason}");
    }

    #[tokio::test]
    async fn test_reboot_wait_ends_at_its_deadline_when_probes_hang() {
        let host = FakeHost::new(r#"echo boot_id > "$state/hang_on"; exit 255"#);

        let started = std::time::Instant::now();
        let err = host
            .backend(4)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        assert!(
            started.elapsed() < Duration::from_secs(7),
            "{:?}",
            started.elapsed()
        );
        let reason = reboot_not_confirmed_reason(err);
        assert!(
            reason.starts_with("host did not come back online within 4s (last error: "),
            "{reason}"
        );
        assert!(reason.contains("did not finish within 2s"), "{reason}");
    }

    /// Boots `host` from `booted` kernel and initrd and returns a system that
    /// has `new` ones; the four files are made up under the host's state
    fn systems(host: &FakeHost, booted: [&str; 2], new: [&str; 2]) -> StorePath {
        let state = host.state.path();
        for (dir, [kernel, initrd]) in [("booted", booted), ("new", new)] {
            std::fs::create_dir_all(state.join(dir)).unwrap();
            for (link, file) in [("kernel", kernel), ("initrd", initrd)] {
                std::fs::write(state.join(file), "").unwrap();
                std::os::unix::fs::symlink(state.join(file), state.join(dir).join(link)).unwrap();
            }
        }
        StorePath::new(state.join("new").display().to_string())
    }

    #[tokio::test]
    async fn test_reboot_check_lists_both_a_new_kernel_and_a_new_initrd() {
        let host = FakeHost::new("exit 0");
        let new_system = systems(&host, ["k1", "i1"], ["k2", "i2"]);

        let check = host
            .backend(5)
            .check_reboot_required(&fake_target(), &new_system)
            .await
            .unwrap();

        let file = |name: &str| host.state.path().join(name).display().to_string();
        assert_eq!(
            check.reasons,
            vec![
                RebootReason::KernelChanged {
                    old: file("k1"),
                    new: file("k2")
                },
                RebootReason::InitrdChanged {
                    old: file("i1"),
                    new: file("i2")
                },
            ]
        );
        assert!(
            !host.commands().iter().any(|c| c.contains("dry-activate")),
            "{:?}",
            host.commands()
        );
    }

    #[tokio::test]
    async fn test_reboot_check_finds_critical_services_dry_activate_would_restart() {
        let host = FakeHost::new("exit 0");
        let new_system = systems(&host, ["k1", "i1"], ["k1", "i1"]);
        std::fs::write(
            host.state.path().join("dry_activate_output"),
            "would stop the following units: systemd-sysctl.service\n\
             would activate the configuration...\n\
             would restart the following units: nix-daemon.service\n\
             would start the following units: systemd-sysctl.service\n",
        )
        .unwrap();

        let check = host
            .backend(5)
            .check_reboot_required(&fake_target(), &new_system)
            .await
            .unwrap();

        assert_eq!(
            check.reasons,
            vec![RebootReason::CriticalService(
                "systemd-sysctl.service".to_string()
            )]
        );
    }

    /// Runs the reboot check on a host whose check for snowglobe-activate
    /// exits with `helper_status`
    async fn dry_activation(helper_status: i32) -> (FakeHost, Result<RebootCheck>, StorePath) {
        let host = FakeHost::new("exit 0");
        let new_system = systems(&host, ["k1", "i1"], ["k1", "i1"]);
        std::fs::write(
            host.state.path().join("helper_status"),
            helper_status.to_string(),
        )
        .unwrap();
        let check = host
            .backend(5)
            .check_reboot_required(&fake_target(), &new_system)
            .await;
        (host, check, new_system)
    }

    fn dry_activate_commands(host: &FakeHost) -> Vec<String> {
        host.commands()
            .into_iter()
            .filter(|c| c.ends_with(" dry-activate"))
            .collect()
    }

    #[tokio::test]
    async fn test_dry_activate_runs_through_the_helper_when_installed() {
        let (host, check, new_system) = dry_activation(0).await;

        check.unwrap();
        assert_eq!(
            dry_activate_commands(&host),
            vec![format!(
                "sudo snowglobe-activate {} dry-activate",
                new_system.as_str()
            )]
        );
    }

    #[tokio::test]
    async fn test_dry_activate_falls_back_only_when_the_helper_is_missing() {
        let (host, check, new_system) = dry_activation(1).await;

        check.unwrap();
        assert_eq!(
            dry_activate_commands(&host),
            vec![format!(
                "sudo {}/bin/switch-to-configuration dry-activate",
                new_system.as_str()
            )]
        );
    }

    #[tokio::test]
    async fn test_reboot_check_stops_when_the_helper_check_fails() {
        let (host, check, _) = dry_activation(255).await;

        assert!(matches!(
            check.unwrap_err(),
            SnowError::Deploy(DeployError::SshFailed { .. })
        ));
        assert_eq!(dry_activate_commands(&host), Vec::<String>::new());
    }

    #[tokio::test]
    async fn test_reboot_check_fails_when_sudo_refuses_dry_activate() {
        let host = FakeHost::new("exit 0");
        let new_system = systems(&host, ["k1", "i1"], ["k1", "i1"]);
        std::fs::write(host.state.path().join("refuse"), "dry-activate").unwrap();

        let err = host
            .backend(5)
            .check_reboot_required(&fake_target(), &new_system)
            .await
            .unwrap_err();

        let reason = crate::error::error_chain(&err);
        assert!(
            reason.contains("dry-activate` failed with exit status: 1."),
            "{reason}"
        );
        assert!(reason.contains("snowglobe.deploy.allowedUsers"), "{reason}");
        assert!(
            reason.trim_end().ends_with("sudo: a password is required"),
            "{reason}"
        );
    }

    #[tokio::test]
    async fn test_reboot_wait_says_when_the_host_never_answered() {
        let host = FakeHost::new(r#"echo boot_id > "$state/hang_on"; exit 255"#);

        let err = host
            .backend(1)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        assert_eq!(
            reboot_not_confirmed_reason(err),
            "host did not come back online within 1s (last error: the host never answered)"
        );
    }

    #[tokio::test]
    async fn test_reboot_max_wait_can_be_changed_after_construction() {
        let host = FakeHost::new(r#"rm "$state/boot_id"; exit 255"#);

        let err = host
            .backend(600)
            .with_reboot_max_wait(2)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        let reason = reboot_not_confirmed_reason(err);
        assert!(reason.contains("within 2s"), "{reason}");
    }

    #[tokio::test]
    async fn test_reboot_wait_reports_the_last_probe_error() {
        let host = FakeHost::new(
            r#"echo "ssh: connect to host hedgehog.example.com port 22: No route to host" > "$state/unreachable"; exit 255"#,
        );

        let err = host
            .backend(3)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        let reason = reboot_not_confirmed_reason(err);
        assert!(
            reason.starts_with("host did not come back online within 3s (last error: "),
            "{reason}"
        );
        assert!(reason.ends_with("No route to host)"), "{reason}");
    }

    #[tokio::test]
    async fn test_reboot_not_requested_when_boot_id_unreadable() {
        let host = FakeHost::new("exit 0");
        std::fs::remove_file(host.state.path().join("boot_id")).unwrap();

        let err = host
            .backend(2)
            .reboot_and_wait(&fake_target())
            .await
            .unwrap_err();

        assert!(matches!(err, SnowError::Deploy(DeployError::Failed { .. })));
        assert!(!host.reboot_requested());
    }

    #[test]
    fn test_ssh_args_use_the_configured_connect_timeout_and_keepalives() {
        let backend = SystemRebootBackend::with_defaults().with_ssh_user("pusher");

        let args = backend
            .build_ssh_args(&fake_target(), "cat /proc/sys/kernel/random/boot_id")
            .join(" ");

        assert_eq!(
            args,
            "-o BatchMode=yes -o ConnectTimeout=5 -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -o ForwardAgent=no \
             pusher@hedgehog.example.com cat /proc/sys/kernel/random/boot_id"
        );
    }

    #[test]
    fn test_ssh_args_connect_to_the_hosts_port() {
        let target = DeployTarget {
            port: Some(2222),
            ..fake_target()
        };

        let args = SystemRebootBackend::with_defaults()
            .build_ssh_args(&target, "true")
            .join(" ");

        assert!(
            args.ends_with("-p 2222 pusher@hedgehog.example.com true"),
            "{args}"
        );
    }

    #[test]
    fn test_reboot_backend_with_ssh_user() {
        let backend = SystemRebootBackend::with_defaults().with_ssh_user("deploy");
        assert_eq!(backend.ssh_user, Some("deploy".to_string()));
    }

    #[test]
    fn test_get_destination_with_backend_user() {
        let backend = SystemRebootBackend::with_defaults().with_ssh_user("deploy");
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        };

        assert_eq!(
            backend.get_destination(&target),
            "deploy@hedgehog.example.com"
        );
    }

    #[test]
    fn test_get_destination_without_backend_user() {
        let backend = SystemRebootBackend::with_defaults();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        };

        assert_eq!(
            backend.get_destination(&target),
            "root@hedgehog.example.com"
        );
    }

    #[test]
    fn test_empty_critical_services_list() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: true,
            check_initrd: true,
            ..Default::default()
        };

        let backend = SystemRebootBackend::new(config.clone());

        assert!(backend.config.critical_services.is_empty());
        assert!(backend.config.check_kernel);
        assert!(backend.config.check_initrd);
    }

    #[test]
    fn test_config_with_no_checks_enabled() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            ..Default::default()
        };

        let backend = SystemRebootBackend::new(config);

        assert!(backend.config.critical_services.is_empty());
        assert!(!backend.config.check_kernel);
        assert!(!backend.config.check_initrd);
    }
}
