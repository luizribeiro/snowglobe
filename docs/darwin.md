# Deploying to nix-darwin Hosts

This guide covers deploying to macOS hosts managed by nix-darwin.

## Prerequisites

### Passwordless Sudo

nix-darwin requires `sudo` to activate system configurations. Since snowglobe uses non-interactive SSH, you must configure passwordless sudo for the activation command.

**Add this to your nix-darwin configuration:**

```nix
{
  security.sudo.extraConfig = ''
    # Allow admin users to run system activation without password
    %admin ALL=(root) NOPASSWD: /nix/store/*/bin/switch-to-configuration
  '';
}
```

After adding this configuration:

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

The passwordless sudo configuration is **safe** because:

1. **Command-specific**: Only `/nix/store/*/bin/switch-to-configuration` can run without a password
2. **No shell access**: The wildcard only matches the Nix store path pattern
3. **Immutable store**: Users cannot modify `/nix/store` contents without root
4. **Standard practice**: This is the same approach used by deploy-rs, NixOps, and similar tools

The activation script itself validates and applies the system configuration - it's a trusted component of nix-darwin.

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
  # Enable passwordless sudo for system activation
  security.sudo.extraConfig = ''
    %admin ALL=(root) NOPASSWD: /nix/store/*/bin/switch-to-configuration
  '';

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

## CI/CD Deployments

For automated deployments (GitHub Actions, etc.):

1. Configure passwordless sudo as described above
2. Set up SSH key authentication for the CI runner
3. Add the target host's SSH key to known_hosts
4. Use `snow push --no-reboot` to avoid interactive reboot prompts

Example GitHub Actions workflow:

```yaml
- name: Deploy to Darwin hosts
  env:
    SSH_AUTH_SOCK: /tmp/ssh_agent.sock
  run: |
    ssh-agent -a $SSH_AUTH_SOCK > /dev/null
    ssh-add - <<< "${{ secrets.DEPLOY_SSH_KEY }}"
    snow push --no-reboot darwin-host-1 darwin-host-2
```
