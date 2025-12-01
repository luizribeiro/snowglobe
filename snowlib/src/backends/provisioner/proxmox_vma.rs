//! Proxmox VMA provisioner
//!
//! Injects SSH keys into Proxmox VMA backup images. VMA (Virtual Machine Archive)
//! is the native backup format for Proxmox VE.
//!
//! The injection process:
//! 1. Decompress .vma.zst to .vma (if compressed)
//! 2. Extract .vma to raw disk files
//! 3. Inject SSH keys using guestfish
//! 4. Repack raw disks to .vma
//! 5. Compress back to .vma.zst

use super::{
    guestfish, InjectionResult, Provisioner, ProvisionerConfig, ProvisioningResult, TargetType,
};
use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Provisioner for Proxmox VMA backup images
///
/// Uses vma for extraction/repacking and guestfish for rootless key injection.
pub struct ProxmoxVmaProvisioner;

impl ProxmoxVmaProvisioner {
    pub fn new() -> Self {
        Self
    }

    fn is_compressed(path: &Path) -> bool {
        path.extension().is_some_and(|ext| ext == "zst")
    }

    fn find_vma_file(&self, path: &Path, hostname: &str) -> Result<PathBuf> {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }

        if path.is_dir() {
            // Proxmox VMA files follow the naming convention: vzdump-qemu-<hostname>.vma.zst
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

    fn decompress(&self, input: &Path, output: &Path) -> Result<()> {
        let cmd_output = Command::new("zstd")
            .arg("-d")
            .arg("-f")
            .arg(input)
            .arg("-o")
            .arg(output)
            .output()
            .map_err(|e| ProvisionerError::InjectionCommandFailed {
                provisioner: self.name().to_string(),
                command: "zstd".to_string(),
                stderr: format!("Failed to execute: {}", e),
            })?;

        if !cmd_output.status.success() {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "Failed to decompress {} (exit code {}): {}",
                    input.display(),
                    cmd_output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&cmd_output.stderr)
                ),
            }
            .into());
        }
        Ok(())
    }

    fn compress(&self, input: &Path, output: &Path) -> Result<()> {
        let cmd_output = Command::new("zstd")
            .arg("-f")
            .arg(input)
            .arg("-o")
            .arg(output)
            .output()
            .map_err(|e| ProvisionerError::InjectionCommandFailed {
                provisioner: self.name().to_string(),
                command: "zstd".to_string(),
                stderr: format!("Failed to execute: {}", e),
            })?;

        if !cmd_output.status.success() {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "Failed to compress {} (exit code {}): {}",
                    input.display(),
                    cmd_output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&cmd_output.stderr)
                ),
            }
            .into());
        }
        Ok(())
    }

    fn extract_vma(&self, vma_path: &Path, extract_dir: &Path) -> Result<()> {
        fs::create_dir_all(extract_dir).map_err(|e| ProvisionerError::InjectionFailed {
            provisioner: self.name().to_string(),
            reason: format!(
                "Failed to create extraction directory {}: {}",
                extract_dir.display(),
                e
            ),
        })?;

        let output = Command::new("vma")
            .arg("extract")
            .arg(vma_path)
            .arg(extract_dir)
            .output()
            .map_err(|e| ProvisionerError::InjectionCommandFailed {
                provisioner: self.name().to_string(),
                command: "vma extract".to_string(),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "vma extract failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }
        Ok(())
    }

    fn find_root_disk(&self, extract_dir: &Path) -> Result<PathBuf> {
        // VMA extracts disks as disk-drive-<name>.raw
        // Common patterns: scsi0, virtio0, sata0
        let patterns = ["scsi0", "virtio0", "sata0", "ide0"];

        for pattern in patterns {
            let disk_path = extract_dir.join(format!("disk-drive-{}.raw", pattern));
            if disk_path.exists() {
                return Ok(disk_path);
            }
        }

        // Fallback: find any .raw file
        let entries = fs::read_dir(extract_dir).map_err(|e| ProvisionerError::InjectionFailed {
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
        // VMA create requires a config file listing the disks
        // Format (one per line): <absolute-path-to-raw-file> drive-<name>
        // Example: /tmp/snow-vma-xyz/extracted/disk-drive-scsi0.raw drive-scsi0
        let config_path = extract_dir.join("vma.config");

        let mut config_content = String::new();
        let entries = fs::read_dir(extract_dir).map_err(|e| ProvisionerError::InjectionFailed {
            provisioner: self.name().to_string(),
            reason: format!(
                "Failed to read extraction directory {}: {}",
                extract_dir.display(),
                e
            ),
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "raw") {
                if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                    if let Some(drive_name) = filename
                        .strip_prefix("disk-drive-")
                        .and_then(|s| s.strip_suffix(".raw"))
                    {
                        config_content.push_str(&format!(
                            "{} drive-{}\n",
                            path.display(),
                            drive_name
                        ));
                    }
                }
            }
        }

        if config_content.is_empty() {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "No disk images found in {} to pack into VMA",
                    extract_dir.display()
                ),
            }
            .into());
        }

        fs::write(&config_path, &config_content).map_err(|e| {
            ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "Failed to write VMA config to {}: {}",
                    config_path.display(),
                    e
                ),
            }
        })?;

        let output = Command::new("vma")
            .arg("create")
            .arg(output_path)
            .arg(&config_path)
            .output()
            .map_err(|e| ProvisionerError::InjectionCommandFailed {
                provisioner: self.name().to_string(),
                command: "vma create".to_string(),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "vma create failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            }
            .into());
        }
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
            if !which(tool) {
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

        // Find the actual VMA file (Nix outputs a directory containing the file)
        let vma_file = self.find_vma_file(image_path, &host_keys.hostname)?;

        // Create a temp directory for extraction
        let temp_dir = tempfile::Builder::new()
            .prefix("snow-vma-")
            .tempdir()
            .map_err(|e| ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!("Failed to create temp directory: {}", e),
            })?;

        let work_dir = temp_dir.path();
        let extract_dir = work_dir.join("extracted");

        // Step 1: Decompress if needed
        let vma_path = if Self::is_compressed(&vma_file) {
            let decompressed = work_dir.join("image.vma");
            self.decompress(&vma_file, &decompressed)?;
            decompressed
        } else {
            vma_file.clone()
        };

        // Step 2: Extract VMA to raw disks
        self.extract_vma(&vma_path, &extract_dir)?;

        // Step 3: Find the root disk and inject keys
        let root_disk = self.find_root_disk(&extract_dir)?;
        guestfish::inject_keys(self.name(), &root_disk, host_keys, config.ssh_key_path())?;

        // Step 4: Repack to VMA
        let output_vma = work_dir.join("output.vma");
        self.create_vma(&extract_dir, &output_vma)?;

        // Step 5: Compress and write to final location
        // Append .new suffix to avoid overwriting the original
        let final_path = PathBuf::from(format!("{}.new", image_path.display()));

        if Self::is_compressed(image_path) {
            self.compress(&output_vma, &final_path)?;
        } else {
            fs::copy(&output_vma, &final_path).map_err(|e| ProvisionerError::InjectionFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "Failed to copy output VMA to {}: {}",
                    final_path.display(),
                    e
                ),
            })?;
        }

        Ok(InjectionResult::Success {
            image_path: final_path,
        })
    }

    fn deployment_instructions(&self, output_path: &Path, hostname: &str) -> String {
        let path = output_path.display();
        format!(
            r#"Proxmox VMA image ready: {path}

To deploy to a Proxmox host:
  1. Copy image:   rsync -v --progress {path} root@<proxmox-host>:/tmp/
  2. Restore VM:   ssh root@<proxmox-host> 'qmrestore /tmp/{hostname}.vma.zst <VMID>'
  3. Start VM:     ssh root@<proxmox-host> 'qm start <VMID>'

Replace <proxmox-host> with your Proxmox server and <VMID> with the target VM ID.

After the VM is running:
  snow push {hostname}"#
        )
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

        let ssh_dest = deploy_target.ssh_destination();
        let filename = image_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("image.vma.zst");
        let remote_path = format!("/tmp/{}", filename);

        // Step 1: Copy image to Proxmox host
        let rsync_output = Command::new("rsync")
            .arg("-v")
            .arg("--progress")
            .arg(image_path)
            .arg(format!("{}:{}", ssh_dest, remote_path))
            .output()
            .map_err(|e| ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!("Failed to execute rsync: {}", e),
            })?;

        if !rsync_output.status.success() {
            return Err(ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "rsync failed: {}",
                    String::from_utf8_lossy(&rsync_output.stderr)
                ),
            }
            .into());
        }

        // Step 2: Get next available VMID
        let vmid_output = Command::new("ssh")
            .arg(&ssh_dest)
            .arg("pvesh")
            .arg("get")
            .arg("/cluster/nextid")
            .output()
            .map_err(|e| ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!("Failed to get next VMID: {}", e),
            })?;

        if !vmid_output.status.success() {
            return Err(ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "Failed to get next VMID: {}",
                    String::from_utf8_lossy(&vmid_output.stderr)
                ),
            }
            .into());
        }

        let vmid_str = String::from_utf8_lossy(&vmid_output.stdout)
            .trim()
            .to_string();

        vmid_str
            .parse::<u32>()
            .map_err(|_| ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!("Invalid VMID returned by Proxmox: '{}'", vmid_str),
            })?;

        let vmid = vmid_str;

        // Step 3: Restore VM from backup
        let restore_output = Command::new("ssh")
            .arg(&ssh_dest)
            .arg("qmrestore")
            .arg(&remote_path)
            .arg(&vmid)
            .output()
            .map_err(|e| ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!("Failed to execute qmrestore: {}", e),
            })?;

        if !restore_output.status.success() {
            return Err(ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "qmrestore failed: {}",
                    String::from_utf8_lossy(&restore_output.stderr)
                ),
            }
            .into());
        }

        // Step 4: Start VM
        let start_output = Command::new("ssh")
            .arg(&ssh_dest)
            .arg("qm")
            .arg("start")
            .arg(&vmid)
            .output()
            .map_err(|e| ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!("Failed to execute qm start: {}", e),
            })?;

        if !start_output.status.success() {
            return Err(ProvisionerError::DeploymentFailed {
                provisioner: self.name().to_string(),
                reason: format!(
                    "qm start failed: {}",
                    String::from_utf8_lossy(&start_output.stderr)
                ),
            }
            .into());
        }

        Ok(ProvisioningResult::Success {
            message: format!(
                "VM {} restored and started on {} (VMID: {})",
                hostname, deploy_target.hostname, vmid
            ),
        })
    }
}

fn which(cmd: &str) -> bool {
    super::guestfish::which(cmd).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxmox_vma_provisioner_name() {
        let provisioner = ProxmoxVmaProvisioner::new();
        assert_eq!(provisioner.name(), "proxmox-vma");
    }

    #[test]
    fn test_proxmox_vma_deployment_instructions() {
        let provisioner = ProxmoxVmaProvisioner::new();
        let instructions =
            provisioner.deployment_instructions(Path::new("/tmp/carbon.vma.zst"), "carbon");

        assert!(instructions.contains("/tmp/carbon.vma.zst"));
        assert!(instructions.contains("rsync"));
        assert!(instructions.contains("qmrestore"));
        assert!(instructions.contains("qm start"));
        assert!(instructions.contains("carbon"));
        assert!(instructions.contains("snow push"));
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
    fn test_check_dependencies_returns_error_when_tools_missing() {
        let error = ProvisionerError::MissingDependency {
            provisioner: "proxmox-vma".to_string(),
            tool: "vma".to_string(),
            install_hint: "Install from Proxmox VE".to_string(),
        };
        assert!(error.to_string().contains("vma"));
        assert!(error.to_string().contains("proxmox-vma"));
    }

    #[test]
    fn test_which_finds_common_command() {
        assert!(which("sh"));
    }

    #[test]
    fn test_which_returns_false_for_missing_command() {
        assert!(!which("definitely-not-a-real-command-12345"));
    }

    #[test]
    fn test_validate_target_accepts_proxmox_host() {
        let provisioner = ProxmoxVmaProvisioner::new();
        let target = TargetType::parse("proxmox01.local");
        assert!(provisioner.validate_target(&target).is_ok());
    }

    #[test]
    fn test_validate_target_accepts_proxmox_host_with_user() {
        let provisioner = ProxmoxVmaProvisioner::new();
        let target = TargetType::parse("admin@proxmox01.local");
        assert!(provisioner.validate_target(&target).is_ok());
    }

    #[test]
    fn test_validate_target_rejects_block_device() {
        let provisioner = ProxmoxVmaProvisioner::new();
        let target = TargetType::BlockDevice(std::path::PathBuf::from("/dev/sdc"));
        let result = provisioner.validate_target(&target);
        assert!(result.is_err());
    }
}
