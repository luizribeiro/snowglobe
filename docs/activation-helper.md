# System Activation

This document describes snowglobe's activation system, which provides passwordless deployment for both NixOS and nix-darwin hosts.

## Overview

The `snowglobe-activate` command is a small shell script that acts as a stable entry point for system activation. It solves a fundamental challenge with push-based NixOS/Darwin deployments: the activation binary path changes with every system update because it contains the Nix store hash.

### The Problem

Traditional NixOS/Darwin deployment requires running activation commands like:

```bash
sudo /nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-darwin-system-25.05/activate switch
```

The hash (`qqz7czg0f46pg7n3f0xqmiv14bampzr7`) changes with every configuration update, making it impossible to create stable sudoers rules. Wildcard patterns like `/nix/store/*/activate` don't work reliably, especially on macOS.

### The Solution

Snowglobe provides an activation command installed at a stable path that:

1. Accepts the store path and activation mode as arguments
2. Validates the store path and activation mode
3. Sets the system profile and runs the system's activation script
4. Has a stable path for sudoers configuration

The NixOS and nix-darwin modules each install their own version of the script.

**Automatic detection**: snow checks whether `snowglobe-activate` is on the target host and uses it when present. Without it, snow falls back to direct activation with a warning. The fallback works on NixOS only, and needs passwordless sudo for `nix-env` and for `<system>/bin/switch-to-configuration`. On Darwin, the module is required.

## Installation

### NixOS

Add to your NixOS configuration:

```nix
{
  # Add snowglobe to your flake inputs
  inputs.snowglobe.url = "github:luizribeiro/snowglobe";

  # In your system configuration:
  imports = [ inputs.snowglobe.nixosModules.default ];

  # Configure passwordless deployment users
  snowglobe.push.allowedUsers = [ "deploy" ];
}
```

Rebuild your system:

```bash
sudo nixos-rebuild switch
```

### nix-darwin

Add to your Darwin configuration:

```nix
{
  # Add snowglobe to your flake inputs
  inputs.snowglobe.url = "github:luizribeiro/snowglobe";

  # In your Darwin configuration:
  imports = [ inputs.snowglobe.darwinModules.default ];

  # Configure passwordless deployment users
  snowglobe.push.allowedUsers = [ "deploy" ];
}
```

Rebuild your system:

```bash
sudo darwin-rebuild switch --flake .#your-darwin-host
```

## Configuration Options

### `snowglobe.push.allowedUsers`

Type: `list of strings`
Default: `[]`
Example: `[ "deploy" ]`

List of users who can run `snowglobe-activate` without a password, and so activate any system from the Nix store. On NixOS, they can also reboot the host with it.

**Users in this list are effectively root on the host.** See [Security](#security).

## Usage

### With snowglobe (automatic)

When using `snow push`, snowglobe automatically:

1. Detects if `snowglobe-activate` is available on the target host
2. Uses it if present, to activate and, on NixOS, to reboot
3. Falls back to direct activation if not found, with a warning (NixOS only, see above)

```bash
# Automatically uses snowglobe-activate if available
snow push hostname
```

### Manual usage

The helper can be called directly:

```bash
# Activate a system in switch mode
sudo snowglobe-activate /nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-nixos-system-25.05 switch

# Set for next boot only
sudo snowglobe-activate /nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-nixos-system-25.05 boot

# Test activation (non-permanent)
sudo snowglobe-activate /nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-nixos-system-25.05 test

# Dry run (show what would change)
sudo snowglobe-activate /nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-nixos-system-25.05 dry-activate

# Reboot the host (NixOS only)
sudo snowglobe-activate reboot
```

On Darwin, only `switch` works.

## How It Works

### Input Validation

The helper validates inputs before execution:

1. **Store path validation**: Must be a single top-level store path, `/nix/store/<hash>-<name>`, so `/run/current-system` or `/nix/store/../…` are rejected
2. **Mode validation**: Must be one of: `switch`, `boot`, `test`, `dry-activate` (only `switch` on Darwin)
3. **Activation script existence**: Verifies the system has `bin/switch-to-configuration` (NixOS) or `activate` (Darwin)

After validation, the helper sets the system profile (`nix-env --profile /nix/var/nix/profiles/system --set <path>`), then runs the activation script. snow skips its own profile update when the helper is present.

### Platform-Specific Behavior

**NixOS**:
- Uses `/nix/store/<hash>/bin/switch-to-configuration`
- Supports all four activation modes: switch, boot, test, dry-activate
- Sets the system profile for `switch` and `boot` only
- Passes mode argument directly to the activation script
- `snowglobe-activate reboot` runs `systemctl reboot`

**nix-darwin**:
- Uses `/nix/store/<hash>/activate`
- Only supports `switch` mode (Darwin's activate script doesn't support modes)
- Returns explicit errors for unsupported modes (boot, test, dry-activate)
- Always sets the system profile, since Darwin's activate doesn't record a generation
- Has no `reboot` subcommand

### Sudoers Configuration

The module automatically configures sudo to allow passwordless execution using a stable path.

**NixOS** (through `security.sudo.extraRules`, which runs as any user by default):
```
user ALL=(ALL:ALL) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate
```

**Darwin** (through `security.sudo.extraConfig`, written to `/etc/sudoers.d/10-nix-darwin-extra-config`):
```
user ALL=(root) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate
```

This uses `/run/current-system/sw/bin/` which is a stable path that always points to the currently active system's binaries. This solves the chicken-and-egg problem: when pushing a new system, the helper from the **current** system is used to activate the **new** system. After activation completes, `/run/current-system` is updated to point to the new system.

## Security

### Allowed users are root-equivalent

Activating a system runs its activation script as root, and the store path is chosen by the caller. Writing to `/nix/store` directly needs root, but adding to it does not: any user allowed to use the Nix daemon can build or add a path whose `bin/switch-to-configuration` (or `activate` on Darwin) does whatever they want. So a user in `allowedUsers` can run arbitrary code as root.

This is inherent to push deployment. deploy-rs, colmena and `nixos-rebuild --target-host` all need a root-equivalent user on the target too.

The input validation catches mistakes, such as a wrong mode or a path that isn't a system. It is not a security boundary against an allowed user.

### What the helper does offer

- A single, auditable command in sudoers instead of `NOPASSWD: ALL`
- A stable path that needs no sudoers changes when the system changes, and works around sudoers wildcard limitations on macOS

### Comparison to Alternatives

All of these give the allowed user root-equivalent access. They differ in what sudoers exposes and how well they work.

| Approach | Sudo surface | Reliability | Ease of Use |
|----------|--------------|-------------|-------------|
| snowglobe-activate helper | One fixed command | ✓ Excellent | ✓ Simple |
| `/nix/store/*/activate` wildcard | Any matching store path | ✗ Unreliable on macOS | ✓ Simple |
| `NOPASSWD: ALL` | Every command | ✓ Always works | ✓ Very simple |
| Per-generation sudoers | One path per generation | ⚠️ Requires automation | ✗ Complex |

A possible future improvement is to only activate systems signed with a deploy key, which would make the helper a real boundary.

### Best Practices

1. **Treat the deploy user's SSH key like a root key**: key-only login, and keep the key only on the machine or CI runner that deploys
2. **Use a dedicated deploy user, and only that one**: a personal account in `allowedUsers` is root-equivalent without a password
3. **Audit activations**: Check system logs for unexpected activation attempts

## Troubleshooting

### Warning: "snowglobe-activate not found on <host>, falling back to direct activation"

**Cause**: The snowglobe module isn't installed on the target host yet.

**Solution**: To use the helper:
1. Add the snowglobe module to your configuration: `imports = [ inputs.snowglobe.nixosModules.default ];` (or `darwinModules.default` on nix-darwin)
2. Add the user snowglobe connects as to `snowglobe.push.allowedUsers`, ideally a dedicated deploy user (see [Security](#security))
3. Rebuild the system: `sudo nixos-rebuild switch` or `sudo darwin-rebuild switch`

On subsequent deployments, snowglobe will automatically use `snowglobe-activate`.

### Error: "Invalid store path '…' (must be /nix/store/<hash>-<name>)"

**Cause**: The first argument isn't a single top-level store path. Symlinks like `/run/current-system`, paths inside a store path, and paths with `..` are all rejected.

**Solution**: Pass the system's resolved store path, e.g. `"$(readlink -f /run/current-system)"`.

### Error: "Activation script not found"

**Cause**: The store path doesn't contain an activation script.

**Solution**: Verify the store path points to a valid system configuration:
- NixOS: Check for `<path>/bin/switch-to-configuration`
- Darwin: Check for `<path>/activate`

### Error: "Boot mode is not supported on Darwin"

**Cause**: You tried to use boot mode on a Darwin system.

**Solution**: Darwin's activate script doesn't support delayed activation. Don't pass `--boot`, `--test` or `--dry-run` when pushing to Darwin hosts. Without them, snow always switches Darwin hosts.

### Helper detection fails

**Cause**: The SSH command that checks for the helper (`command -v snowglobe-activate`) fails or times out. snow treats this as an error for that host, not as a missing helper.

**Solution**:
1. Verify SSH connectivity: `ssh target-host echo test`
2. Check that the helper is in PATH: `ssh target-host which snowglobe-activate`
3. Ensure SSH batch mode works: `ssh -o BatchMode=yes target-host echo test`

### Sudo still prompts for password

**Cause**: The sudoers configuration hasn't been applied or doesn't match the helper path.

**Solution**:
1. Verify the module is imported: `imports = [ inputs.snowglobe.nixosModules.default ];` (or `darwinModules.default`)
2. Check the user snowglobe connects as is in `allowedUsers`
3. Rebuild the system to apply sudoers changes
4. Test manually: `ssh target-host sudo -n -l /run/current-system/sw/bin/snowglobe-activate` prints the path if no password is needed
5. Check sudoers config: `ssh target-host sudo cat /etc/sudoers` on NixOS, or `/etc/sudoers.d/10-nix-darwin-extra-config` on Darwin (look for snowglobe-activate rules)

## Migration from Manual Configuration

If you're currently using manual sudoers configuration (`NOPASSWD: ALL` or wildcards), here's how to migrate:

1. **Add the module** to your configuration (don't remove old sudoers yet)
2. **Rebuild the system** to install the helper
3. **Test the helper** manually: `ssh target 'sudo snowglobe-activate "$(readlink -f /run/current-system)" switch'`
4. **Deploy with snowglobe**: `snow push target` (should use helper automatically)
5. **Remove old sudoers config** from your configuration
6. **Rebuild again** to clean up old sudoers rules

## Implementation Details

### Helper Script Location

The helper is installed via `environment.systemPackages`, making it available at a stable path:

```bash
/run/current-system/sw/bin/snowglobe-activate
```

This is a symlink to the actual helper in the Nix store (`/nix/store/<hash>-snowglobe-activate/bin/snowglobe-activate`). The sudoers configuration references the stable `/run/current-system/sw/bin/` path rather than the store path, which ensures that:
- The helper from the currently active system can be used to activate new systems
- No chicken-and-egg problem when the helper script itself is updated
- The sudoers rule doesn't need to change with every system update

### Source Code

The helper is a simple bash script defined in:
- NixOS: `modules/nixos-activate.nix`
- Darwin: `modules/darwin-activate.nix`
- Shared store path check: `modules/store-path-check.nix`

`modules/nixos.nix` and `modules/darwin.nix` install the script and configure sudo.

### Detection Logic

Snowglobe detects helper availability by running:

```bash
ssh target-host command -v snowglobe-activate
```

Exit code 0 means the helper is installed, and exit code 1 means it isn't, so snow falls back. Any other result, such as an SSH failure, is an error. The check runs before every activation and isn't cached. The reboot command checks for the helper again on its own.

## Related Documentation

- [darwin.md](darwin.md) - Darwin-specific deployment guide
- [push.md](push.md) - Complete push command documentation
