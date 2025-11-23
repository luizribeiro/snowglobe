# Push Command

The `push` command is the primary deployment mechanism in Snowglobe. It builds NixOS configurations and deploys them to remote hosts over SSH.

## Table of Contents

- [Basic Usage](#basic-usage)
- [Reboot Policies](#reboot-policies)
- [Activation Modes](#activation-modes)
- [Reboot Detection & Handling](#reboot-detection--handling)
- [Command Reference](#command-reference)
- [Examples](#examples)
- [Error Reporting](#error-reporting)
- [Design Philosophy](#design-philosophy)

---

## Basic Usage

```bash
# Deploy to specific hosts
snow push hydrogen oxygen

# Deploy to all hosts
snow push

# Deploy with custom SSH user
snow push hydrogen --user deploy

# Control parallelism
snow push --concurrency 5
```

---

## Reboot Policies

Snowglobe supports per-host reboot policies that can be configured in your NixOS configuration to control how deployments handle reboots. This allows you to define different reboot behaviors for different types of hosts (e.g., always reboot production hosts, never reboot development hosts).

### Configuring Reboot Policy

Set the `core.rebootPolicy` option in your NixOS configuration:

```nix
{
  # In your host configuration
  config.core.rebootPolicy = "auto";  # or "never" or "always"
}
```

### Available Policies

#### `auto` (Default)
Automatically detects if a reboot is required and reboots only when necessary.

**Behavior:**
- Runs reboot detection (checks kernel, initrd, init, and critical services)
- Uses `boot` mode + automatic reboot if changes require it
- Uses `switch` mode for immediate activation if safe

**Use for:** Most hosts, especially production servers with regular updates

#### `never`
Never automatically reboots, even if kernel or critical services changed.

**Behavior:**
- Always uses `boot` mode (sets config for next boot)
- Never triggers automatic reboot
- Requires manual reboot by administrator

**Use for:**
- Development/testing hosts where stability is more important than updates
- Hosts with special maintenance windows
- Systems that require coordinated reboots with external services

#### `always`
Always reboots after deployment, regardless of what changed.

**Behavior:**
- Always uses `boot` mode
- Always triggers automatic reboot after deployment
- Ensures the new configuration is running immediately

**Use for:**
- Production hosts requiring guaranteed activation
- Stateless systems that can reboot freely
- Hosts with critical security updates that must be applied immediately

### Policy Override Behavior

Reboot policies override command-line flags:

```bash
# Host has rebootPolicy = "never"
snow push myhost --switch
# → Will use boot mode (policy overrides --switch)

# Host has rebootPolicy = "always"
snow push myhost --switch
# → Will use boot mode and reboot (policy overrides --switch)

# Host has rebootPolicy = "auto"
snow push myhost --switch
# → Will use switch mode (respects --switch flag)
```

### Example Configuration

```nix
# In your flake or host configuration
{
  # Production database server - never auto-reboot
  hosts.db-prod = {
    config.core.rebootPolicy = "never";
  };

  # Stateless web server - always reboot to ensure fresh state
  hosts.web-frontend = {
    config.core.rebootPolicy = "always";
  };

  # Standard server - use automatic detection
  hosts.app-server = {
    config.core.rebootPolicy = "auto";  # or omit (auto is default)
  };
}
```

---

## Activation Modes

Snowglobe supports several activation modes that control how the new configuration is applied on the target host. These modes correspond to the standard NixOS activation modes.

### Default: Auto Mode

**When you run:**
```bash
snow push myhost
```

**What happens:**
1. Builds the configuration
2. **Detects if a reboot is required** by checking:
   - Kernel changes
   - Initrd changes
   - Init (systemd) changes
   - Critical system services that would restart
3. **Chooses the appropriate mode:**
   - If reboot needed → uses `boot` mode + automatically reboots and waits
   - If no reboot needed → uses `switch` mode (immediate activation)

This is the **recommended default** for most deployments.

### Explicit Modes

You can override the automatic behavior with these flags:

#### `--switch` - Force Immediate Activation

Forces immediate activation regardless of reboot detection.

```bash
snow push myhost --switch
```

**Use when:**
- You know the changes are safe to activate immediately
- You're testing and want to see changes right away
- You're deploying to a development system

**Warning:** This can leave the system in an inconsistent state if kernel/initrd/critical services changed.

#### `--boot` - Set for Next Boot

Sets the new configuration as the boot default but doesn't activate it immediately.

```bash
snow push myhost --boot
```

**Use when:**
- You want to deploy now but reboot manually later
- You're batching updates during a maintenance window
- You want to review changes before activating

#### `--dry-run` - Preview Only

Shows what would change without actually deploying or activating anything.

```bash
snow push myhost --dry-run
```

**Use when:**
- You want to preview changes before deploying
- You're testing your configuration
- You want to see which services would restart

#### `--test` - Test Activation

Activates the new configuration but doesn't make it the boot default.

```bash
snow push myhost --test
```

**Use when:**
- You want to test the new configuration without committing to it
- The system will revert on next reboot

### System Profile Generations

Snowglobe automatically manages NixOS system profile generations during deployments. This ensures that successful deployments create visible generations that appear in `nixos-rebuild list-generations` and support system rollbacks.

#### Which Modes Create Generations?

**Persistent Modes (create generations):**
- `switch` - Creates a generation and activates immediately
- `boot` - Creates a generation for next boot

**Ephemeral Modes (skip generations):**
- `test` - Temporary activation, no generation created
- `dry-activate` - Preview only, no changes made

#### Verifying Generations

After a successful deployment with `--switch` or `--boot`:

```bash
# On the target host
nixos-rebuild list-generations

# Output shows new generation:
# Generation 142 (current): Mon Nov 23 12:00:00 2025
```

#### Why Ephemeral Modes Skip Generations

Test and dry-activate modes are designed for experimentation and validation. Creating generations for these operations would pollute the generation list and make rollback history harder to navigate. These modes intentionally skip profile updates to keep your generation list clean and meaningful.

---

## Reboot Detection & Handling

Snowglobe automatically detects when a deployment requires a reboot using a sophisticated two-phase detection system:

### What Requires a Reboot?

1. **Kernel upgrades** - The running kernel cannot be replaced
2. **Initrd changes** - Early boot components need a reboot
3. **Init (systemd) changes** - PID 1 cannot restart itself
4. **Critical system services** - Services that are too fundamental to safely restart:
   - `audit.service`
   - `kmod-static-nodes.service`
   - `network-local-commands.service`
   - `network-setup.service`
   - `syslog.socket`
   - `systemd-modules-load.service`
   - `systemd-oomd.socket`
   - `systemd-sysctl.service`
   - `systemd-udevd-control.socket`
   - `systemd-udevd-kernel.socket`

### How Detection Works

**Phase 1: Fast Symlink Check**
- Compares `/run/booted-system/{kernel,initrd,init}` with new system paths
- Very fast, no activation needed
- Catches most reboot-requiring changes

**Phase 2: Detailed Service Check**
- Runs `dry-activate` to see what services would change
- Reads machine-readable output from `/run/nixos/dry-activation-restart-list`
- Checks if any critical services would restart

### Controlling Reboot Behavior

#### Default: Automatic Reboot

By default, if a reboot is detected as necessary:

```bash
snow push myhost
# → Detects kernel upgrade
# → Uses boot mode
# → Deploys configuration
# → Runs `sudo systemctl reboot` via SSH
# → Waits for host to come back online
# → Shows "Deployed & Rebooted" in results
```

#### `--skip-reboot` - Inform Only

Detects reboot requirements but doesn't automatically reboot:

```bash
snow push myhost --skip-reboot
# → Detects kernel upgrade
# → Uses boot mode
# → Deploys configuration
# → Shows "Reboot Required: Yes (kernel changed)"
# → User must manually reboot
```

**Use when:**
- You want to reboot during a specific maintenance window
- You're deploying to multiple hosts and want to reboot them all at once later
- You want to verify the deployment before rebooting

#### `--reboot` - Force Reboot

Forces a reboot even if detection says it's not needed:

```bash
snow push myhost --reboot
# → May detect no reboot needed
# → Uses boot mode anyway
# → Deploys and reboots
```

**Use when:**
- You know a reboot is needed for other reasons
- You want to ensure a clean state

---

## Command Reference

### Activation Mode Flags (Mutually Exclusive)

| Flag | Description | Reboot Detection | Auto-reboot | Notes |
|------|-------------|------------------|-------------|-------|
| (none) | Auto mode - detect and choose | Yes | Yes (if boot) | **Recommended default** |
| `--switch` | Force immediate activation | Yes | N/A | Blocked if reboot required (unless `--force`) |
| `--boot` | Set for next boot only | No | Yes* | |
| `--dry-run` | Preview changes only | Yes | N/A | |
| `--test` | Test activation (non-persistent) | No | N/A | |

\* Can be controlled with `--no-reboot`

### Reboot Control Flags

| Flag | Description | When it applies |
|------|-------------|-----------------|
| (none) | Auto-reboot if boot mode used | When boot mode is chosen |
| `--reboot` | Force reboot after boot deploy | When boot mode is used |
| `--skip-reboot` | Never auto-reboot | Always |

### Safety Flags

| Flag | Description |
|------|-------------|
| `--force` | Override policies and safety checks (use with `--switch` to ignore reboot policies and detection) |

### Other Flags

| Flag | Description |
|------|-------------|
| `--quiet`, `-q` | Suppress subprocess output, only show summary |
| `--user <USER>` | SSH user to use for deployment |
| `--concurrency <N>` | Max hosts to deploy in parallel (default: 3) |

---

## Complete Interaction Matrix

| Command | Detection? | Activation | Auto-reboot? | Use Case |
|---------|-----------|------------|--------------|----------|
| `snow push myhost` | Yes | Auto (boot/switch) | Yes (if boot) | **Default**: Do the right thing automatically |
| `snow push myhost --skip-reboot` | Yes | Auto (boot/switch) | No | Deploy now, reboot manually later |
| `snow push myhost --switch` | Yes | Switch (if safe) or Error | N/A | Force immediate activation (blocked if reboot required or policy) |
| `snow push myhost --switch --force` | Yes (shows warning) | Switch | N/A | Force immediate activation despite reboot requirement or policy |
| `snow push myhost --boot` | No | Boot | Yes | Set for next boot and reboot now |
| `snow push myhost --boot --skip-reboot` | No | Boot | No | Set for next boot, reboot manually later |
| `snow push myhost --boot --reboot` | No | Boot | Yes | Same as `--boot` (redundant) |
| `snow push myhost --dry-run` | Yes | None | N/A | Preview what would change |
| `snow push myhost --test` | No | Test | N/A | Test configuration (reverts on reboot) |

---

## Examples

### Example 1: Normal Deployment

```bash
$ snow push hydrogen

→ Building configurations...
✓ Built 1 configuration(s)

→ Deploying...
activating the configuration...
setting up /etc...
reloading the following units: dbus.service

Push Results

+-----------+--------+--------+-----------------------+
| Host      | Status | Reboot | Message               |
+-----------+--------+--------+-----------------------+
| hydrogen  | ✓      | No     | Deployed successfully |
+-----------+--------+--------+-----------------------+

1/1 hosts deployed successfully
```

### Example 2: Kernel Upgrade (Auto-reboot)

```bash
$ snow push hydrogen

→ Building configurations...
✓ Built 1 configuration(s)

→ Deploying...
activating the configuration...
setting up /etc...

Rebooting hydrogen...
Waiting for hydrogen to come back online...
hydrogen is back online

Push Results

+-----------+--------+-----------------------+----------------------------+
| Host      | Status | Reboot                | Message                    |
+-----------+--------+-----------------------+----------------------------+
| hydrogen  | ✓      | Yes (kernel changed)  | Deployed & Rebooted        |
+-----------+--------+-----------------------+----------------------------+

1/1 hosts deployed successfully
```

### Example 3: Kernel Upgrade (Manual Reboot)

```bash
$ snow push hydrogen --skip-reboot

→ Building configurations...
✓ Built 1 configuration(s)

→ Deploying...
activating the configuration...
setting up /etc...

Push Results

+-----------+--------+-----------------------+----------------------------+
| Host      | Status | Reboot                | Message                    |
+-----------+--------+-----------------------+----------------------------+
| hydrogen  | ✓      | Yes (kernel changed)  | Set for next boot          |
+-----------+--------+-----------------------+----------------------------+

1/1 hosts deployed successfully

⚠  The following hosts require a manual reboot:
  - hydrogen (kernel changed)
```

### Example 4: Multi-Host with Mixed Requirements

```bash
$ snow push hydrogen oxygen nitrogen

→ Building configurations...
✓ Built 3 configuration(s)

→ Deploying...
[parallel deployment output...]

Push Results

+-----------+--------+------------------------------------+------------------------+
| Host      | Status | Reboot                             | Message                |
+-----------+--------+------------------------------------+------------------------+
| hydrogen  | ✓      | Yes (kernel changed)               | Deployed & Rebooted    |
| oxygen    | ✓      | No                                 | Deployed successfully  |
| nitrogen  | ✓      | Yes (critical service: audit.ser…) | Deployed & Rebooted    |
+-----------+--------+------------------------------------+------------------------+

3/3 hosts deployed successfully
```

### Example 5: Dry Run to Preview

```bash
$ snow push hydrogen --dry-run

→ Building configurations...
✓ Built 1 configuration(s)

→ Deploying...
would activate the configuration...
would restart the following units: postgresql.service nginx.service
would start the following units: grafana.service

Push Results

+-----------+--------+--------+--------------------+
| Host      | Status | Reboot | Message            |
+-----------+--------+--------+--------------------+
| hydrogen  | ✓      | No     | Dry run successful |
+-----------+--------+--------+--------------------+

1/1 hosts deployed successfully
```

### Example 6: Switch Blocked by Reboot Requirement

```bash
$ snow push hydrogen --switch

→ Building configurations...
✓ Built 1 configuration(s)

Error: Cannot use --switch when reboot is required

Reboot required because: kernel changed

The --switch flag forces immediate activation, which can leave the system in an
inconsistent state when kernel/initrd/init changes occur or critical services restart.

Options:
  1. Remove --switch to use automatic mode (will use boot mode + reboot)
  2. Use --switch --force to force immediate activation anyway (not recommended)
  3. Use --boot to set for next boot without immediate activation
```

### Example 7: Force Switch Despite Reboot Need or Policy

```bash
$ snow push hydrogen --switch --force

→ Building configurations...
✓ Built 1 configuration(s)

⚠  Warning: Forcing immediate activation despite reboot requirement (kernel changed)
   This may leave the system in an inconsistent state.

→ Deploying...
activating the configuration...
setting up /etc...
restarting the following units: systemd-udevd.service

Push Results

+-----------+--------+--------+-----------------------+
| Host      | Status | Reboot | Message               |
+-----------+--------+--------+-----------------------+
| hydrogen  | ✓      | N/A    | Deployed successfully |
+-----------+--------+--------+-----------------------+

1/1 hosts deployed successfully

⚠  Warning: System may be in an inconsistent state
   - New configuration active but old kernel still running
   - Consider rebooting soon
```

**Note**: `--force` also overrides reboot policies. If a host has `rebootPolicy = "always"` or `"never"`, using `--switch --force` will use switch mode anyway.

---

## Error Reporting

Snowglobe provides detailed error messages to help diagnose deployment failures quickly, especially for copy and activation failures.

### Error Message Format

When a deployment step fails, the error includes:

- **Which host failed** - The hostname where the failure occurred
- **Which step failed** - Build, copy, activate, or reboot
- **The command that failed** - The exact command that was executed
- **Exit code** - The exit code returned by the failed command
- **Full stderr output** - Complete error output from the command
- **Full stdout output** - Standard output (if relevant)

### Example: SSH Connection Failure

```bash
$ snow push krypton

Phase p1 (1 hosts): ✗ 1/1 failed
  ✗ krypton - failed at copy: Command failed on host 'krypton'

Errors:
  krypton (copy):
    Command failed on host 'krypton'

    Command: nix copy --to ssh://krypton.i.thepromisedlan.club /nix/store/...
    Exit code: 1

    Stderr:
    building SSH tunnel...
    error: failed to connect to 'krypton.i.thepromisedlan.club': Host key verification failed.
    ssh: Could not resolve hostname krypton.i.thepromisedlan.club: Name or service not known
```

This makes it immediately clear that the issue is an SSH host key verification failure, allowing you to quickly add the host key to your known_hosts or `secrets/pubkeys/hosts/` directory.

### Example: Activation Failure

```bash
$ snow push hydrogen

Phase p1 (1 hosts): ✗ 1/1 failed
  ✗ hydrogen - failed at activate: Command failed on host 'hydrogen'

Errors:
  hydrogen (activate):
    Command failed on host 'hydrogen'

    Command: ssh root@hydrogen.example.com sudo /nix/store/.../bin/switch-to-configuration switch
    Exit code: 1

    Stderr:
    activating the configuration...
    setting up /etc...
    reloading the following units: dbus.service
    restarting the following units: postgresql.service

    Job for postgresql.service failed because the control process exited with error code.
    See "systemctl status postgresql.service" and "journalctl -xe" for details.
```

### Verbose Mode

Use the `RUST_LOG` environment variable to enable debug-level logging for even more detailed output:

```bash
$ RUST_LOG=debug snow push hydrogen

# Shows:
# - Full command execution details
# - Nix evaluation traces
# - SSH connection details
# - Timing information
# - All tracing events at debug level
```

You can also set `RUST_LOG=trace` for maximum verbosity, or target specific modules like `RUST_LOG=snowlib::push=debug`.

### Error Truncation

For very long error messages (>30 lines), the error output is truncated with an indicator:

```
Stderr:
[first 30 lines of error output]
... (15 more lines omitted)
```

You can use `RUST_LOG=debug` to see full tracing output during execution.

---

## Design Philosophy

### Smart Defaults

The default behavior (`snow push`) is designed to **do the right thing automatically**:

- **Detects** when a reboot is actually necessary
- **Chooses** the appropriate activation mode
- **Handles** reboots automatically
- **Waits** for hosts to come back online
- **Scales** to multi-host deployments

This matches how production deployments actually work: you want the tool to handle the complexity and edge cases, not require manual decision-making for every deployment.

### Explicit Control When Needed

Power users have full control via explicit flags:

- `--switch` - "I know this is safe to activate immediately"
- `--switch --force` - "Override policies and force immediate activation"
- `--boot --skip-reboot` - "I'll reboot during my maintenance window"
- `--dry-run` - "Let me see what would happen first"

### Safe by Default

- Reboot detection prevents inconsistent system states
- Automatic reboot ensures changes take effect properly
- Clear communication about what's happening
- Escape hatches for edge cases

### Multi-Host Aware

When deploying to multiple hosts:

- Hosts can have different activation modes in the same deployment
- Some may need reboot (boot mode), others may not (switch mode)
- Reboots happen in parallel (respecting `--concurrency`)
- Clear table output shows per-host status

### Inspired by Production Workflows

This design is based on real-world NixOS deployment workflows where:

- Kernel upgrades are common and must be handled gracefully
- Network service changes can be dangerous
- You want "push and forget" behavior for routine updates
- You still need manual control for special situations

---

## Related Documentation

- [Reboot Detection System](wip/reboot-detection.md) - Detailed design of the reboot detection system
- [SSH Deployment](TODO) - How SSH-based deployment works
- [Configuration Guide](TODO) - Configuring deployment behavior in your flake
