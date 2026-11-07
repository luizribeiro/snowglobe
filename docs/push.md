# Push Command

The `push` command is the primary deployment mechanism in Snowglobe. It builds NixOS and nix-darwin configurations and deploys them to remote hosts over SSH.

## Table of Contents

- [Basic Usage](#basic-usage)
  - [SSH connections](#ssh-connections)
- [Phases](#phases)
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

# Select hosts by phase, tag or a Nix expression
snow push --phase production
snow push --tags web,database
snow push -e 'config: config.services.nginx.enable or false'
```

snow finds `flake.nix` in the current directory or a parent. `-f <path>` or
the `FLAKE_PATH` environment variable points it elsewhere.

nix-darwin hosts always switch. Reboot policies and reboot detection don't
apply to them, they are never rebooted, and `--boot`, `--test` and
`--dry-run` fail on them. See [darwin.md](darwin.md).

### SSH connections

snow reaches each host at its FQDN (`networking.hostName` and
`networking.domain`) through your ssh config, so `Host` entries there apply.
A host whose sshd listens on another port can say so in its own config, and
snow uses that port for every connection, `nix copy` included:

```nix
snowglobe.push.sshPort = 2222;
```

snow never forwards your ssh agent to a host (`-o ForwardAgent=no`), so a
compromised host can't use it to reach the others.

---

## Phases

A host's phase comes from `snowglobe.push.phase` in its config.

```nix
snowglobe.push.phase = "p1";
```

- Phases run one after another, in name order with numbers by value: `p2`
  runs before `p10`.
- Hosts without a phase run last, together, after every named phase.
- Hosts in the phase `never` are left out unless named (`snow push curium`)
  or selected by phase (`--phase never`, or `--phases` with `never`). A bare
  `snow push`, `--tag`, `-e` or another `--phase` never selects them, even
  when they match. `snow build` and `snow plan` select hosts the same way;
  `snow list` shows every host.
- If any host in a phase fails, later phases are skipped and `snow push`
  exits non-zero, unless the host is [optional](#optional-hosts).
- `--ignore-phases` puts every selected host in a single phase.
- A phase name may contain only letters, digits, `-` and `_`, and can't be
  `default`, which snow keeps for hosts without a phase.

### Optional hosts

A host that may be offline, like a laptop, can be marked optional:

```nix
snowglobe.push.optional = true;
```

When an optional host fails, at any step, it is reported as failed and marked
`(optional)`, but its phase doesn't fail: later phases still run, and
`snow push` exits zero if only optional hosts failed. Its on-failure hooks
still run. It doesn't stop its serial group either, so the group's next host
may start while those hooks run.

```
! Phase p1 (1/2 failed in 0:41)
  ✓ boron (switch)
  ✗ curium (optional) at check: ssh: connect to host curium port 22: Connection timed out
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
- **`--concurrency`** is one pool of slots shared by steps 1–4 and the deploy
  section, so at most that many hosts are active at once. A host gives up its
  slot after planning and takes one again once its serial group lets it in, so
  a host waiting for its serial group holds no slot.
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
- A failed [optional host](#optional-hosts) doesn't stop its group.
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

Set `snowglobe.push.rebootPolicy` in the host's configuration:

```nix
{
  snowglobe.push.rebootPolicy = "auto";  # or "never" or "always"
}
```

### Available Policies

#### `auto` (Default)
Automatically detects if a reboot is required and reboots only when necessary.

**Behavior:**
- Runs reboot detection (checks kernel, initrd and critical services)
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

Reboot policies apply to auto mode and `--switch` (unless `--force` is
given). `--boot`, `--test` and `--dry-run` ignore them.

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
# hosts/db-prod.nix: never auto-reboot
{ snowglobe.push.rebootPolicy = "never"; }

# hosts/web-frontend.nix: stateless, always reboot
{ snowglobe.push.rebootPolicy = "always"; }

# hosts/app-server.nix: automatic detection (the default, so it can be omitted)
{ snowglobe.push.rebootPolicy = "auto"; }
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
   - Critical system services that would restart
3. **Chooses the appropriate mode:**
   - If reboot needed → uses `boot` mode + automatically reboots and waits
   - If no reboot needed → uses `switch` mode (immediate activation)

This is the **recommended default** for most deployments.

### Explicit Modes

You can override the automatic behavior with these flags:

#### `--switch` - Force Immediate Activation

Activates immediately. Under `rebootPolicy = "auto"`, snow still runs reboot
detection: if a reboot is required, the host fails at the plan step with the
reason, and nothing is activated. Hosts with `"never"` or `"always"` get boot
mode instead. `--force` switches in every case.

```bash
snow push myhost --switch
```

**Use when:**
- You know the changes are safe to activate immediately
- You're testing and want to see changes right away
- You're deploying to a development system

**Warning:** With `--force`, this can leave the system in an inconsistent state if the kernel, initrd or critical services changed.

#### `--boot` - Set for Next Boot

Sets the new configuration as the boot default without activating it, like
`nixos-rebuild boot`. The host keeps running its current system and boots the
new one at its next reboot; snow never reboots it. `--boot` skips reboot
detection and ignores the host's reboot policy, so even a host with
`rebootPolicy = "always"` is not rebooted.

```bash
snow push myhost --boot
```

**Use when:**
- You're batching updates for a maintenance window
- You'll reboot the host yourself, when it suits you

#### `--dry-run` - Preview Only

Builds and copies the system, then runs `dry-activate` on the host to show what would change. Nothing is activated, no hooks run, and there is no reboot detection.

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

In auto mode and with `--switch`, snow checks whether the new system needs a reboot, in two phases:

### What Requires a Reboot?

1. **Kernel upgrades** - The running kernel cannot be replaced
2. **Initrd changes** - Early boot components need a reboot
3. **Critical system services** - Services that are too fundamental to safely restart:
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
- Compares `/run/booted-system/{kernel,initrd}` with the new system's
- Very fast, no activation needed
- Catches most reboot-requiring changes

**Phase 2: Detailed Service Check**
- Skipped when phase 1 already found a reason to reboot
- Runs `dry-activate` (through `sudo snowglobe-activate` when the host has it)
  to see what services would change
- Checks if any critical services would be stopped or restarted, from the
  units it says it would stop or restart

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
# → Reports "✓ myhost (boot, rebooted)"
```

#### `--skip-reboot` - Inform Only

Detects reboot requirements but doesn't automatically reboot:

```bash
snow push myhost --skip-reboot
# → Detects kernel upgrade
# → Uses boot mode
# → Deploys configuration
# → Reports "✓ myhost (boot)"
# → User must manually reboot
```

**Use when:**
- You want to reboot during a specific maintenance window
- You're deploying to multiple hosts and want to reboot them all at once later
- You want to verify the deployment before rebooting

To reboot a host whatever changed, use `rebootPolicy = "always"`.

---

## Command Reference

### Activation Mode Flags (Mutually Exclusive)

| Flag | Description | Reboot Detection | Auto-reboot | Notes |
|------|-------------|------------------|-------------|-------|
| (none) | Auto mode - detect and choose | Yes | Yes (if boot) | **Recommended default** |
| `--switch` | Immediate activation | Yes | N/A | Blocked if reboot required (unless `--force`) |
| `--boot` | Set for next boot | No | No | Ignores reboot policy |
| `--dry-run` | Preview changes only | No | N/A | No hooks run |
| `--test` | Test activation (non-persistent) | No | N/A | |

### Reboot Control Flags

| Flag | Description |
|------|-------------|
| (none) | Reboot after an auto or `--switch` deploy that uses boot mode, when a reboot is required or the policy is `always` |
| `--skip-reboot` | Never reboot |
| `--reboot-timeout <SECONDS>` | How long to wait for a rebooted host to come back (default: 600) |

### Safety Flags

| Flag | Description |
|------|-------------|
| `--force` | Switch even when a reboot is required or the reboot policy says otherwise. Requires `--switch`; can't be combined with `--dry-run`, `--boot` or `--test` |

### Host Selection Flags

| Flag | Description |
|------|-------------|
| `<HOSTS>...` | Hosts to push (default: all hosts but those in the phase `never`) |
| `-e`, `--filter <EXPR>` | Hosts whose config matches a Nix expression, e.g. `config: config.services.nginx.enable or false` |
| `--phase <PHASE>` | Hosts in one phase |
| `--phases <A,B>` | Hosts in any of these phases |
| `--tag <TAG>` | Hosts with a tag (from `snowglobe.push.tags`) |
| `--tags <A,B>` | Hosts with any of these tags |

`--filter`, `--phase`, `--phases`, `--tag` and `--tags` are mutually
exclusive. Given with host names, they narrow those hosts down.

### Other Flags

| Flag | Description |
|------|-------------|
| `-f`, `--flake-path <PATH>` | Flake directory (also `FLAKE_PATH`; default: search upward from the current directory) |
| `--ignore-phases` | Push all selected hosts in one phase |
| `--quiet`, `-q` | Suppress subprocess output, only show summary |
| `--user <USER>` | SSH user to use for deployment |
| `--concurrency <N>` | Max hosts to deploy in parallel (default: 3) |
| `--redeploy` | Deploy hosts even if they already run the built system |

---

## Complete Interaction Matrix

| Command | Detection? | Activation | Auto-reboot? | Use Case |
|---------|-----------|------------|--------------|----------|
| `snow push myhost` | Yes | Auto (boot/switch) | Yes (if boot) | **Default**: Do the right thing automatically |
| `snow push myhost --skip-reboot` | Yes | Auto (boot/switch) | No | Deploy now, reboot manually later |
| `snow push myhost --switch` | Yes | Switch (if safe) or Error | N/A | Immediate activation (blocked if reboot required; policy `never`/`always` uses boot) |
| `snow push myhost --switch --force` | Yes (shows warning) | Switch | N/A | Force immediate activation despite reboot requirement or policy |
| `snow push myhost --boot` | No | Boot | No | Set for next boot, reboot manually later |
| `snow push myhost --dry-run` | No | None | N/A | Preview what would change |
| `snow push myhost --test` | No | Test | N/A | Test configuration (reverts on reboot) |

---

## Examples

Each phase ends with a report: one line per host, with the activation mode
and whether it rebooted or was unchanged, or the step it failed at. A push
of hosts without a phase is reported under `Push`. When those hosts run after
named phases, the overview lists them as `(no phase)` and their report is
titled `Hosts without a phase`.

### Normal deployment

```
$ snow push hydrogen oxygen
✓ Push (2 hosts in 1:12)
  ✓ hydrogen (switch)
  ✓ oxygen (boot, rebooted)
```

oxygen needed a reboot (for example a new kernel), so it was set to boot the
new system and rebooted.

### Manual reboot

With `snow push --skip-reboot`, oxygen would be reported as `(boot)` and keep
running the old system until you reboot it.

### Switch blocked by a reboot requirement

`snow push hydrogen --switch` fails hydrogen at the plan step when a reboot is
required, with the reason, such as `kernel linux-6.12.57 → linux-6.12.58` or
`audit.service would restart`. Nothing is activated. Drop `--switch` to let
snow pick boot mode and reboot, or add `--force` to switch anyway.

### Force switch despite a reboot need or policy

`snow push hydrogen --switch --force` activates the new system right away.
The old kernel keeps running until the next reboot.

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
- `--boot` - "I'll reboot during my maintenance window"
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
- Reboots happen in parallel (respecting `--concurrency` and serial groups)
- Each phase's report shows per-host status

### Inspired by Production Workflows

This design is based on real-world NixOS deployment workflows where:

- Kernel upgrades are common and must be handled gracefully
- Network service changes can be dangerous
- You want "push and forget" behavior for routine updates
- You still need manual control for special situations

---

## Related Documentation

- [hooks.md](hooks.md) - Lifecycle hooks
- [darwin.md](darwin.md) - Deploying to nix-darwin hosts
- [activation-helper.md](activation-helper.md) - `snowglobe-activate` and its security properties
