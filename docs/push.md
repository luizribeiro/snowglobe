# Push Command

The `push` command is the primary deployment mechanism in Snowglobe. It builds NixOS configurations and deploys them to remote hosts over SSH.

## Table of Contents

- [Basic Usage](#basic-usage)
- [Host Pipeline](#host-pipeline)
- [Serial Groups](#serial-groups)
- [Lifecycle Hooks](#lifecycle-hooks)
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

## Host Pipeline

Phases run one after another. Within a phase, every host runs this pipeline
independently:

```
1. build
2. unchanged? ── yes ──▶ done, reported as "unchanged"
3. copy
4. plan
   ┌─ deploy section (one host at a time per serial group) ─────────┐
5. │ pre-deploy hooks                                               │
6. │ activate                                                       │
7. │ reboot (if needed)                                             │
8. │ post-reboot hooks (only after a reboot)                        │
9. │ post-deploy hooks                                              │
   └────────────────────────────────────────────────────────────────┘

A failure at any step stops the host and runs its on-failure hooks.
```

- **Before any host starts**, snow loads every host's metadata, including its
  hooks: it builds each host's hooks file on the deploying machine, which also
  fetches or builds the packages local hooks reference (see [Packages in Hook
  Commands](hooks.md#packages-in-hook-commands)). If that fails for a host,
  the push stops before anything is deployed.
- **`--concurrency`** limits how many hosts are in steps 1–4 at once, and,
  separately, how many are inside a deploy section at once. A host waiting for
  its serial group holds no concurrency slot.
- **Unchanged hosts** are skipped right after the build. A host is unchanged
  when both `/run/current-system` (what it runs) and
  `/nix/var/nix/profiles/system` (what it boots next) already point at the
  built system. Nix store paths hash every input of the system, so any
  configuration change produces a different path. Unchanged wins over
  `rebootPolicy = "always"`: such hosts are not drained or rebooted. If those
  paths can't be read, the host is deployed (with a warning) rather than
  skipped.
- **`--redeploy`** deploys selected hosts even when unchanged. Use it to retry
  a host whose last activation partly failed: NixOS points
  `/run/current-system` at the new system before starting services, so such a
  host otherwise reads as unchanged. It is a full deploy: hooks run, and hosts
  with `rebootPolicy = "always"` reboot.
- **Planning happens before any hook**, so a host blocked by planning (for
  example `--switch` when a reboot is required) is never drained.
- **post-deploy** runs last, once the host runs the new system, whether or not
  it rebooted, and only if everything before it succeeded.

---

## Serial Groups

Hosts that share a serial group deploy their disruptive part one at a time
within a phase, even if the phase contains several of them. This keeps, for
example, two Kubernetes storage nodes from being drained and rebooted at the
same time.

```nix
snowglobe.push.serialGroup = "kubernetes";
```

- **What is serialized:** the deploy section (steps 5–9 in the diagram
  above). The next host starts only after the previous one finished all of it,
  so a post-deploy health gate (e.g. "storage volumes healthy") decides when
  the next node may start.
- **What stays parallel:** build, copy and planning. Hosts with nothing to
  deploy never wait. Hosts in other groups, or without a group, are not
  affected (beyond the shared `--concurrency` limit).
- **Order:** hosts go in the order they finish planning. It is neither
  alphabetical nor configurable. A waiting host is logged as `waiting for
  serial group 'kubernetes' (held by praseodymium)`.
- **Activation modes:** `--test` and `--boot` are serialized the same way.
  `--dry-run` is serialized too, but runs no hooks at all.

### When a host in a serial group fails

- The group stops before the failed host's on-failure hooks run, so no other
  member starts while they run. This covers a failure at any step, including
  build, copy, planning, activation, reboot or a hook.
- Group members that haven't started are reported as failed with
  `serial group 'kubernetes' halted after praseodymium failed`. They
  run no hooks, not even on-failure hooks, since nothing touched them.
- Hosts outside the group carry on. The phase fails, later phases are skipped,
  and `snow push` exits non-zero.
- **Limitation:** only hosts pushed by the same `snow push` invocation
  coordinate. Two concurrent `snow push` runs don't know about each other's
  groups.

---

## Lifecycle Hooks

Lifecycle hooks allow you to execute custom commands at specific points in the deployment pipeline, enabling sophisticated orchestration like draining Kubernetes nodes, coordinating with service discovery systems, or performing custom validation.

### Overview

Hooks are configured in your NixOS configuration and execute automatically during deployment at four stages:

- **`pre-deploy`**: Before activation, only for hosts that change (e.g., drain cluster nodes)
- **`post-reboot`**: After host reboots (e.g., wait for the node to be Ready)
- **`post-deploy`**: Last, once the host runs the new system, with or without a reboot (e.g., uncordon nodes, health gate)
- **`on-failure`**: When deployment fails (e.g., cleanup, alerts)

### Quick Example

```nix
{
  config.snowglobe.hooks = {
    pre-deploy = [
      {
        location = "local";  # Run on machine executing snow
        command = "kubectl drain ${config.networking.hostName}";
        timeout = 300;
        continueOnFailure = false;
      }
    ];
    post-deploy = [
      {
        location = "local";
        command = "kubectl uncordon ${config.networking.hostName}";
        timeout = 60;
        continueOnFailure = true;  # Always uncordon
      }
    ];
  };
}
```

### Documentation

For comprehensive documentation including:
- All hook stages and when they execute
- Local vs remote execution
- Error handling and `continueOnFailure`
- Environment variables available to hooks
- Complete examples (Kubernetes, Consul, load balancers, etc.)
- Best practices and troubleshooting

See **[docs/hooks.md](hooks.md)** for full details.

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
Always reboots when the host is deployed, regardless of what changed. Hosts
that are already running the built system are skipped and not rebooted (see
[Host Pipeline](#host-pipeline)).

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
# → Runs `sudo snowglobe-activate reboot` via SSH (or `sudo systemctl reboot`
#   on hosts without the snowglobe module)
# → Waits until the host answers with a new boot ID
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
| `--reboot-timeout <SECONDS>` | How long to wait for a rebooted host to come back (default: 600) |
| `--redeploy` | Deploy hosts even if they already run the built system |

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

✗ Cannot use --switch when reboot is required

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

When a phase ends, its report lists every host. A failed host's line names
the step it failed at and the cause: the one line of the error that says what
went wrong, cut to the terminal width. Under it come up to ten lines leading up
to the cause, taken from the failed command's stderr, and where to read the
rest.

### Example: Activation Failure

```
✗ Phase p1 (1/1 failed in 0:41)
  ✗ hydrogen at activate: Job for postgresql.service failed because the control process exited with error code.
    activating the configuration...
    setting up /etc...
    reloading the following units: dbus.service
    restarting the following units: postgresql.service
    Job for postgresql.service failed because the control process exited with error code.
    … 7 more lines in /tmp/snow-1762255380.log
```

The context ends at the cause again, at full contrast, with the lines before
it dim. A command's stdout is left out: for an activation it is mostly service
logs.

### Where the full error is

Run in a terminal, snow writes its log to `/tmp/snow-<timestamp>.log` and
prints the path when it starts. The log has every failure in full: the
command, its exit code, its stderr and its stdout. When snow's stderr isn't a
terminal, the log goes to stderr instead.

Use the `RUST_LOG` environment variable for more detail, for example
`RUST_LOG=debug snow push hydrogen`, `RUST_LOG=trace`, or a single module with
`RUST_LOG=snowlib::push=debug`.

---

## Design Philosophy

### Smart Defaults

The default behavior (`snow push`) is designed to **do the right thing automatically**:

- **Detects** when a reboot is actually necessary
- **Chooses** the appropriate activation mode
- **Handles** reboots automatically
- **Waits** for hosts to come back online (up to 10 minutes; `--reboot-timeout` changes it)
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
