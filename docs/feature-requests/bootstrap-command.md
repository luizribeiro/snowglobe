# Feature Request: `snow bootstrap` Command

## Summary

Add host provisioning capabilities to snowglobe through a modular `snow bootstrap` command that handles the pre-deployment lifecycle: generating host identity (SSH keys), integrating with secrets management, and preparing bootable images with injected keys.

## Problem Statement

When deploying a new NixOS host that uses secrets management (SOPS, agenix, etc.), there's a chicken-and-egg problem:

1. The host needs its SSH host key to derive the age key for secrets decryption
2. The SSH host key doesn't exist until the host boots
3. The host can't boot properly without decrypting its secrets

The manual workflow is tedious and error-prone:

```
Generate SSH keys → Convert to age → Update secrets config → Re-encrypt →
Build image → Extract → Mount → Inject keys → Repackage → Deploy
```

This process differs for each deployment target (Proxmox VMA, Raspberry Pi SD image, bare metal, etc.) and requires knowledge of target-specific tooling.

## Design Principles

1. **Separation of Concerns**: Key management, image building, and key injection are distinct operations
2. **Nix Owns Image Building**: Image formats are defined in Nix flakes, not reimplemented in Rust
3. **Extensible Target System**: Provisioners are pluggable—new targets can be added without changing core logic
4. **Progressive Enhancement**: Start with key generation, add SOPS integration later
5. **Fail Explicitly**: Clear errors with recovery steps, not silent failures
6. **Rootless First**: Prefer userspace tools over sudo/mount whenever possible

## Architecture

### Three-Layer Design

```
┌─────────────────────────────────────────────────────────────┐
│                    snow bootstrap <host>                     │
├─────────────────────────────────────────────────────────────┤
│  1. Key Management        │  2. Image Building  │  3. Key    │
│  ─────────────────        │  ──────────────────  │  Injection │
│  • Generate SSH keypair   │  • Delegate to Nix  │  ──────────│
│  • Derive age public key  │  • Build flake attr │  • Target- │
│  • Store keys securely    │  • User defines     │    specific│
│  • (Future: SOPS update)  │    image format     │  • Modular │
└─────────────────────────────────────────────────────────────┘
                                    │
                    ┌───────────────┼───────────────┐
                    ▼               ▼               ▼
              ┌──────────┐   ┌──────────┐   ┌──────────┐
              │ Proxmox  │   │ SD Image │   │Raw/Manual│
              │Provisioner│  │Provisioner│  │Provisioner│
              └──────────┘   └──────────┘   └──────────┘
```

### Provisioner Trait

Each deployment target implements a common interface:

```rust
pub trait Provisioner: Send + Sync {
    /// Human-readable name for this provisioner
    fn name(&self) -> &str;

    /// Check if required external tools are available
    fn check_dependencies(&self) -> Result<(), ProvisionerError>;

    /// Inject SSH keys into the built image
    /// Returns path to the modified image (may be same as input for in-place)
    fn inject_keys(
        &self,
        image_path: &Path,
        host_keys: &HostKeys,
        config: &ProvisionerConfig,
    ) -> Result<PathBuf, ProvisionerError>;

    /// Validate that a target is appropriate for this provisioner
    fn validate_target(&self, target: &str) -> Result<TargetType, ProvisionerError>;

    /// Deploy to the specified target
    fn deploy_to_target(
        &self,
        image_path: &Path,
        target: &str,
        hostname: &str,
        config: &ProvisionerConfig,
    ) -> Result<(), ProvisionerError>;

    /// Instructions for manual deployment (shown when no --target provided)
    fn deployment_instructions(&self, output_path: &Path, hostname: &str) -> String;
}

pub enum TargetType {
    BlockDevice(PathBuf),   // /dev/sdc
    SshHost(String),        // user@host
    Hostname(String),       // proxmox01.local
}
```

### Target Support by Provisioner

| Provisioner | Supported Targets | Method |
|-------------|-------------------|--------|
| `proxmox-vma` | Hostname | rsync + qmrestore + qm start |
| `remote` | SSH (user@host) | disko + nixos-install |
| `sd-image` | Block device | dd |
| `raw` | — | Instructions only |
| `qcow2` | — | Instructions only |
| `manual` | — | Instructions only |

### Host Keys Structure

```rust
pub struct HostKeys {
    pub hostname: String,
    pub ssh_private_key: PathBuf,  // ed25519
    pub ssh_public_key: PathBuf,
    pub age_public_key: String,    // Derived from SSH public key
}
```

## Phased Implementation

### Phase 1: Key Generation (MVP)

Minimal `snow bootstrap` that generates keys and outputs instructions. The command grows to include build and injection in later phases.

```bash
$ snow bootstrap carbon

Bootstrapping carbon...
  ✓ Generated SSH host key

  Age public key: age1v35854ga7r4xl5x8ehwz5tf32dn2z6asrpsrfekke4tsredcwalszjx6np

To use with SOPS, add to .sops.yaml:
  keys:
    - &carbon age1v35854ga7r4xl5x8ehwz5tf32dn2z6asrpsrfekke4tsredcwalszjx6np

Then run: sops updatekeys <your-secrets-file>

Note: Image building and key injection coming in future releases.
```

**Scope**:
- `ssh-keygen` for key generation
- `ssh-to-age` for age derivation
- Keys stored in secure temp directory
- Output age public key for SOPS configuration

### Phase 2: Image Building + Key Injection

Add the provisioner system. Key insight: **build the image first**, then generate keys and inject. This avoids orphaned keys if the Nix build fails.

```bash
$ snow bootstrap carbon

Bootstrapping carbon (sd-image)...
  → Building image...
  ✓ Built image: .#nixosConfigurations.carbon.config.system.build.sdImage
  → Generating SSH host key...
  ✓ Generated SSH host key
  → Injecting keys...
  ✓ Injected SSH keys

  Age public key: age1v35854ga7r4xl5x8ehwz5tf32dn2z6asrpsrfekke4tsredcwalszjx6np

Output: /tmp/snow-bootstrap-x7k2m/carbon.img

To deploy manually:
  sudo dd if=/tmp/snow-bootstrap-x7k2m/carbon.img of=/dev/sdX bs=4M status=progress
```

**Initial Provisioners**:

| Provisioner | Image Type | Key Injection Method |
|------------|------------|----------------------|
| `sd-image` | Raw disk image | guestfish or virt-copy-in |
| `raw` | Generic raw image | guestfish or virt-copy-in |
| `manual` | Any/none | Output keys only, user handles injection |

### Phase 3: More Provisioners

Add provisioners for other targets:

| Provisioner | Image Type | Dependencies |
|------------|------------|--------------|
| `proxmox-vma` | Proxmox backup format | `vma`, `zstd`, `guestfish` |
| `qcow2` | QEMU disk image | `guestfish` |

### Phase 4: SOPS Integration

Automatic secrets management (opt-in):

```bash
$ snow bootstrap carbon --update-sops

Bootstrapping carbon...
  ✓ Generated SSH host key
  ✓ Derived age key: age1v35854...
  ✓ Updated .sops.yaml (added carbon)
  ✓ Re-encrypted secrets/hosts/carbon.yaml
  ✓ Built image
  ✓ Injected SSH keys

Output: ./result/carbon.vma.zst
```

**SOPS Integration Scope**:
- Parse and update `.sops.yaml`
- Run `sops updatekeys` on affected files
- SOPS-nix patterns only (no agenix support initially)

## NixOS Module Integration

### Host Configuration

```nix
{
  snowglobe.bootstrap = {
    # Which provisioner to use for this host
    provisioner = "sd-image";  # or "proxmox-vma", "raw", "manual"

    # Flake attribute that builds the image
    # If not set, snowglobe will try to detect it
    imageAttr = "config.system.build.sdImage";

    # Where to inject SSH host keys in the image
    sshKeyPath = "/etc/ssh";

    # Additional files to inject (future)
    # extraFiles = { "/etc/machine-id" = ./machine-id; };
  };
}
```

### Auto-Detection (Optional)

If `provisioner` is not explicitly set, snowglobe can infer it:

```nix
# Detection heuristics (evaluated in snowlib)
if hasAttr "proxmox" config then "proxmox-vma"
else if hasAttr "sdImage" config.system.build then "sd-image"
else if hasAttr "isoImage" config.system.build then "iso"
else "manual"
```

## Command-Line Interface

### `snow bootstrap`

```bash
# Full bootstrap: generate keys + build + inject (no deployment)
snow bootstrap <hostname>

# Build, inject, AND deploy to target
snow bootstrap <hostname> --target <destination>

# Specify output location for the final image
snow bootstrap <hostname> --output ./images

# Override provisioner (ignore host config)
snow bootstrap <hostname> --provisioner sd-image

# Dry run - show what would be done
snow bootstrap <hostname> --dry-run

# With SOPS integration (Phase 5)
snow bootstrap <hostname> --update-sops
```

Note: Bootstrap operates on a single host at a time. Bootstrapping is a deliberate, one-time operation per host—batch operations don't fit the workflow.

### Deployment Examples

**Proxmox**:
```bash
$ snow bootstrap carbon --target proxmox01.local

Bootstrapping carbon (proxmox-vma)...
  ✓ Generated SSH host key
  ✓ Built VMA image
  ✓ Injected SSH keys

Deploying to proxmox01.local...
  → Copying image to proxmox01.local:/tmp/carbon.vma.zst
  → Restoring VM (qmrestore 105)
  → Starting VM (qm start 105)
  ✓ VM started

carbon is now running. Ready for: snow push carbon
```

**SD card**:
```bash
$ snow bootstrap carbon --target /dev/sdc

Bootstrapping carbon (sd-image)...
  ✓ Generated SSH host key
  ✓ Built SD image
  ✓ Injected SSH keys

⚠ WARNING: All data on /dev/sdc will be destroyed.
Continue? [y/N] y

  → Flashing image to /dev/sdc...
  ✓ Flash complete (2m 14s)

Insert the SD card into carbon and power on.
Ready for: snow push carbon
```

**Remote installer** (Phase 8):
```bash
$ snow bootstrap carbon --target root@192.168.1.100

Bootstrapping carbon (remote)...
  ✓ Generated SSH host key
  → Connecting to installer at 192.168.1.100...
  → Partitioning disks (disko)
  → Installing NixOS
  → Injecting SSH keys
  → Rebooting...
  ✓ Installation complete

carbon is now running. Ready for: snow push carbon
```

**No target (output only)**:
```bash
$ snow bootstrap carbon

Bootstrapping carbon (sd-image)...
  ✓ Generated SSH host key
  ✓ Built image
  ✓ Injected SSH keys

Output: /tmp/snow-bootstrap-x7k2m/carbon.img

To deploy manually:
  sudo dd if=/tmp/snow-bootstrap-x7k2m/carbon.img of=/dev/sdX bs=4M status=progress

After the host is running: snow push carbon
```

## Key Storage

Private keys are **ephemeral**—they exist only during the bootstrap process and are securely deleted after injection:

```
/tmp/snow-bootstrap-XXXXXX/    # Secure temp directory
└── carbon/
    ├── ssh_host_ed25519_key      # 0600 permissions
    └── ssh_host_ed25519_key.pub  # 0644 permissions
```

**Lifecycle**:
1. Keys generated in a secure temp directory with restricted permissions
2. Keys injected into the image
3. **Private key deleted immediately after injection**
4. Only the age public key (derived from SSH public key) is shown to user for SOPS config

**Why ephemeral?**
- Private keys should never be in your repo
- Once injected, the host owns its identity—you don't need the private key
- The age public key is all you need for secrets configuration
- If you need to recover, regenerate keys and re-bootstrap (or restore from backup)

**Recovery**: If a host is lost and needs to be rebuilt with the same identity, that's a key rotation scenario—covered in future extensions.

## Security Considerations

### Images Containing Private Keys Are Sensitive

Once SSH keys are injected, the output image is a sensitive artifact. Anyone with the image can:
- Impersonate the host
- Decrypt any SOPS secrets encrypted for that host's age key
- Potentially access systems that trust that host key

### Precautions Taken by Snowglobe

**1. Restrictive file permissions**

Output images are created with `0600` permissions:

```rust
fn write_output_image(path: &Path, data: &[u8]) -> Result<()> {
    let file = File::create(path)?;
    file.set_permissions(Permissions::from_mode(0o600))?;
    // ...
}
```

**2. Warn if output is in a git repository**

```bash
$ snow bootstrap carbon --output ./images/

Warning: Output directory is inside a git repository.
The generated image contains SSH private keys and should NOT be committed.

Ensure ./images/ is in your .gitignore.

Continue? [y/N]
```

**3. Secure temp directory for intermediate files**

All intermediate files (extracted disks, uncompressed images) use a secure temp directory that's cleaned up on completion or failure:

```rust
let work_dir = tempfile::Builder::new()
    .prefix("snow-bootstrap-")
    .permissions(Permissions::from_mode(0o700))
    .tempdir()?;
```

**4. Post-deployment cleanup prompt**

After successful deployment, offer to securely delete the image:

```bash
$ snow bootstrap carbon

✓ Bootstrap complete: ./result/carbon.img

The image contains SSH private keys. Now that it's deployed,
you may want to delete it.

Securely delete ./result/carbon.img? [y/N] y
✓ Image securely deleted
```

For `--output` to temp directory (default), cleanup is automatic.

**5. Default output to temp directory**

By default, images are written to a temp location rather than the current directory:

```bash
$ snow bootstrap carbon

Output: /tmp/snow-bootstrap-a8f3d2/carbon.img

# Explicitly specify output to keep it:
$ snow bootstrap carbon --output ./images/carbon.img
```

This encourages a "use and discard" workflow—build the image, flash/deploy it, then it's gone.

### User Responsibilities

- **Don't commit images** to version control
- **Delete images** after deployment when possible
- **Secure transfer**: If copying images to a deployment target, use encrypted channels (scp, rsync over SSH)

## Rootless Image Manipulation

A key design goal is avoiding sudo whenever possible. Several userspace tools enable this:

### Tool Overview

| Tool | Use Case | Filesystems | Notes |
|------|----------|-------------|-------|
| [libguestfs/guestfish](https://libguestfs.org/) | General disk image access | ext2/3/4, xfs, btrfs, FAT, NTFS | Uses internal mini-VM |
| [virt-copy-in](https://libguestfs.org/virt-copy-in.1.html) | Simple file injection | Same as libguestfs | Wrapper around guestfish |
| [mtools](https://www.gnu.org/software/mtools/manual/mtools.html) | FAT filesystem access | FAT12/16/32 only | Perfect for boot partitions |
| [nbdfuse](https://libguestfs.org/nbdfuse.1.html) | Expose disk as file | Any (via qemu-nbd) | FUSE-based, no root |
| fuse2fs | Mount ext filesystems | ext2/3/4 | Userspace ext driver |
| QEMU FUSE exports | Block device access | Any QEMU-supported | Since QEMU 6.0 |

### Strategy Per Image Type

**SD Images (Raspberry Pi)**:
- Boot partition (FAT): Use `mtools` (mcopy)
- Root partition (ext4): Use `guestfish` or `fuse2fs`

**Raw disk images**:
- Use `guestfish` / `virt-copy-in` for direct file injection

**qcow2 images**:
- Use `nbdfuse` + `qemu-nbd` to expose as raw, then `fuse2fs`
- Or use `guestfish` which handles qcow2 natively

**VMA (Proxmox)**:
- Extract to raw with `vma` tool
- Then use `guestfish` on the raw disk

### Example: Rootless SD Image Injection

```bash
# Using guestfish (no root required)
guestfish --rw -a raspberry-pi.img <<EOF
  run
  mount /dev/sda2 /
  copy-in /tmp/ssh_host_ed25519_key /etc/ssh/
  copy-in /tmp/ssh_host_ed25519_key.pub /etc/ssh/
  chmod 0600 /etc/ssh/ssh_host_ed25519_key
  chmod 0644 /etc/ssh/ssh_host_ed25519_key.pub
EOF

# Or using virt-copy-in (simpler syntax)
virt-copy-in -a raspberry-pi.img /tmp/ssh_keys/* /etc/ssh/
```

### Fallback Strategy

If rootless tools aren't available or fail, provisioners can fall back to sudo-based mounting with a clear warning:

```
Warning: libguestfs not available. Falling back to sudo mount.
[sudo] password for user:
```

## Provisioner Implementation Details

### SD Image Provisioner

For Raspberry Pi and similar—**rootless by default**:

```rust
impl Provisioner for SdImageProvisioner {
    fn check_dependencies(&self) -> Result<()> {
        // Prefer guestfish, fall back to sudo mount
        if which("guestfish").is_ok() {
            return Ok(());
        }
        if which("virt-copy-in").is_ok() {
            return Ok(());
        }
        // Fallback available but will need sudo
        warn!("libguestfs not found, will require sudo for mounting");
        Ok(())
    }

    fn inject_keys(&self, image: &Path, keys: &HostKeys, _: &Config) -> Result<PathBuf> {
        // Try rootless methods first
        if let Ok(()) = self.inject_with_guestfish(image, keys) {
            return Ok(image.to_path_buf());
        }

        if let Ok(()) = self.inject_with_virt_copy_in(image, keys) {
            return Ok(image.to_path_buf());
        }

        // Fallback to sudo mount (with warning)
        warn!("Rootless injection failed, falling back to sudo mount");
        self.inject_with_sudo_mount(image, keys)
    }

    fn inject_with_guestfish(&self, image: &Path, keys: &HostKeys) -> Result<()> {
        // guestfish --rw -a image.img -i copy-in keys /etc/ssh/
        run_cmd("guestfish", &[
            "--rw", "-a", image.to_str().unwrap(),
            "-i",  // Auto-mount filesystems
            "copy-in", keys.ssh_private_key.to_str().unwrap(), "/etc/ssh/",
            ":",
            "copy-in", keys.ssh_public_key.to_str().unwrap(), "/etc/ssh/",
            ":",
            "chmod", "0600", "/etc/ssh/ssh_host_ed25519_key",
            ":",
            "chmod", "0644", "/etc/ssh/ssh_host_ed25519_key.pub",
        ])
    }
}
```

### Proxmox VMA Provisioner

Uses `vma` for extraction, `guestfish` for injection:

```rust
impl Provisioner for ProxmoxVmaProvisioner {
    fn check_dependencies(&self) -> Result<()> {
        which("vma")?;       // Proxmox VMA tool
        which("zstd")?;      // Compression
        which("guestfish")?; // Rootless disk access
        Ok(())
    }

    fn inject_keys(&self, image: &Path, keys: &HostKeys, _: &Config) -> Result<PathBuf> {
        // ... (same as before: decompress, extract, guestfish inject, repackage)
    }

    fn validate_target(&self, target: &str) -> Result<TargetType> {
        // Proxmox expects a hostname, not a device or SSH user@host
        if target.starts_with("/dev/") {
            return Err(ProvisionerError::InvalidTarget(
                "proxmox-vma expects a hostname, not a device path".into()
            ));
        }
        if target.contains('@') {
            return Err(ProvisionerError::InvalidTarget(
                "proxmox-vma expects a hostname, not user@host".into()
            ));
        }
        Ok(TargetType::Hostname(target.to_string()))
    }

    fn deploy_to_target(
        &self,
        image: &Path,
        target: &str,
        hostname: &str,
        config: &Config,
    ) -> Result<()> {
        let vmid = &config.proxmox.vmid;  // From NixOS config
        let remote_path = format!("/tmp/{}.vma.zst", hostname);

        // 1. Copy image to Proxmox host
        run_cmd("rsync", &[
            "-v", "--progress",
            image.to_str().unwrap(),
            &format!("root@{}:{}", target, remote_path),
        ])?;

        // 2. Restore VM from backup
        run_ssh(target, &format!("qmrestore {} {}", remote_path, vmid))?;

        // 3. Start the VM
        run_ssh(target, &format!("qm start {}", vmid))?;

        // 4. Clean up remote image
        run_ssh(target, &format!("rm {}", remote_path))?;

        Ok(())
    }

    fn deployment_instructions(&self, output: &Path, hostname: &str) -> String {
        format!(r#"
To deploy manually:
  rsync -v --progress {} root@<proxmox-host>:/tmp/
  ssh root@<proxmox-host> 'qmrestore /tmp/{}.vma.zst <VMID>'
  ssh root@<proxmox-host> 'qm start <VMID>'
"#, output.display(), hostname)
    }
}
```

### Manual Provisioner

For unsupported targets or when user wants full control:

```rust
impl Provisioner for ManualProvisioner {
    fn inject_keys(&self, _: &Path, _: &HostKeys, _: &Config) -> Result<PathBuf> {
        // No-op: keys were already generated, user handles the rest
        Err(ProvisionerError::ManualRequired)
    }

    fn deployment_instructions(&self, _: &Path, hostname: &str) -> String {
        format!(r#"
Keys generated for {hostname}. Manual injection required.

Copy these files to the target's /etc/ssh/:
  ssh_host_ed25519_key
  ssh_host_ed25519_key.pub

Ensure correct permissions:
  chmod 600 /etc/ssh/ssh_host_ed25519_key
  chmod 644 /etc/ssh/ssh_host_ed25519_key.pub
"#, hostname = hostname)
    }
}
```

## Dependencies

### Core (Phase 1)
- `ssh-keygen` - Key generation (usually pre-installed)
- `ssh-to-age` - Age derivation

### Provisioner-Specific

| Provisioner | Primary (Rootless) | Fallback (Sudo) |
|-------------|-------------------|-----------------|
| `sd-image` | `guestfish` or `virt-copy-in` | `mount`, `losetup` |
| `raw` | `guestfish` or `virt-copy-in` | `mount`, `losetup` |
| `proxmox-vma` | `vma`, `zstd`, `guestfish` | — |
| `qcow2` | `guestfish` (native qcow2 support) | `qemu-nbd`, `mount` |
| `manual` | None | — |

### Nix-Bundled Dependencies

Since snowglobe is Nix-native, we can bundle all provisioner dependencies:

```nix
# In snowglobe's flake.nix
devShells.bootstrap = pkgs.mkShell {
  packages = with pkgs; [
    # Core
    ssh-to-age

    # Rootless image manipulation (preferred)
    libguestfs-with-appliance  # Includes guestfish, virt-copy-in
    mtools                      # FAT filesystem access
    fuse2fs                     # Userspace ext4

    # Format-specific
    zstd
    qemu                        # For qemu-nbd, FUSE exports

    # vma - would need to package from Proxmox sources
  ];
};
```

With `libguestfs` bundled, most provisioners work without sudo. Users run `nix develop .#bootstrap` and get everything they need.

For `vma`, options:
1. Package it from Proxmox sources (open source, Apache 2.0)
2. Provide a container-based fallback
3. Document running bootstrap on a Proxmox host directly

**Dependency checking**: Each provisioner implements `check_dependencies()` and prefers rootless tools. Missing rootless tools trigger a warning before falling back to sudo-based methods.

## Error Handling

```bash
$ snow bootstrap carbon

Error: Missing dependency for proxmox-vma provisioner

The 'vma' command is not available. This tool is part of Proxmox VE.

Options:
  1. Use the bundled environment: nix develop .#bootstrap
  2. Install on Proxmox host and run bootstrap there
  3. Switch to 'raw' provisioner: snow bootstrap carbon --provisioner raw

See: https://pve.proxmox.com/wiki/VMA
```

Rootless tool warnings:

```bash
$ snow bootstrap carbon

Warning: libguestfs not found. Will fall back to sudo mount.

Tip: For rootless operation, enter the bootstrap environment:
  nix develop .#bootstrap

Proceeding with sudo mount...
[sudo] password for user:
```

## Integration with Existing Features

### Progress Reporting

Bootstrap is a single-host, synchronous operation. Unlike `push` (which runs parallel pipelines across multiple hosts), bootstrap doesn't need the full observer pattern.

Simple progress output is sufficient:

```bash
$ snow bootstrap carbon

Bootstrapping carbon...
  → Generating SSH host key...
  ✓ Generated SSH host key
  → Building image (.#nixosConfigurations.carbon.config.system.build.sdImage)...
  ✓ Built image (2m 34s)
  → Injecting keys (sd-image provisioner)...
  ✓ Injected keys

Output: ./result/carbon.img
```

If we later need testability or alternative UIs, we can add an observer. For now, direct output keeps things simple.

### Hooks

```nix
{
  snowglobe.hooks.bootstrap = {
    pre = "echo 'Starting bootstrap for ${config.networking.hostName}'";
    post = "notify-send 'Bootstrap complete'";
  };
}
```

## Future Extensions

### Remote Bootstrap (High Priority)

The most exciting extension: boot a snowglobe installation disk and have it provision itself.

```bash
# Bootstrap via SSH to a machine booted from snowglobe installer
snow bootstrap carbon --target root@192.168.1.100
```

**Vision**: A custom NixOS installer ISO branded for snowglobe that:
1. Boots into a minimal environment with SSH enabled
2. Displays a nice branded splash screen with connection instructions
3. Waits for `snow bootstrap --target` to connect
4. Snowglobe handles everything: disk partitioning (via disko), NixOS installation, key injection

**Workflow**:
```
1. Build snowglobe installer: nix build .#installer-iso
2. Boot target machine from ISO (USB, PXE, etc.)
3. ISO shows: "Ready for bootstrap. Connect with: snow bootstrap <host> --target root@<ip>"
4. Run: snow bootstrap carbon --target root@192.168.1.100
5. Snowglobe:
   - Generates host keys
   - Runs disko to partition disks
   - Installs NixOS configuration from your flake
   - Injects SSH keys
   - Reboots into the new system
6. Host is ready for normal `snow push` operations
```

**Why this is powerful**:
- Zero-touch provisioning for bare metal
- Works for VMs too (boot from ISO)
- Consistent experience across all hardware
- No manual `nixos-install` or partition management

### Key Rotation

```bash
# Regenerate keys and update secrets for existing host
snow rotate-keys carbon --update-sops
```

This would connect to a running host, generate new keys, update SOPS, and re-push the configuration.

## Answered Questions

From the original proposal:

1. **Should SOPS integration be automatic or opt-in?**
   Opt-in via `--update-sops` flag. Key generation is separate from secrets management.

2. **How to handle hosts that don't use SOPS?**
   Keys are always generated. SOPS integration is optional. Keys work with any secrets manager that uses age.

3. **Should the command commit changes to git automatically?**
   No. User controls their git workflow.

4. **How to handle multi-disk configurations?**
   Use interactive prompting with [dialoguer](https://docs.rs/dialoguer) when the situation is ambiguous:

   ```bash
   $ snow bootstrap carbon

   Multiple disks found in image. Which contains the root filesystem?

   > sda (128GB) - NIXOS_ROOT label
     sdb (512GB) - DATA label
     sdc (256GB) - no label

   Selected: sda
   ```

   For non-interactive use (CI/scripts), allow explicit configuration in the NixOS module or CLI flag:
   ```bash
   snow bootstrap carbon --root-disk sda
   ```

5. **Should we support injecting additional files beyond SSH keys?**
   Future extension via `snowglobe.bootstrap.extraFiles` in NixOS config.

## Implementation Order

1. **Phase 1**: `snow bootstrap` with key generation only (MVP)
2. **Phase 2**: Provisioner trait + `manual` provisioner + image building
3. **Phase 3**: `sd-image` provisioner (Raspberry Pi, common hobbyist use case)
4. **Phase 4**: `raw` provisioner (generic disk images)
5. **Phase 5**: SOPS integration (`--update-sops`)
6. **Phase 6**: `proxmox-vma` provisioner
7. **Phase 7**: Deployment to target (`--target`)
8. **Phase 8**: Snowglobe installer ISO for remote bootstrap

Each phase should be broken into small, focused commits (trait definitions, implementations, tests separately). See project commit guidelines in CLAUDE.md.

## Testing Strategy

Following the project's backend trait pattern (see `NixBackend`, `DeployBackend`, `RebootBackend`), bootstrap functionality should be testable without external dependencies.

### Backend Traits for Testability

```rust
/// Abstraction for key generation operations
pub trait KeyGenBackend: Send + Sync {
    fn generate_ssh_keypair(&self, output_dir: &Path) -> Result<SshKeyPair>;
    fn derive_age_key(&self, ssh_public_key: &Path) -> Result<String>;
}

/// Abstraction for image manipulation operations
pub trait ImageBackend: Send + Sync {
    fn copy_file_into(&self, image: &Path, src: &Path, dest: &str) -> Result<()>;
    fn set_permissions(&self, image: &Path, path: &str, mode: u32) -> Result<()>;
}
```

Real implementations call `ssh-keygen`, `ssh-to-age`, `guestfish`, etc. Mock implementations allow testing provisioner logic without these tools installed.

### Test Coverage

- **Unit tests**: Key generation, age derivation, provisioner selection logic
- **Integration tests**: Full bootstrap flow using mock backends
- **Provisioner tests**: Each provisioner tested with fixture images (small test images)
- **Security tests**: Verify file permissions, temp directory cleanup, git warning detection

### Example Test Pattern

```rust
#[test]
fn test_sd_image_provisioner_injects_keys() {
    let mock_image_backend = MockImageBackend::new();
    let provisioner = SdImageProvisioner::new(mock_image_backend.clone());

    let keys = HostKeys { /* test fixture */ };
    provisioner.inject_keys(&test_image, &keys, &config)?;

    // Verify the right files were "injected"
    assert!(mock_image_backend.was_called_with("/etc/ssh/ssh_host_ed25519_key"));
    assert!(mock_image_backend.permissions_set("/etc/ssh/ssh_host_ed25519_key", 0o600));
}
```

## Deployment Target Design

The `--target` flag provides a unified interface for deployment across all provisioners:

```bash
# Deploy VMA to Proxmox host
snow bootstrap carbon --target proxmox01.local

# Flash SD image to local device
snow bootstrap carbon --target /dev/sdc

# Remote install via SSH to installer environment
snow bootstrap carbon --target root@192.168.1.100
```

### How It Works

1. **Single `--target` flag**: The provisioner interprets the target based on its deployment method
2. **Target type detection**:
   - Path starting with `/dev/` → block device
   - Contains `@` → SSH destination
   - Otherwise → hostname for provisioner-specific handling
3. **Extra parameters in NixOS config**: Provisioner-specific details (like Proxmox VMID) come from host configuration, keeping the CLI simple

```nix
{
  snowglobe.bootstrap = {
    provisioner = "proxmox-vma";
    proxmox = {
      vmid = 105;
      storage = "local-lvm";  # Optional, has sensible default
    };
  };
}
```

### Validation

Each provisioner validates whether the target makes sense:
- `proxmox-vma`: Expects a hostname (Proxmox API host)
- `sd-image`: Expects a block device path
- `remote`: Expects user@host for SSH
- `manual`: `--target` not supported (instructions only)

Invalid targets produce clear errors:
```bash
$ snow bootstrap carbon --target /dev/sdc
Error: proxmox-vma provisioner expects a hostname, not a device path.
Did you mean to use --provisioner sd-image?
```

## A Note on Examples

The code snippets and output examples in this document are **illustrative**, not prescriptive. They demonstrate the intended behavior and architecture but should not be copied verbatim into implementation. Developers should:

- Follow project patterns and conventions discovered during implementation
- Apply code review feedback that may refine the approach
- Use their judgment on implementation details not specified here

The proposal defines *what* the feature should do and *why*. The *how* has flexibility.

## Related Work

- [sops-nix](https://github.com/Mic92/sops-nix) - SOPS integration for NixOS
- [agenix](https://github.com/ryantm/agenix) - Age-based secrets for NixOS
- [nixos-anywhere](https://github.com/nix-community/nixos-anywhere) - Remote NixOS installation (different scope)
- [disko](https://github.com/nix-community/disko) - Declarative disk partitioning
