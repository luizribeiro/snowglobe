use super::{parse_hostnames, NixBackend};
use async_trait::async_trait;

use crate::bootstrap::{BootstrapConfig, BootstrapConfigJson};
use crate::error::Result;
use crate::hooks::{HookConfig, HookLocation, HookStage};
use crate::host::{Host, HostMetadata};
use crate::nix::{Flake, NixCommand, StorePath, SystemType};
use crate::process::OutputMode;
use crate::reboot::RebootPolicy;
use std::path::PathBuf;
use std::str::FromStr;

/// System implementation of NixBackend that executes real Nix commands
///
/// This is the production implementation that invokes `nix eval` and `nix build`
/// to interact with the actual Nix system.
#[derive(Debug, Clone, Default)]
pub struct SystemNixBackend {
    gc_roots: Option<PathBuf>,
}

impl SystemNixBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Roots the hooks files and systems this backend builds in `dir`, so the
    /// machine's GC can't collect them while the links exist
    pub fn with_gc_roots(mut self, dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        // nix resolves a relative --out-link against the flake, not our cwd
        assert!(dir.is_absolute(), "GC roots dir must be absolute: {dir:?}");
        self.gc_roots = Some(dir);
        self
    }

    fn link_output(&self, command: NixCommand, host: &str, kind: GcRootKind) -> NixCommand {
        match &self.gc_roots {
            Some(dir) => command.out_link(dir.join(gc_root_name(host, kind))),
            None => command.no_link(),
        }
    }

    /// Builds the host's hooks file on this machine, which also fetches or builds
    /// what local hooks reference, and returns its path
    async fn build_hooks_file(&self, flake: &Flake, host: &str, drv: &str) -> Result<String> {
        let command = NixCommand::build(flake, format!("{}^out", drv));
        let stdout = self
            .link_output(command, host, GcRootKind::Hooks)
            .print_out_paths()
            .output_mode(OutputMode::Capture)
            .run()
            .await?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_string())
    }
}

#[derive(Debug, Clone, Copy)]
enum GcRootKind {
    Hooks,
    System,
}

fn gc_root_name(host: &str, kind: GcRootKind) -> String {
    let suffix = match kind {
        GcRootKind::Hooks => "hooks",
        GcRootKind::System => "system",
    };
    format!("{host}-{suffix}")
}

#[async_trait]
impl NixBackend for SystemNixBackend {
    async fn list_hosts(
        &self,
        flake: &Flake,
        system_type: SystemType,
        filter_expr: Option<&str>,
    ) -> Result<Vec<Host>> {
        let apply_arg = if let Some(expr) = filter_expr {
            format!(
                r#"configs:
  let
    inherit (builtins) filter attrNames;
    predicate = (
      {}
    );
    matchingHosts = filter (hostname: predicate configs.${{hostname}}.config) (attrNames configs);
  in matchingHosts"#,
                expr
            )
        } else {
            "builtins.attrNames".to_string()
        };

        let flake_ref = format!(".#{}", system_type.flake_attr());
        let stdout = NixCommand::eval(flake, &flake_ref)
            .apply(&apply_arg)
            .json()
            .run()
            .await?;

        parse_hostnames(&stdout)
    }

    async fn get_host_metadata(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
    ) -> Result<HostMetadata> {
        let as_metadata_error = |e| match e {
            crate::error::SnowError::Nix(nix_err) => {
                crate::error::SnowError::Nix(crate::error::NixError::MetadataFailed {
                    host: host.to_string(),
                    source: Box::new(nix_err),
                })
            }
            other => other,
        };

        let flake_ref = format!(".#{}.{}", system_type.flake_attr(), host);
        let stdout = NixCommand::eval(flake, &flake_ref)
            .apply(metadata_apply_expr(&local_nix_system()))
            .json()
            .run()
            .await
            .map_err(as_metadata_error)?;

        let (mut metadata, hooks_file) = parse_host_metadata(host, system_type, &stdout)?;
        if let Some(drv) = hooks_file {
            let path = self
                .build_hooks_file(flake, host, &drv)
                .await
                .map_err(as_metadata_error)?;
            metadata.hooks = read_hooks_file(&path).map_err(as_metadata_error)?;
        }
        Ok(metadata)
    }

    async fn build_host(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
        output_mode: OutputMode,
    ) -> Result<StorePath> {
        let target = format!(
            ".#{}.{}.{}",
            system_type.flake_attr(),
            host,
            system_type.build_path_suffix()
        );

        let stdout = self
            .link_output(NixCommand::build(flake, &target), host, GcRootKind::System)
            .print_out_paths()
            .output_mode(output_mode)
            .run()
            .await
            .map_err(|e| match e {
                crate::error::SnowError::Nix(crate::error::NixError::EvalFailed { stderr }) => {
                    crate::error::SnowError::Nix(crate::error::NixError::BuildFailed {
                        host: host.to_string(),
                        stderr,
                    })
                }
                other => other,
            })?;

        let path = String::from_utf8(stdout)
            .map_err(|e| crate::error::NixError::EvalFailed {
                stderr: format!("Non-UTF8 build path: {:?}", e.as_bytes()),
            })?
            .trim()
            .to_string();

        Ok(StorePath::new(path))
    }

    async fn get_bootstrap_config(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
    ) -> Result<BootstrapConfig> {
        let flake_ref = format!(".#{}.{}", system_type.flake_attr(), host);

        let apply_expr = r#"config: {
          provisioner = config.config.snowglobe.bootstrap.provisioner or null;
          imageAttr = config.config.snowglobe.bootstrap.imageAttr or null;
          sshKeyPath = config.config.snowglobe.bootstrap.sshKeyPath or null;
          rootFsDevice = config.config.fileSystems."/".device or null;
        }"#;

        let stdout = NixCommand::eval(flake, &flake_ref)
            .apply(apply_expr)
            .json()
            .run()
            .await
            .map_err(|e| match e {
                crate::error::SnowError::Nix(nix_err) => {
                    crate::error::SnowError::Nix(crate::error::NixError::MetadataFailed {
                        host: host.to_string(),
                        source: Box::new(nix_err),
                    })
                }
                other => other,
            })?;

        let json_config: BootstrapConfigJson = serde_json::from_slice(&stdout).map_err(|e| {
            crate::error::NixError::OutputParseFailed {
                output: String::from_utf8_lossy(&stdout).to_string(),
                source: e,
            }
        })?;

        Ok(json_config.into())
    }

    async fn build_image(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
        image_attr: &str,
        output_mode: OutputMode,
    ) -> Result<StorePath> {
        // Build the image attribute: .#nixosConfigurations.hostname.config.system.build.sdImage
        let target = format!(".#{}.{}.{}", system_type.flake_attr(), host, image_attr);

        // Images are copied out right after the build, so they need no GC root
        let stdout = NixCommand::build(flake, &target)
            .no_link()
            .print_out_paths()
            .output_mode(output_mode)
            .run()
            .await
            .map_err(|e| match e {
                crate::error::SnowError::Nix(crate::error::NixError::EvalFailed { stderr }) => {
                    crate::error::ProvisionerError::ImageBuildFailed {
                        host: host.to_string(),
                        stderr,
                    }
                    .into()
                }
                other => other,
            })?;

        let path = String::from_utf8(stdout)
            .map_err(|e| crate::error::NixError::EvalFailed {
                stderr: format!("Non-UTF8 build path: {:?}", e.as_bytes()),
            })?
            .trim()
            .to_string();

        Ok(StorePath::new(path))
    }
}

/// The Nix platform string of the machine running snow, e.g. "aarch64-darwin"
fn local_nix_system() -> String {
    nix_system(std::env::consts::ARCH, std::env::consts::OS)
}

/// Converts Rust's target arch/OS names to a Nix platform string
fn nix_system(arch: &str, os: &str) -> String {
    let arch = match arch {
        "x86" => "i686",
        "arm" => "armv7l",
        arch => arch,
    };
    let os = match os {
        "macos" => "darwin",
        os => os,
    };
    format!("{}-{}", arch, os)
}

/// Builds the `--apply` expression that extracts host metadata as JSON
///
/// When this machine's platform differs from the host's, the host is
/// re-evaluated with `snowglobe.localSystem` set to it, so local hooks get
/// packages that run here. The re-evaluation costs about half a second per
/// host, so it's skipped when the platforms already match.
fn metadata_apply_expr(local_system: &str) -> String {
    format!(
        r#"host:
        let
          needsLocalSystem = host ? options && host.options ? snowglobe && host.options.snowglobe ? localSystem
            && host.config.snowglobe.localSystem != "{local_system}";
          extended = if needsLocalSystem
            then host.extendModules {{
              modules = [ ({{ lib, ... }}: {{ snowglobe.localSystem = lib.mkForce "{local_system}"; }}) ];
            }}
            else host;
          config = extended.config;
        in {{
          hostName = config.networking.hostName;
          domain = config.networking.domain or "";
          phase = config.snowglobe.push.phase or null;
          tags = config.snowglobe.push.tags or [];
          rebootPolicy = config.snowglobe.push.rebootPolicy or "auto";
          serialGroup = config.snowglobe.push.serialGroup or null;
          sshPort = config.snowglobe.push.sshPort or null;
          hooksFile = if config ? snowglobe && config.snowglobe ? hooksFile
            then config.snowglobe.hooksFile.drvPath
            else null;
        }}"#
    )
}

fn read_hooks_file(path: &str) -> Result<Vec<HookConfig>> {
    let json =
        std::fs::read(path).map_err(|source| crate::error::NixError::HooksFileReadFailed {
            path: path.to_string(),
            source,
        })?;
    parse_hooks(&json)
}

/// Kept apart from `get_host_metadata` so parsing is testable without invoking nix
///
/// Returns the metadata (without hooks) and the hooks file's derivation, if any.
fn parse_host_metadata(
    host: &str,
    system_type: SystemType,
    stdout: &[u8],
) -> Result<(HostMetadata, Option<String>)> {
    #[derive(serde::Deserialize)]
    struct MetadataJson {
        #[serde(rename = "hostName")]
        hostname: String,
        // Domain can be null for hosts that connect to different networks (like laptops)
        // or when not configured in the NixOS/nix-darwin configuration
        domain: Option<String>,
        phase: Option<String>,
        tags: Vec<String>,
        #[serde(rename = "rebootPolicy")]
        reboot_policy: String,
        #[serde(default, rename = "serialGroup")]
        serial_group: Option<String>,
        #[serde(default, rename = "sshPort")]
        ssh_port: Option<u16>,
        #[serde(default, rename = "hooksFile")]
        hooks_file: Option<String>,
    }

    let meta: MetadataJson =
        serde_json::from_slice(stdout).map_err(|e| crate::error::NixError::OutputParseFailed {
            output: String::from_utf8_lossy(stdout).to_string(),
            source: e,
        })?;

    crate::push::check_phase(host, meta.phase.as_deref())?;

    let fqdn = match meta.domain {
        Some(domain) if !domain.is_empty() => format!("{}.{}", meta.hostname, domain),
        _ => meta.hostname.clone(),
    };

    let metadata = HostMetadata {
        name: host.to_string(),
        phase: meta.phase,
        tags: meta.tags,
        fqdn,
        reboot_policy: RebootPolicy::from_str(&meta.reboot_policy)?,
        system_type,
        serial_group: meta.serial_group,
        ssh_port: meta.ssh_port,
        hooks: Vec::new(),
    };
    Ok((metadata, meta.hooks_file))
}

/// Parses a hooks file (stage → hook name → hook) into a flat list
fn parse_hooks(json: &[u8]) -> Result<Vec<HookConfig>> {
    #[derive(serde::Deserialize)]
    struct HookConfigWithoutStage {
        location: HookLocation,
        command: String,
        #[serde(default)]
        environment: std::collections::HashMap<String, String>,
        #[serde(default, rename = "environmentFiles")]
        environment_files: Vec<std::path::PathBuf>,
        #[serde(default = "default_hook_timeout")]
        timeout: u64,
        #[serde(default, rename = "continueOnFailure")]
        continue_on_failure: bool,
        #[serde(default = "default_hook_priority")]
        priority: i32,
        #[serde(default)]
        retries: u32,
        #[serde(default = "default_retry_delay", rename = "retryDelay")]
        retry_delay: u64,
    }

    fn default_hook_timeout() -> u64 {
        300
    }

    fn default_hook_priority() -> i32 {
        100
    }

    fn default_retry_delay() -> u64 {
        5
    }

    type HooksByStage = std::collections::HashMap<
        String,
        std::collections::HashMap<String, HookConfigWithoutStage>,
    >;

    let by_stage: HooksByStage =
        serde_json::from_slice(json).map_err(|e| crate::error::NixError::OutputParseFailed {
            output: String::from_utf8_lossy(json).to_string(),
            source: e,
        })?;

    let mut hooks = Vec::new();
    for (stage_str, stage_hooks) in by_stage {
        let stage = HookStage::from_str(&stage_str)?;
        for (hook_name, hook) in stage_hooks {
            hooks.push(HookConfig {
                name: hook_name,
                stage,
                location: hook.location,
                command: hook.command,
                environment: hook.environment,
                environment_files: hook.environment_files,
                timeout: hook.timeout,
                continue_on_failure: hook.continue_on_failure,
                priority: hook.priority,
                retries: hook.retries,
                retry_delay: hook.retry_delay,
            });
        }
    }
    Ok(hooks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn try_parse(json: &str) -> Result<HostMetadata> {
        parse_host_metadata("web01", SystemType::NixOS, json.as_bytes()).map(|(meta, _)| meta)
    }

    fn parse(json: &str) -> HostMetadata {
        try_parse(json).unwrap()
    }

    #[test]
    fn test_parse_metadata_with_null_domain() {
        let meta = parse(
            r#"{
            "hostName": "macbook",
            "domain": null,
            "phase": null,
            "tags": [],
            "rebootPolicy": "auto",
            "serialGroup": null
        }"#,
        );

        assert_eq!(meta.name, "web01");
        assert_eq!(meta.fqdn, "macbook");
        assert_eq!(meta.phase, None);
        assert!(meta.tags.is_empty());
        assert_eq!(meta.reboot_policy, RebootPolicy::Auto);
        assert!(meta.hooks.is_empty());
        assert_eq!(meta.serial_group, None);
    }

    #[test]
    fn test_parse_metadata_with_empty_domain() {
        let meta = parse(
            r#"{
            "hostName": "server",
            "domain": "",
            "phase": "p1",
            "tags": ["web"],
            "rebootPolicy": "never"
        }"#,
        );

        assert_eq!(meta.fqdn, "server");
        assert_eq!(meta.phase, Some("p1".to_string()));
        assert_eq!(meta.tags, vec!["web"]);
        assert_eq!(meta.reboot_policy, RebootPolicy::Never);
    }

    #[test]
    fn test_parse_metadata_with_valid_domain() {
        let meta = parse(
            r#"{
            "hostName": "web01",
            "domain": "example.com",
            "phase": "p2",
            "tags": ["mysql", "ldap"],
            "rebootPolicy": "always"
        }"#,
        );

        assert_eq!(meta.fqdn, "web01.example.com");
        assert_eq!(meta.phase, Some("p2".to_string()));
        assert_eq!(meta.tags, vec!["mysql", "ldap"]);
        assert_eq!(meta.reboot_policy, RebootPolicy::Always);
    }

    #[test]
    fn test_parse_metadata_with_serial_group() {
        let meta = parse(
            r#"{
            "hostName": "praseodymium",
            "domain": "example.com",
            "phase": "p2",
            "tags": ["kubernetes"],
            "rebootPolicy": "always",
            "serialGroup": "kubernetes"
        }"#,
        );

        assert_eq!(meta.serial_group.as_deref(), Some("kubernetes"));
    }

    #[test]
    fn test_parse_metadata_with_ssh_port() {
        let meta = parse(
            r#"{
            "hostName": "praseodymium",
            "domain": "example.com",
            "phase": "p2",
            "tags": [],
            "rebootPolicy": "auto",
            "sshPort": 2222
        }"#,
        );

        assert_eq!(meta.ssh_port, Some(2222));
    }

    #[test]
    fn test_parse_metadata_without_ssh_port_leaves_it_to_ssh_config() {
        let meta = parse(
            r#"{
            "hostName": "praseodymium",
            "domain": "example.com",
            "phase": "p2",
            "tags": [],
            "rebootPolicy": "auto",
            "sshPort": null
        }"#,
        );

        assert_eq!(meta.ssh_port, None);
    }

    #[test]
    fn test_parse_metadata_rejects_an_out_of_range_ssh_port() {
        let result = try_parse(
            r#"{
            "hostName": "praseodymium",
            "domain": "example.com",
            "phase": "p2",
            "tags": [],
            "rebootPolicy": "auto",
            "sshPort": 70000
        }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_metadata_rejects_an_invalid_phase() {
        let result = try_parse(
            r#"{
            "hostName": "web01",
            "domain": "example.com",
            "phase": "p2 k8s",
            "tags": [],
            "rebootPolicy": "auto"
        }"#,
        );

        assert_eq!(
            result.unwrap_err().to_string(),
            "web01 has an invalid push phase 'p2 k8s'"
        );
    }

    #[test]
    fn test_metadata_apply_expr_reads_phase_tags_and_reboot_policy_from_the_module() {
        let expr = metadata_apply_expr("x86_64-linux");

        assert!(expr.contains("phase = config.snowglobe.push.phase or null;"));
        assert!(expr.contains("tags = config.snowglobe.push.tags or [];"));
        assert!(expr.contains(r#"rebootPolicy = config.snowglobe.push.rebootPolicy or "auto";"#));
    }

    #[test]
    fn test_metadata_apply_expr_reads_ssh_port() {
        assert!(metadata_apply_expr("x86_64-linux")
            .contains("sshPort = config.snowglobe.push.sshPort or null;"));
    }

    #[test]
    fn test_metadata_apply_expr_reads_serial_group() {
        assert!(metadata_apply_expr("x86_64-linux")
            .contains("serialGroup = config.snowglobe.push.serialGroup or null;"));
    }

    #[test]
    fn test_metadata_apply_expr_sets_local_system_and_reads_hooks_file() {
        let expr = metadata_apply_expr("aarch64-darwin");

        assert!(expr.contains(r#"snowglobe.localSystem = lib.mkForce "aarch64-darwin";"#));
        assert!(expr.contains("config.snowglobe.hooksFile.drvPath"));
    }

    #[test]
    fn test_nix_system_uses_nix_platform_names() {
        assert_eq!(nix_system("x86_64", "linux"), "x86_64-linux");
        assert_eq!(nix_system("aarch64", "linux"), "aarch64-linux");
        assert_eq!(nix_system("aarch64", "macos"), "aarch64-darwin");
        assert_eq!(nix_system("x86_64", "macos"), "x86_64-darwin");
        assert_eq!(nix_system("x86", "linux"), "i686-linux");
        assert_eq!(nix_system("arm", "linux"), "armv7l-linux");
    }

    #[test]
    fn test_read_hooks_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snowglobe-hooks.json");
        std::fs::write(
            &path,
            r#"{"post-deploy": {"uncordon-node": {"location": "local", "command": "kubectl uncordon samarium"}}}"#,
        )
        .unwrap();

        let hooks = read_hooks_file(path.to_str().unwrap()).unwrap();

        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].stage, HookStage::PostDeploy);
        assert_eq!(hooks[0].command, "kubectl uncordon samarium");
    }

    #[test]
    fn test_read_hooks_file_missing() {
        let err = read_hooks_file("/nonexistent/snowglobe-hooks.json").unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Nix(crate::error::NixError::HooksFileReadFailed { .. })
        ));
    }

    #[test]
    fn test_parse_metadata_returns_hooks_file_derivation() {
        let (meta, hooks_file) = parse_host_metadata(
            "praseodymium",
            SystemType::NixOS,
            br#"{"hostName": "praseodymium", "domain": "example.com", "phase": "p2", "tags": [],
                "rebootPolicy": "always", "hooksFile": "/nix/store/abc-snowglobe-hooks.json.drv"}"#,
        )
        .unwrap();

        assert_eq!(
            hooks_file.as_deref(),
            Some("/nix/store/abc-snowglobe-hooks.json.drv")
        );
        assert!(meta.hooks.is_empty());
    }

    #[test]
    fn test_parse_metadata_without_hooks_file() {
        let (_, hooks_file) = parse_host_metadata(
            "web01",
            SystemType::NixOS,
            br#"{"hostName": "web01", "domain": null, "phase": null, "tags": [], "rebootPolicy": "auto", "hooksFile": null}"#,
        )
        .unwrap();

        assert_eq!(hooks_file, None);
    }

    #[test]
    fn test_parse_hooks() {
        let hooks = parse_hooks(
            br#"{
            "pre-deploy": {
                "drain-node": { "location": "local", "command": "kubectl drain praseodymium", "timeout": 900 }
            },
            "post-deploy": {},
            "post-reboot": {},
            "on-failure": {}
        }"#,
        )
        .unwrap();

        assert_eq!(hooks.len(), 1);
        let hook = &hooks[0];
        assert_eq!(hook.name, "drain-node");
        assert_eq!(hook.stage, HookStage::PreDeploy);
        assert_eq!(hook.timeout, 900);
        assert_eq!(hook.retries, 0);
        assert_eq!(hook.priority, 100);
        assert_eq!(hook.retry_delay, 5);
        assert!(!hook.continue_on_failure);
        assert!(hook.environment.is_empty());
    }

    #[test]
    fn test_parse_metadata_rejects_invalid_reboot_policy() {
        let err = try_parse(
            r#"{"hostName": "web01", "domain": null, "phase": null, "tags": [], "rebootPolicy": "sometimes"}"#,
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("sometimes") || format!("{:?}", err).contains("sometimes")
        );
    }

    #[test]
    fn test_parse_hooks_rejects_unknown_stage() {
        assert!(parse_hooks(br#"{"pre-activate": {}}"#).is_err());
    }

    #[test]
    fn test_parse_metadata_rejects_malformed_json() {
        let err = try_parse(r#"{"hostName": "web01""#).unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Nix(crate::error::NixError::OutputParseFailed { .. })
        ));
    }

    fn link_args(backend: &SystemNixBackend, kind: GcRootKind) -> Vec<String> {
        let target = ".#nixosConfigurations.radon.config.system.build.toplevel";
        let command = NixCommand::build(&Flake::test_context(), target);
        backend.link_output(command, "radon", kind).args()[1..].to_vec()
    }

    #[test]
    fn test_system_build_is_rooted_in_the_gc_roots_dir() {
        let backend = SystemNixBackend::new().with_gc_roots("/tmp/snow-push-x1");

        assert_eq!(
            link_args(&backend, GcRootKind::System),
            [
                ".#nixosConfigurations.radon.config.system.build.toplevel",
                "--out-link",
                "/tmp/snow-push-x1/radon-system"
            ]
        );
    }

    #[test]
    fn test_hooks_and_system_get_separate_roots() {
        assert_eq!(gc_root_name("radon", GcRootKind::Hooks), "radon-hooks");
        assert_eq!(gc_root_name("radon", GcRootKind::System), "radon-system");
    }

    #[test]
    fn test_builds_are_unrooted_without_gc_roots_dir() {
        assert_eq!(
            link_args(&SystemNixBackend::new(), GcRootKind::Hooks),
            [
                ".#nixosConfigurations.radon.config.system.build.toplevel",
                "--no-link"
            ]
        );
    }

    #[test]
    #[should_panic(expected = "must be absolute")]
    fn test_relative_gc_roots_dir_is_rejected() {
        let _ = SystemNixBackend::new().with_gc_roots("roots");
    }
}
