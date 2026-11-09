//! Proxmox VMA provisioner
//!
//! Injects SSH keys into Proxmox VMA backup images and deploys them to Proxmox hosts.

use super::command::{CommandExecutor, CommandOutput, SystemCommandExecutor};
use super::{
    guestfish, InjectionResult, Provisioner, ProvisionerConfig, ProvisioningResult, TargetType,
};
use crate::backends::deploy::DeployTarget;
use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use crate::ssh_options::connection_options;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Proxmox VMA provisioner
///
/// Uses vma for extraction/repacking and guestfish for rootless key injection.
pub struct ProxmoxVmaProvisioner {
    executor: Arc<dyn CommandExecutor>,
}

impl ProxmoxVmaProvisioner {
    pub fn new() -> Self {
        Self {
            executor: Arc::new(SystemCommandExecutor::new()),
        }
    }

    #[cfg(any(debug_assertions, test, feature = "test-utils"))]
    pub fn with_executor(executor: Arc<dyn CommandExecutor>) -> Self {
        Self { executor }
    }

    fn is_compressed(path: &Path) -> bool {
        path.extension().is_some_and(|ext| ext == "zst")
    }

    fn find_vma_file(&self, path: &Path, hostname: &str) -> Result<PathBuf> {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }

        if path.is_dir() {
            let expected_zst = path.join(format!("vzdump-qemu-{}.vma.zst", hostname));
            if expected_zst.exists() {
                return Ok(expected_zst);
            }

            let expected_vma = path.join(format!("vzdump-qemu-{}.vma", hostname));
            if expected_vma.exists() {
                return Ok(expected_vma);
            }

            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "Expected vzdump-qemu-{}.vma.zst in {}, but not found",
                    hostname,
                    path.display()
                ),
            }
            .into());
        }

        Err(ProvisionerError::InjectionFailed {
            provisioner: self.name().to_string(),
            reason: format!("Path does not exist: {}", path.display()),
        }
        .into())
    }

    fn run_tool(&self, tool: &str, args: &[&str], operation: &str) -> Result<CommandOutput> {
        let output = self.executor.execute(tool, args)?;

        if !output.success {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "{} failed (exit {}): {}",
                    operation,
                    output.exit_code.unwrap_or(-1),
                    output.stderr_str()
                ),
            }
            .into());
        }

        Ok(output)
    }

    fn run_remote(&self, host: &DeployTarget, cmd: &str, operation: &str) -> Result<CommandOutput> {
        let output = self.executor.execute_remote(host, cmd)?;

        if !output.success {
            return Err(ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "{} failed (exit {}): {}",
                    operation,
                    output.exit_code.unwrap_or(-1),
                    output.stderr_str()
                ),
            }
            .into());
        }

        Ok(output)
    }

    fn decompress(&self, input: &Path, output: &Path) -> Result<()> {
        self.run_tool(
            "zstd",
            &[
                "-d",
                "-f",
                &input.to_string_lossy(),
                "-o",
                &output.to_string_lossy(),
            ],
            "Decompression",
        )?;
        Ok(())
    }

    fn compress(&self, input: &Path, output: &Path) -> Result<()> {
        self.run_tool(
            "zstd",
            &[
                "-f",
                &input.to_string_lossy(),
                "-o",
                &output.to_string_lossy(),
            ],
            "Compression",
        )?;
        Ok(())
    }

    fn extract_vma(&self, vma_path: &Path, extract_dir: &Path) -> Result<()> {
        self.run_tool(
            "vma",
            &[
                "extract",
                &vma_path.to_string_lossy(),
                &extract_dir.to_string_lossy(),
            ],
            "VMA extraction",
        )?;
        Ok(())
    }

    fn find_root_disk(&self, extract_dir: &Path) -> Result<PathBuf> {
        let patterns = ["scsi0", "virtio0", "sata0", "ide0"];

        for pattern in patterns {
            let disk_path = extract_dir.join(format!("disk-drive-{}.raw", pattern));
            if disk_path.exists() {
                return Ok(disk_path);
            }
        }

        // Fallback: find any .raw file
        let entries =
            std::fs::read_dir(extract_dir).map_err(|e| ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!("Failed to read extraction directory: {}", e),
            })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "raw") {
                return Ok(path);
            }
        }

        Err(ProvisionerError::InjectionFailed {
            provisioner: self.name().to_string(),
            reason: "No disk image found in extracted VMA".to_string(),
        }
        .into())
    }

    fn create_vma(&self, extract_dir: &Path, output_path: &Path) -> Result<()> {
        let qemu_conf_path = extract_dir.join("qemu-server.conf");

        let entries =
            std::fs::read_dir(extract_dir).map_err(|e| ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "Failed to read extraction directory {}: {}",
                    extract_dir.display(),
                    e
                ),
            })?;

        let mut args: Vec<String> = vec![
            "create".to_string(),
            output_path.to_string_lossy().to_string(),
        ];

        if qemu_conf_path.exists() {
            args.push("-c".to_string());
            args.push(qemu_conf_path.to_string_lossy().to_string());
        }

        let mut disk_count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "raw") {
                if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                    if let Some(drive_name) = filename
                        .strip_prefix("disk-")
                        .and_then(|s| s.strip_suffix(".raw"))
                    {
                        args.push(format!("{}={}", drive_name, path.display()));
                        disk_count += 1;
                    }
                }
            }
        }

        if disk_count == 0 {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "No disk images found in {} to pack into VMA",
                    extract_dir.display()
                ),
            }
            .into());
        }

        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run_tool("vma", &args_refs, "VMA creation")?;
        Ok(())
    }
}

impl Default for ProxmoxVmaProvisioner {
    fn default() -> Self {
        Self::new()
    }
}

impl Provisioner for ProxmoxVmaProvisioner {
    fn name(&self) -> &str {
        "proxmox-vma"
    }

    fn check_dependencies(&self) -> Result<()> {
        for (tool, hint) in [
            (
                "vma",
                "Install from Proxmox VE or run bootstrap on a Proxmox host",
            ),
            ("zstd", "Install zstd: nix-shell -p zstd"),
            ("guestfish", "Install libguestfs: nix-shell -p libguestfs"),
        ] {
            if self.executor.which(tool).is_none() {
                return Err(ProvisionerError::MissingDependency {
                    provisioner: self.name().to_string(),
                    tool: tool.to_string(),
                    install_hint: hint.to_string(),
                }
                .into());
            }
        }
        Ok(())
    }

    fn inject_keys(
        &self,
        image_path: &Path,
        host_keys: &HostKeys,
        config: &ProvisionerConfig,
    ) -> Result<InjectionResult> {
        self.check_dependencies()?;

        let vma_file = self.find_vma_file(image_path, &host_keys.hostname)?;

        let temp_dir = tempfile::Builder::new()
            .prefix("snow-vma-")
            .tempdir()
            .map_err(|e| ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!("Failed to create temp directory: {}", e),
            })?;

        let work_dir = temp_dir.path();
        let extract_dir = work_dir.join("extracted");

        let vma_path = if Self::is_compressed(&vma_file) {
            let decompressed = work_dir.join("image.vma");
            self.decompress(&vma_file, &decompressed)?;
            decompressed
        } else {
            vma_file.clone()
        };

        self.extract_vma(&vma_path, &extract_dir)?;

        let root_disk = self.find_root_disk(&extract_dir)?;
        guestfish::inject_keys(self.name(), &root_disk, host_keys, config)?;

        let output_vma = work_dir.join("output.vma");
        self.create_vma(&extract_dir, &output_vma)?;

        let final_path = if Self::is_compressed(&vma_file) {
            PathBuf::from(format!("/tmp/vzdump-qemu-{}.vma.zst", host_keys.hostname))
        } else {
            PathBuf::from(format!("/tmp/vzdump-qemu-{}.vma", host_keys.hostname))
        };

        if Self::is_compressed(&vma_file) {
            self.compress(&output_vma, &final_path)?;
        } else {
            std::fs::copy(&output_vma, &final_path).map_err(|e| {
                ProvisionerError::InjectionFailed {
                    provisioner: self.name().to_string(),
                    reason: format!(
                        "Failed to copy output VMA to {}: {}",
                        final_path.display(),
                        e
                    ),
                }
            })?;
        }

        Ok(InjectionResult::Success {
            image_path: final_path,
        })
    }

    fn validate_target(&self, target: &TargetType) -> Result<()> {
        match target {
            TargetType::ProxmoxHost(_) => Ok(()),
            _ => Err(ProvisionerError::DeploymentNotSupported {
                provisioner: self.name().to_string(),
                reason: format!("proxmox-vma only supports Proxmox hosts, got {}", target),
            }
            .into()),
        }
    }

    fn deploy_to_target(
        &self,
        image_path: &Path,
        target: &TargetType,
        hostname: &str,
    ) -> Result<ProvisioningResult> {
        self.validate_target(target)?;

        let TargetType::ProxmoxHost(deploy_target) = target else {
            unreachable!("validate_target ensures only ProxmoxHost reaches here");
        };

        let deployer = VmaDeployer::new(self, deploy_target, image_path, hostname)?;
        deployer.deploy()
    }
}

/// Without BatchMode: bootstrapping a Proxmox host may need a password prompt
fn rsync_ssh_command(host: &DeployTarget) -> String {
    format!("ssh {}", connection_options(host.port).join(" "))
}

/// Handles the multi-step VMA deployment process
struct VmaDeployer<'a> {
    provisioner: &'a ProxmoxVmaProvisioner,
    host: DeployTarget,
    remote_path: String,
    hostname: String,
    vmid: u32,
    extract_dir: String,
}

impl<'a> VmaDeployer<'a> {
    fn new(
        provisioner: &'a ProxmoxVmaProvisioner,
        host: &DeployTarget,
        image_path: &Path,
        hostname: &str,
    ) -> Result<Self> {
        let filename = image_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image.vma.zst");
        let remote_path = format!("/tmp/{}", filename);

        provisioner.run_tool(
            "rsync",
            &[
                "-v",
                "--progress",
                "-e",
                &rsync_ssh_command(host),
                &image_path.to_string_lossy(),
                &format!("{}:{}", host.ssh_destination(), remote_path),
            ],
            "Image upload",
        )?;

        let vmid = Self::get_next_vmid(provisioner, host)?;
        let extract_dir = format!("/tmp/snow-extract-{}", vmid);

        Ok(Self {
            provisioner,
            host: host.clone(),
            remote_path,
            hostname: hostname.to_string(),
            vmid,
            extract_dir,
        })
    }

    fn get_next_vmid(provisioner: &ProxmoxVmaProvisioner, host: &DeployTarget) -> Result<u32> {
        let output = provisioner.run_remote(host, "pvesh get /cluster/nextid", "Get VMID")?;
        let vmid_str = output.stdout_str().trim().to_string();

        vmid_str.parse::<u32>().map_err(|_| {
            ProvisionerError::DeploymentFailed {
                provisioner: "proxmox-vma".to_string(),
                reason: format!("Invalid VMID returned by Proxmox: '{}'", vmid_str),
            }
            .into()
        })
    }

    fn deploy(self) -> Result<ProvisioningResult> {
        self.extract_vma()?;
        let vm_config = self.read_vm_config()?;
        let disk_path = self.find_disk()?;
        self.create_vm(&vm_config)?;
        let disk_volume = self.import_disk(&disk_path)?;
        self.configure_vm(&disk_volume)?;
        self.cleanup();
        self.start_vm()?;

        Ok(ProvisioningResult::Success {
            message: format!(
                "VM {} created and started on {} (VMID: {})",
                self.hostname, self.host.hostname, self.vmid
            ),
        })
    }

    fn extract_vma(&self) -> Result<()> {
        let cmd = format!(
            "rm -rf {} && zstd -d -c {} | vma extract - {}",
            shell_escape(&self.extract_dir),
            shell_escape(&self.remote_path),
            shell_escape(&self.extract_dir)
        );
        self.provisioner
            .run_remote(&self.host, &cmd, "VMA extraction")
            .map(|_| ())
    }

    fn read_vm_config(&self) -> Result<VmConfig> {
        let cmd = format!("cat {}/qemu-server.conf", shell_escape(&self.extract_dir));
        let output = self
            .provisioner
            .run_remote(&self.host, &cmd, "Read VM config")?;
        Ok(VmConfig::parse(&output.stdout_str()))
    }

    fn find_disk(&self) -> Result<String> {
        // Try to find the boot disk from qemu-server.conf first
        // Look for virtio0, scsi0, sata0, ide0 in that order (common boot disk names)
        for disk_type in ["virtio0", "scsi0", "sata0", "ide0"] {
            let disk_file = format!("{}/disk-drive-{}.raw", self.extract_dir, disk_type);
            let check_cmd = format!("test -f {} && echo exists", shell_escape(&disk_file));
            if let Ok(output) = self
                .provisioner
                .run_remote(&self.host, &check_cmd, "Check disk")
            {
                if output.stdout_str().contains("exists") {
                    return Ok(disk_file);
                }
            }
        }

        // Fallback: find any .raw file
        let cmd = format!(
            "ls {}/*.raw 2>/dev/null | head -1",
            shell_escape(&self.extract_dir)
        );
        let output = self.provisioner.run_remote(&self.host, &cmd, "Find disk")?;
        let disk_path = output.stdout_str().trim().to_string();

        if disk_path.is_empty() {
            return Err(ProvisionerError::DeploymentFailed {
                provisioner: self.provisioner.name().to_string(),
                reason: format!("No .raw disk file found in {}", self.extract_dir),
            }
            .into());
        }

        Ok(disk_path)
    }

    fn create_vm(&self, config: &VmConfig) -> Result<()> {
        let cmd = format!(
            "qm create {} --name {} --memory {} --cores {} \
             --net0 {} --scsihw {} --ostype {} \
             --bios seabios --serial0 socket --agent {}",
            self.vmid,
            shell_escape(&self.hostname),
            config.memory,
            config.cores,
            shell_escape(&config.net0),
            shell_escape(&config.scsihw),
            shell_escape(&config.ostype),
            config.agent,
        );
        self.provisioner
            .run_remote(&self.host, &cmd, "Create VM")
            .map(|_| ())
    }

    fn import_disk(&self, disk_path: &str) -> Result<String> {
        let cmd = format!(
            "qm importdisk {} {} local-lvm --format raw 2>&1",
            self.vmid,
            shell_escape(disk_path)
        );
        let output = self
            .provisioner
            .run_remote(&self.host, &cmd, "Import disk")?;

        let disk_volume = parse_imported_disk(&output.stdout_str()).unwrap_or_else(|| {
            tracing::warn!(
                "Failed to parse disk volume from importdisk output, using fallback. Output: {}",
                output.stdout_str()
            );
            format!("local-lvm:vm-{}-disk-0", self.vmid)
        });

        Ok(disk_volume)
    }

    fn configure_vm(&self, disk_volume: &str) -> Result<()> {
        let cmd = format!(
            "qm set {} --virtio0 {} --bootdisk virtio0 --boot order=virtio0",
            self.vmid,
            shell_escape(disk_volume)
        );
        self.provisioner
            .run_remote(&self.host, &cmd, "Configure VM")
            .map(|_| ())
    }

    fn cleanup(&self) {
        let cmd = format!(
            "rm -rf {} {}",
            shell_escape(&self.extract_dir),
            shell_escape(&self.remote_path)
        );
        if let Err(e) = self.provisioner.run_remote(&self.host, &cmd, "Cleanup") {
            tracing::warn!(
                "Failed to cleanup temporary files on {}: {}",
                self.host.ssh_destination(),
                e
            );
        }
    }

    fn start_vm(&self) -> Result<()> {
        let cmd = format!("qm start {}", self.vmid);
        self.provisioner
            .run_remote(&self.host, &cmd, "Start VM")
            .map(|_| ())
    }
}

/// VM configuration parsed from qemu-server.conf
struct VmConfig {
    memory: String,
    cores: String,
    scsihw: String,
    ostype: String,
    net0: String,
    agent: String,
}

impl VmConfig {
    fn parse(conf: &str) -> Self {
        Self {
            memory: parse_conf_value(conf, "memory").unwrap_or_else(|| "4096".to_string()),
            cores: parse_conf_value(conf, "cores").unwrap_or_else(|| "2".to_string()),
            scsihw: parse_conf_value(conf, "scsihw")
                .unwrap_or_else(|| "virtio-scsi-single".to_string()),
            ostype: parse_conf_value(conf, "ostype").unwrap_or_else(|| "l26".to_string()),
            net0: parse_conf_value(conf, "net0")
                .unwrap_or_else(|| "virtio,bridge=vmbr0,firewall=1".to_string()),
            agent: parse_conf_value(conf, "agent").unwrap_or_else(|| "1".to_string()),
        }
    }
}

/// Parse a value from Proxmox qemu-server.conf format (key: value)
fn parse_conf_value(conf: &str, key: &str) -> Option<String> {
    for line in conf.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(value) = rest.strip_prefix(':') {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Parse the imported disk volume from qm importdisk output
fn parse_imported_disk(output: &str) -> Option<String> {
    for line in output.lines() {
        if line.contains("Successfully imported") {
            if let Some(start) = line.find("unused0:") {
                let rest = &line[start + 8..];
                if let Some(end) = rest.find('\'') {
                    return Some(rest[..end].to_string());
                }
                return Some(rest.trim_end_matches(&['\'', '"'][..]).to_string());
            }
        }
    }
    None
}

/// Escape a string for safe shell use
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::provisioner::MockCommandExecutor;

    #[test]
    fn test_provisioner_name() {
        let provisioner = ProxmoxVmaProvisioner::new();
        assert_eq!(provisioner.name(), "proxmox-vma");
    }

    #[test]
    fn test_is_compressed() {
        assert!(ProxmoxVmaProvisioner::is_compressed(Path::new(
            "/tmp/image.vma.zst"
        )));
        assert!(!ProxmoxVmaProvisioner::is_compressed(Path::new(
            "/tmp/image.vma"
        )));
        assert!(!ProxmoxVmaProvisioner::is_compressed(Path::new(
            "/tmp/image.raw"
        )));
    }

    #[test]
    fn test_parse_conf_value() {
        let conf = "memory: 4096\ncores: 2\nscsihw: virtio-scsi-single\n";
        assert_eq!(parse_conf_value(conf, "memory"), Some("4096".to_string()));
        assert_eq!(parse_conf_value(conf, "cores"), Some("2".to_string()));
        assert_eq!(
            parse_conf_value(conf, "scsihw"),
            Some("virtio-scsi-single".to_string())
        );
        assert_eq!(parse_conf_value(conf, "missing"), None);
    }

    #[test]
    fn test_parse_imported_disk() {
        let output = "  Logical volume \"vm-100-disk-0\" created.\nSuccessfully imported disk as 'unused0:local-lvm:vm-100-disk-0'";
        assert_eq!(
            parse_imported_disk(output),
            Some("local-lvm:vm-100-disk-0".to_string())
        );
    }

    #[test]
    fn test_parse_imported_disk_no_match() {
        let output = "Some random output without disk info";
        assert_eq!(parse_imported_disk(output), None);
    }

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn test_shell_escape_with_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_shell_escape_with_spaces() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }

    #[test]
    fn test_vm_config_parse_defaults() {
        let config = VmConfig::parse("");
        assert_eq!(config.memory, "4096");
        assert_eq!(config.cores, "2");
        assert_eq!(config.scsihw, "virtio-scsi-single");
        assert_eq!(config.ostype, "l26");
    }

    #[test]
    fn test_vm_config_parse_custom() {
        let conf = "memory: 8192\ncores: 4\nscsihw: virtio-scsi-pci\nostype: l26\nnet0: virtio,bridge=vmbr1\nagent: 0";
        let config = VmConfig::parse(conf);
        assert_eq!(config.memory, "8192");
        assert_eq!(config.cores, "4");
        assert_eq!(config.scsihw, "virtio-scsi-pci");
        assert_eq!(config.net0, "virtio,bridge=vmbr1");
        assert_eq!(config.agent, "0");
    }

    #[test]
    fn test_validate_target_accepts_proxmox_host() {
        let provisioner = ProxmoxVmaProvisioner::new();
        let target = TargetType::parse("proxmox01.local");
        assert!(provisioner.validate_target(&target).is_ok());
    }

    #[test]
    fn test_validate_target_rejects_block_device() {
        let provisioner = ProxmoxVmaProvisioner::new();
        let target = TargetType::BlockDevice(PathBuf::from("/dev/sdc"));
        let result = provisioner.validate_target(&target);
        assert!(result.is_err());
    }

    #[test]
    fn test_check_dependencies_with_mock() {
        let mock = MockCommandExecutor::new().with_available_commands(vec![
            "vma".to_string(),
            "zstd".to_string(),
            "guestfish".to_string(),
        ]);

        let provisioner = ProxmoxVmaProvisioner::with_executor(Arc::new(mock));
        assert!(provisioner.check_dependencies().is_ok());
    }

    #[test]
    fn test_check_dependencies_missing_vma() {
        let mock = MockCommandExecutor::new()
            .with_available_commands(vec!["zstd".to_string(), "guestfish".to_string()]);

        let provisioner = ProxmoxVmaProvisioner::with_executor(Arc::new(mock));
        let result = provisioner.check_dependencies();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("vma"));
    }

    #[test]
    fn test_run_tool_success() {
        let mock = MockCommandExecutor::new()
            .with_response("zstd", MockCommandExecutor::success_output("compressed"));

        let provisioner = ProxmoxVmaProvisioner::with_executor(Arc::new(mock));
        let result = provisioner.run_tool("zstd", &["-d", "test.zst"], "Decompress");
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_tool_failure() {
        let mock = MockCommandExecutor::new()
            .with_response("zstd", MockCommandExecutor::failure_output("zstd: error"))
            .with_default_success(false);

        let provisioner = ProxmoxVmaProvisioner::with_executor(Arc::new(mock));
        let result = provisioner.run_tool("zstd", &["-d", "test.zst"], "Decompress");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Decompress failed"));
    }

    // Integration tests for deploy_to_target

    #[test]
    fn test_vma_deployer_get_next_vmid_success() {
        let mock = MockCommandExecutor::new()
            .with_response("pvesh", MockCommandExecutor::success_output("100\n"));
        let provisioner = ProxmoxVmaProvisioner::with_executor(Arc::new(mock));

        let result = VmaDeployer::get_next_vmid(&provisioner, &DeployTarget::parse("root@proxmox"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 100);
    }

    #[test]
    fn test_vma_deployer_get_next_vmid_invalid() {
        let mock = MockCommandExecutor::new().with_response(
            "pvesh",
            MockCommandExecutor::success_output("not-a-number\n"),
        );
        let provisioner = ProxmoxVmaProvisioner::with_executor(Arc::new(mock));

        let result = VmaDeployer::get_next_vmid(&provisioner, &DeployTarget::parse("root@proxmox"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid VMID"));
    }

    #[test]
    fn test_run_remote_success() {
        let mock = MockCommandExecutor::new()
            .with_response("ssh", MockCommandExecutor::success_output("success"));
        let provisioner = ProxmoxVmaProvisioner::with_executor(Arc::new(mock));

        let result = provisioner.run_remote(
            &DeployTarget::parse("root@proxmox"),
            "echo test",
            "Test command",
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap().stdout_str(), "success");
    }

    #[test]
    fn test_run_remote_connects_to_the_hosts_port_without_forwarding_the_agent() {
        let mock = Arc::new(
            MockCommandExecutor::new()
                .with_response("ssh", MockCommandExecutor::success_output("101\n")),
        );
        let provisioner = ProxmoxVmaProvisioner::with_executor(mock.clone());

        provisioner
            .run_remote(
                &DeployTarget::parse("root@proxmox:2222"),
                "pvesh get /cluster/nextid",
                "Get VMID",
            )
            .unwrap();

        assert_eq!(
            mock.get_executed(),
            vec![(
                "ssh".to_string(),
                [
                    "-o",
                    "ForwardAgent=no",
                    "-p",
                    "2222",
                    "root@proxmox",
                    "pvesh get /cluster/nextid"
                ]
                .map(String::from)
                .to_vec()
            )]
        );
    }

    #[test]
    fn test_image_upload_reaches_the_hosts_port_without_forwarding_the_agent() {
        let mock = Arc::new(
            MockCommandExecutor::new()
                .with_response("pvesh", MockCommandExecutor::success_output("101\n")),
        );
        let provisioner = ProxmoxVmaProvisioner::with_executor(mock.clone());

        VmaDeployer::new(
            &provisioner,
            &DeployTarget::parse("root@proxmox:2222"),
            Path::new("/tmp/hedgehog.vma.zst"),
            "hedgehog",
        )
        .unwrap();

        let (program, args) = &mock.get_executed()[0];
        assert_eq!(program, "rsync");
        assert_eq!(
            args,
            &[
                "-v",
                "--progress",
                "-e",
                "ssh -o ForwardAgent=no -p 2222",
                "/tmp/hedgehog.vma.zst",
                "root@proxmox:/tmp/hedgehog.vma.zst"
            ]
            .map(String::from)
        );
    }

    #[test]
    fn test_run_remote_failure() {
        let mock = MockCommandExecutor::new()
            .with_response(
                "ssh",
                MockCommandExecutor::failure_output("connection refused"),
            )
            .with_default_success(false);
        let provisioner = ProxmoxVmaProvisioner::with_executor(Arc::new(mock));

        let result = provisioner.run_remote(
            &DeployTarget::parse("root@proxmox"),
            "echo test",
            "Test command",
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Test command failed"));
    }

    #[test]
    fn test_vm_config_preserves_network_settings() {
        let conf = r#"memory: 8192
cores: 4
net0: virtio=AA:BB:CC:DD:EE:FF,bridge=vmbr1,tag=100
agent: 0
"#;
        let config = VmConfig::parse(conf);
        assert_eq!(config.net0, "virtio=AA:BB:CC:DD:EE:FF,bridge=vmbr1,tag=100");
        assert_eq!(config.agent, "0");
    }
}
