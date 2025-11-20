# Deploying to nix-darwin Hosts

This guide covers deploying to macOS hosts managed by nix-darwin.

## Prerequisites

### Passwordless Sudo

nix-darwin requires `sudo` to activate system configurations. Since snowglobe uses non-interactive SSH, you must configure passwordless sudo for the activation command.

**Recommended: Use the snowglobe module**

Snowglobe provides a nix-darwin module that handles passwordless deployment configuration automatically:

```nix
{
  # Import snowglobe's Darwin module
  imports = [ inputs.snowglobe.darwinModules.default ];

  # Configure passwordless deployment users
  snowglobe.push.allowedUsers = [ "your-username" ];
}
```

This module:
- Installs the `snowglobe-activate` command system-wide
- Configures passwordless sudo for specified users only (more secure)
- Validates inputs to prevent unauthorized command execution
- Works with both local and remote deployments

Snowglobe automatically detects and uses `snowglobe-activate` when available, falling back to direct activation for compatibility.

**Alternative: Manual sudoers configuration**

If you prefer not to use the module, you can configure sudoers manually:

```nix
{
  security.sudo.extraConfig = ''
    # Allow admin users to run system activation without password
    # Note: This is less secure than using the snowglobe module
    %admin ALL=(root) NOPASSWD: /nix/store/*/activate
  '';
}
```

⚠️ **Note**: The wildcard pattern (`/nix/store/*/activate`) may not work reliably on macOS due to sudo's path validation. The snowglobe module is strongly recommended.

After adding your chosen configuration:

1. Deploy it manually once (you'll need to enter your password):
   ```bash
   darwin-rebuild switch
   ```

2. Future snowglobe deployments will work without password prompts:
   ```bash
   snow push your-darwin-host
   ```

### Why Passwordless Sudo?

Snowglobe uses SSH in batch mode (`-o BatchMode=yes`) for reliable, non-interactive deployments. This is essential for:
- Deploying to multiple hosts in parallel
- Running deployments from CI/CD
- Consistent behavior across all hosts

Since batch mode SSH cannot prompt for passwords, the target host must allow passwordless sudo for the specific activation command.

## Security Considerations

When using the **snowglobe module** (recommended), security is enhanced because:

1. **Stable command path**: Only `snowglobe-activate` can run without password, not arbitrary store paths
2. **Input validation**: Validates that paths start with `/nix/store/` and modes are valid
3. **Explicit allowlist**: Only users in `allowedUsers` can activate without password
4. **No wildcards**: sudoers rules use exact paths, avoiding wildcard matching issues
5. **Immutable store**: Users cannot modify `/nix/store` contents without root

When using **manual sudoers configuration**, consider:

1. **Wildcard limitations**: The pattern `/nix/store/*/activate` may not work reliably on macOS
2. **Broader access**: Any user in the specified group can activate any store path
3. **Less validation**: No additional input validation beyond sudo's path matching

The activation script itself is a trusted component of nix-darwin that validates and applies system configurations.

## Troubleshooting

### "sudo: a password is required"

If you see this error:

```
sudo: a terminal is required to read the password; either use the -S option to read from standard input or configure an askpass helper
sudo: a password is required
```

**Solution**: Add the passwordless sudo configuration above to your nix-darwin config and rebuild.

### Testing Sudo Configuration

After configuring passwordless sudo, test it manually:

```bash
# SSH to the host and try running the activation command
ssh your-darwin-host sudo /nix/store/*/bin/switch-to-configuration test

# Should not prompt for password
```

If it still prompts for a password:
1. Verify the `security.sudo.extraConfig` is in your active configuration
2. Check `/etc/sudoers` includes the rule (or `/etc/sudoers.d/` files)
3. Ensure your SSH user is in the `admin` group: `groups`

### Alternative: Specific User

If you want to limit this to a specific deployment user instead of all admin users:

```nix
{
  security.sudo.extraConfig = ''
    # Replace 'deployuser' with your SSH username
    deployuser ALL=(root) NOPASSWD: /nix/store/*/bin/switch-to-configuration
  '';
}
```

## Differences from NixOS

| Aspect | NixOS | nix-darwin |
|--------|-------|------------|
| Activation | Requires root/sudo | Requires sudo |
| System updates | Can run as root directly | Always runs as user with sudo |
| Reboot detection | Full support | Limited (no kernel/initrd on macOS) |
| Activation modes | switch, boot, test | switch, activate (boot=activate) |

See [push.md](push.md) for full documentation on activation modes and reboot detection.

## Example Configuration

Here's a complete minimal nix-darwin host configuration for snowglobe:

```nix
{
  # Import snowglobe's Darwin module
  imports = [ inputs.snowglobe.darwinModules.default ];

  # Configure passwordless deployment users
  snowglobe.push.allowedUsers = [ "your-username" "ci-bot" ];

  # Configure SSH for remote deployment
  services.openssh = {
    enable = true;
    settings = {
      PermitRootLogin = "no";
      PasswordAuthentication = false;
    };
  };

  # Optional: Set reboot policy for this host
  # (Darwin rarely needs reboots, so 'never' is often appropriate)
  config.core.rebootPolicy = "never";  # or "auto"

  # Your other configuration...
}
```

Add snowglobe to your flake inputs:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    darwin.url = "github:LnL7/nix-darwin";
    snowglobe.url = "github:luizribeiro/snowglobe";
  };

  outputs = { self, nixpkgs, darwin, snowglobe }: {
    darwinConfigurations.your-host = darwin.lib.darwinSystem {
      modules = [
        snowglobe.darwinModules.default
        ./configuration.nix
      ];
    };
  };
}
```

## CI/CD Deployments

For automated deployments (GitHub Actions, etc.):

1. Add the snowglobe module with a CI user in `allowedUsers`
2. Set up SSH key authentication for the CI runner
3. Add the target host's SSH key to known_hosts
4. Use `snow push --skip-reboot` to avoid interactive reboot prompts

Example GitHub Actions workflow:

```yaml
- name: Deploy to Darwin hosts
  env:
    SSH_AUTH_SOCK: /tmp/ssh_agent.sock
  run: |
    ssh-agent -a $SSH_AUTH_SOCK > /dev/null
    ssh-add - <<< "${{ secrets.DEPLOY_SSH_KEY }}"
    snow push --skip-reboot darwin-host-1 darwin-host-2
```

Snowglobe will automatically detect if `snowglobe-activate` is available and use it for deployments. If not found (e.g., first deployment), it falls back to direct activation with a warning.

See [activation-helper.md](activation-helper.md) for complete documentation on the activation system.
