# Feature Request: System Activation Wrapper for Passwordless Deployment

## Problem

Currently, `snow push` (remote SSH deployment) requires passwordless sudo for the activation command. However, the activation binary path contains a Nix store hash that changes with every system update:

```bash
sudo /nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-darwin-system-25.05.d76299b/bin/switch-to-configuration switch
```

This creates a challenge for sudoers configuration:

1. **Wildcards don't work reliably**: On macOS (and sometimes Linux), sudoers rules like `NOPASSWD: /nix/store/*/bin/switch-to-configuration` don't match correctly due to how sudo validates command paths.

2. **Overly permissive workaround**: The only working solution is to allow all sudo commands without password:
   ```
   luiz ALL=(root) NOPASSWD: ALL
   ```
   This is a security concern, especially for production systems or CI/CD deployments.

3. **Platform differences**: NixOS uses `/bin/switch-to-configuration` while nix-darwin uses `/activate` at the root of the system closure, requiring different handling.

## Proposed Solution

Snowglobe should provide a NixOS/darwin module that installs a stable wrapper script for system activation. This wrapper would be installed at a fixed path and handle the platform-specific activation logic.

### Implementation

**1. Add a NixOS/darwin module to snowglobe's flake:**

```nix
# snowglobe/modules/default.nix
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.snowglobe.push;

  # Wrapper script that calls the appropriate activation command
  activationWrapper = pkgs.writeScriptBin "snowglobe-activate" ''
    #!${pkgs.bash}/bin/bash
    set -e

    if [ $# -lt 2 ]; then
      echo "Usage: snowglobe-activate <store-path> <mode>"
      echo "Modes: switch, boot, test, dry-activate"
      exit 1
    fi

    STORE_PATH="$1"
    MODE="$2"

    # Validate store path
    if [[ ! "$STORE_PATH" =~ ^/nix/store/ ]]; then
      echo "Error: Invalid store path (must start with /nix/store/)"
      exit 1
    fi

    # Check if this is NixOS or darwin
    if [ -f "$STORE_PATH/bin/switch-to-configuration" ]; then
      # NixOS
      exec "$STORE_PATH/bin/switch-to-configuration" "$MODE"
    elif [ -f "$STORE_PATH/activate" ]; then
      # nix-darwin
      # Darwin's activate script handles modes differently
      exec "$STORE_PATH/activate"
    else
      echo "Error: Could not find activation script in $STORE_PATH"
      exit 1
    fi
  '';
in
{
  options.snowglobe.push = {
    enable = mkEnableOption "snowglobe remote deployment support";

    allowedUsers = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Users allowed to run snowglobe-activate without password";
      example = [ "alice" "pusher" ];
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [ activationWrapper ];

    # Configure passwordless sudo for the wrapper script
    security.sudo.extraRules = [{
      users = cfg.allowedUsers;
      commands = [{
        command = "${activationWrapper}/bin/snowglobe-activate";
        options = [ "NOPASSWD" ];
      }];
    }];
  };
}
```

**2. Export the module from snowglobe's flake:**

```nix
# flake.nix
{
  outputs = { self, nixpkgs, ... }: {
    # ... existing outputs ...

    nixosModules.default = import ./modules/default.nix;
    darwinModules.default = import ./modules/default.nix;
  };
}
```

**3. Update snowglobe to use the wrapper script:**

In `snowlib/src/backends/deploy/mod.rs`, detect if the wrapper is available and prefer it:

```rust
fn activate(
    &self,
    target: &DeployTarget,
    store_path: &StorePath,
    mode: ActivationMode,
) -> Result<()> {
    let mode_arg = match mode {
        ActivationMode::Switch => "switch",
        ActivationMode::Boot => "boot",
        ActivationMode::Test => "test",
        ActivationMode::DryActivate => "dry-activate",
    };

    // Prefer snowglobe-activate wrapper if available
    // Fall back to direct activation for backward compatibility
    let activation_cmd = vec![
        "sudo", "snowglobe-activate", store_path.as_str(), mode_arg
    ];

    let mut cmd = self.ssh_command(target, &activation_cmd);
    // ... rest of implementation
}
```

### Usage

Users would configure their systems:

**NixOS:**
```nix
{
  imports = [ inputs.snowglobe.nixosModules.default ];

  snowglobe.push = {
    enable = true;
    allowedUsers = [ "luiz" "pusher" ];  # pusher for CI/CD
  };
}
```

**nix-darwin:**
```nix
{
  imports = [ inputs.snowglobe.darwinModules.default ];

  snowglobe.push = {
    enable = true;
    allowedUsers = [ "luiz" "pusher" ];
  };
}
```

The sudoers configuration would then contain:
```
luiz ALL=(root) NOPASSWD: /nix/store/<hash>-snowglobe-activate/bin/snowglobe-activate
pusher ALL=(root) NOPASSWD: /nix/store/<hash>-snowglobe-activate/bin/snowglobe-activate
```

This works because:
1. The wrapper script path is stable within a system generation
2. Users need to update sudoers only when the wrapper package itself changes (rare)
3. The wrapper validates inputs and only allows calling activation scripts from `/nix/store/`

## Benefits

1. **More secure**: Only allows running the specific activation wrapper, not all sudo commands
2. **Cross-platform**: Works identically on NixOS and nix-darwin
3. **Explicit**: Users opt-in by enabling the module and specifying allowed users
4. **Validated inputs**: The wrapper validates that paths are legitimate Nix store paths
5. **Better UX**: Users don't need to figure out the correct sudoers configuration

## Security Considerations

The wrapper script should:
- Validate that the store path starts with `/nix/store/`
- Validate that the mode is one of the allowed values
- Only execute activation scripts found in the provided store path
- Not allow arbitrary command injection

Since the Nix store is immutable (requires root to modify), and the wrapper only executes scripts from validated store paths, this is significantly more secure than `NOPASSWD: ALL`.

## Alternatives Considered

1. **Use `/run/current-system` path**: This only works for activating the currently built system, not for testing new configurations before switching to them.

2. **Different wrapper per system generation**: Would require updating sudoers after every system build, defeating the purpose.

3. **SSH as root directly**: Not recommended for security reasons; better to use sudo with specific allowed commands.

## Related Issues

This addresses the problem described in:
- `docs/darwin.md` - Current documentation suggests using wildcards that don't work reliably
- Remote deployments requiring `NOPASSWD: ALL` as a workaround
