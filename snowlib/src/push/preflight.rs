//! Checks that run before any host is touched

use std::path::Path;

use super::config::PushStrategy;
use super::context::PushContext;
use crate::backends::deploy::check_mode_supported;
use crate::error::DeployError;
use crate::hooks::{find_store_paths, HookLocation};
use crate::host::HostMetadata;

/// Fails before any host is touched if a local hook needs a path this machine lacks
pub(super) fn check_local_hooks(
    push_ctx: &PushContext<'_>,
    hosts: &[HostMetadata],
) -> Result<(), DeployError> {
    // A dry run runs no hooks
    if push_ctx.config.strategy == PushStrategy::DryRun {
        return Ok(());
    }
    hosts
        .iter()
        .try_for_each(|host| check_local_hook_paths(host, |path| Path::new(path).exists()))
}

/// Fails before anything is built if a nix-darwin host can't activate as asked
pub(super) fn check_hosts_support_strategy(
    push_ctx: &PushContext<'_>,
    hosts: &[HostMetadata],
) -> Result<(), DeployError> {
    let Some(mode) = push_ctx.config.strategy.fixed_activation_mode() else {
        return Ok(());
    };
    check_mode_supported(
        mode,
        hosts
            .iter()
            .map(|host| (host.name.as_str(), host.system_type)),
    )
}

/// Fails before any host is touched if ssh would hang on the ssh-agent
pub(super) async fn check_ssh_agent(push_ctx: &PushContext<'_>) -> crate::error::Result<()> {
    push_ctx.deploy_backend.preflight().await
}

/// Fails on the first store path a local hook references that `exists` says is missing
fn check_local_hook_paths(
    host_metadata: &HostMetadata,
    exists: impl Fn(&str) -> bool,
) -> Result<(), DeployError> {
    let missing = host_metadata
        .hooks
        .iter()
        .filter(|hook| hook.location == HookLocation::Local)
        .flat_map(|hook| {
            find_store_paths(&hook.command)
                .into_iter()
                .map(move |path| (hook, path))
        })
        .find(|(_, path)| !exists(path));
    match missing {
        Some((hook, path)) => Err(DeployError::HookPathMissing {
            host: host_metadata.name.clone(),
            hook: hook.name.clone(),
            path: path.to_string(),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookConfig, HookStage};
    use crate::nix::SystemType;
    use crate::reboot::RebootPolicy;

    const KUBECTL: &str = "/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6y-kubectl-1.34.2";
    const GATE: &str = "/nix/store/ywilmysgc3p103v1y4hhzbhb3v0r0mkh-kubernetes-storage-healthy";

    fn host_with_hooks(hooks: Vec<HookConfig>) -> HostMetadata {
        HostMetadata {
            name: "praseodymium".to_string(),
            phase: Some("p2".to_string()),
            tags: vec![],
            fqdn: "praseodymium.example.com".to_string(),
            reboot_policy: RebootPolicy::Always,
            system_type: SystemType::NixOS,
            system: None,
            hooks,
            serial_group: None,
            ssh_port: None,
            optional: false,
        }
    }

    fn hook(name: &str, location: HookLocation, command: String) -> HookConfig {
        HookConfig::test_fixture(name, HookStage::PreDeploy, location, &command)
    }

    #[test]
    fn test_check_local_hook_paths_names_hook_and_path() {
        let host = host_with_hooks(vec![
            hook(
                "drain-node",
                HookLocation::Local,
                format!("{}/bin/kubectl drain x", KUBECTL),
            ),
            hook(
                "storage-gate",
                HookLocation::Local,
                format!("{}/bin/gate", GATE),
            ),
        ]);

        let result = check_local_hook_paths(&host, |path| path == KUBECTL);

        assert!(matches!(
            result,
            Err(DeployError::HookPathMissing { ref hook, ref path, .. })
                if hook == "storage-gate" && path == GATE
        ));
    }

    #[test]
    fn test_check_local_hook_paths_ignores_remote_hooks_and_plain_commands() {
        let host = host_with_hooks(vec![
            hook("verify", HookLocation::Remote, format!("{}/bin/gate", GATE)),
            hook(
                "uncordon-node",
                HookLocation::Local,
                "kubectl uncordon x".to_string(),
            ),
        ]);

        assert!(check_local_hook_paths(&host, |_| false).is_ok());
    }
}
