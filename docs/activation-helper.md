# System Activation

This document describes snowglobe's activation system, which provides secure passwordless deployment for both NixOS and nix-darwin hosts.

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
2. Validates inputs to prevent command injection
3. Detects the platform (NixOS vs Darwin) and calls the appropriate activation script
4. Has a stable path for sudoers configuration

**Automatic detection**: Snowglobe automatically detects if `snowglobe-activate` is available on target hosts and uses it when present. If not found, it falls back to direct activation with a warning - no configuration needed.

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
  snowglobe.push.allowedUsers = [ "deploy-user" "ci-bot" ];
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
  snowglobe.push.allowedUsers = [ "your-username" "ci-bot" ];
}
```

Rebuild your system:

```bash
darwin-rebuild switch
```

## Configuration Options

### `snowglobe.push.allowedUsers`

Type: `list of strings`
Default: `[]`
Example: `[ "alice" "deploy-bot" ]`

List of users who can run `snowglobe-activate` without a password. These users will be able to activate any system configuration from the Nix store, but cannot modify the store itself (which requires root).

**Security note**: Only add trusted users and service accounts to this list. While the helper validates inputs, users in this list can still activate arbitrary system configurations from `/nix/store/`.

## Usage

### With snowglobe (automatic)

When using `snow push`, snowglobe automatically:

1. Detects if `snowglobe-activate` is available on the target host
2. Uses it if present for better security
3. Falls back to direct activation if not found, with a warning

```bash
# Automatically uses snowglobe-activate if available
snow push hostname
```

No flags needed - it just works!

### Manual usage

The helper can be called directly:

```bash
# Activate a system in switch mode
sudo snowglobe-activate /nix/store/abc123-nixos-system-24.05 switch

# Set for next boot only
sudo snowglobe-activate /nix/store/abc123-nixos-system-24.05 boot

# Test activation (non-permanent)
sudo snowglobe-activate /nix/store/abc123-nixos-system-24.05 test

# Dry run (show what would change)
sudo snowglobe-activate /nix/store/abc123-nixos-system-24.05 dry-activate
```

## How It Works

### Input Validation

The helper validates inputs before execution:

1. **Store path validation**: Must start with `/nix/store/`
2. **Mode validation**: Must be one of: `switch`, `boot`, `test`, `dry-activate`
3. **Platform detection**: Checks which activation script exists
4. **Activation script existence**: Verifies the script is present

### Platform-Specific Behavior

**NixOS**:
- Uses `/nix/store/<hash>/bin/switch-to-configuration`
- Supports all four activation modes: switch, boot, test, dry-activate
- Passes mode argument directly to the activation script

**nix-darwin**:
- Uses `/nix/store/<hash>/activate`
- Only supports `switch` mode (Darwin's activate script doesn't support modes)
- Returns explicit errors for unsupported modes (boot, test, dry-activate)

### Sudoers Configuration

The module automatically configures sudo to allow passwordless execution using a stable path:

**NixOS and Darwin**:
```
user ALL=(root) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate
```

This uses `/run/current-system/sw/bin/` which is a stable path that always points to the currently active system's binaries. This solves the chicken-and-egg problem: when pushing a new system, the helper from the **current** system is used to activate the **new** system. After activation completes, `/run/current-system` is updated to point to the new system.

## Security

### Threat Model

The helper protects against:

1. **Arbitrary command execution**: Only validated store paths and modes are accepted
2. **Path traversal**: Store path must start with `/nix/store/`
3. **Command injection**: Arguments are properly quoted and validated
4. **Mode confusion**: Only valid activation modes are accepted

The helper does **not** protect against:

1. **Malicious store paths**: If an attacker can place malicious activation scripts in `/nix/store/`, they can execute them. However, modifying `/nix/store/` requires root access.
2. **Social engineering**: If an attacker convinces a user to run activation manually with a malicious store path

### Comparison to Alternatives

| Approach | Security | Reliability | Ease of Use |
|----------|----------|-------------|-------------|
| snowglobe-activate helper | ✓ Best | ✓ Excellent | ✓ Simple |
| `/nix/store/*/activate` wildcard | ⚠️ Moderate | ✗ Unreliable on macOS | ✓ Simple |
| `NOPASSWD: ALL` | ✗ Dangerous | ✓ Always works | ✓ Very simple |
| Per-generation sudoers | ✓ Good | ⚠️ Requires automation | ✗ Complex |

### Best Practices

1. **Minimize allowedUsers**: Only add users who need deployment access
2. **Use --require-helper in CI**: Ensures configuration is correct
3. **Monitor helper updates**: Watch for security updates to snowglobe
4. **Audit activations**: Check system logs for unexpected activation attempts
5. **Use dedicated deployment users**: Create separate users for CI/CD rather than using personal accounts

## Troubleshooting

### Warning: "snowglobe-activate not found, falling back to direct activation"

**Cause**: The snowglobe module isn't installed or configured on the target host yet.

**Solution**: This is normal for first deployments. To enable better security:
1. Add snowglobe module to your configuration: `imports = [ inputs.snowglobe.nixosModules.default ];`
2. Add your username to `snowglobe.push.allowedUsers`
3. Rebuild the system: `nixos-rebuild switch` or `darwin-rebuild switch`

On subsequent deployments, snowglobe will automatically use `snowglobe-activate`.

### Error: "Invalid store path (must start with /nix/store/)"

**Cause**: The store path argument doesn't start with `/nix/store/`.

**Solution**: Ensure you're passing a valid Nix store path as the first argument.

### Error: "Activation script not found"

**Cause**: The store path doesn't contain an activation script.

**Solution**: Verify the store path points to a valid system configuration:
- NixOS: Check for `<path>/bin/switch-to-configuration`
- Darwin: Check for `<path>/activate`

### Error: "Boot mode is not supported on Darwin"

**Cause**: You tried to use boot mode on a Darwin system.

**Solution**: Darwin's activate script doesn't support delayed activation. Use switch mode instead, or configure your flake to avoid selecting boot mode for Darwin hosts.

### Helper detection fails

**Cause**: SSH command to check for helper (`command -v snowglobe-activate`) fails or times out.

**Solution**:
1. Verify SSH connectivity: `ssh target-host echo test`
2. Check that the helper is in PATH: `ssh target-host which snowglobe-activate`
3. Ensure SSH batch mode works: `ssh -o BatchMode=yes target-host echo test`

### Sudo still prompts for password

**Cause**: The sudoers configuration hasn't been applied or doesn't match the helper path.

**Solution**:
1. Verify the module is enabled: `snowglobe.push.enable = true`
2. Check your username is in `allowedUsers`
3. Rebuild the system to apply sudoers changes
4. Test manually: `ssh target-host sudo snowglobe-activate --help`
5. Check sudoers config: `ssh target-host sudo cat /etc/sudoers` (look for snowglobe-activate rules)

## Migration from Manual Configuration

If you're currently using manual sudoers configuration (`NOPASSWD: ALL` or wildcards), here's how to migrate:

1. **Add the module** to your configuration (don't remove old sudoers yet)
2. **Rebuild the system** to install the helper
3. **Test the helper** manually: `ssh target sudo snowglobe-activate /run/current-system switch`
4. **Deploy with snowglobe**: `snow push target` (should use helper automatically)
5. **Enable --require-helper** in your deployment scripts
6. **Remove old sudoers config** from your configuration
7. **Rebuild again** to clean up old sudoers rules

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
- NixOS: `modules/nixos.nix`
- Darwin: `modules/darwin.nix`

Both versions follow the same logic with platform-specific differences for activation script paths and mode support.

### Detection Logic

Snowglobe detects helper availability by running:

```bash
ssh target-host command -v snowglobe-activate
```

This returns exit code 0 if the helper is in PATH, non-zero otherwise. The check runs before each activation and is cached for the duration of the SSH connection.

## Related Documentation

- [darwin.md](darwin.md) - Darwin-specific deployment guide
- [push.md](push.md) - Complete push command documentation
- [Original feature request](../docs/feature-requests/activation-helper.md) - Design rationale and discussion
