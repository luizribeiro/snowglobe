# Deploying to nix-darwin Hosts

This guide covers deploying to macOS hosts managed by nix-darwin.

## Prerequisites

### The snowglobe module

nix-darwin needs `sudo` to activate a system. snowglobe connects over
non-interactive SSH, so the deploy user needs passwordless sudo for the
activation command. The snowglobe nix-darwin module sets this up:

```nix
{
  imports = [ inputs.snowglobe.darwinModules.default ];

  snowglobe.push.allowedUsers = [ "deploy" ];
}
```

This module:
- Installs the `snowglobe-activate` command system-wide
- Configures passwordless sudo for that one command, for the listed users only

The module is required for remote pushes to Darwin hosts. Without
`snowglobe-activate`, snow falls back to running
`<system>/bin/switch-to-configuration`, which nix-darwin systems don't have,
so activation fails.

Deploy the module once by hand (you'll need to enter your password):

```bash
sudo darwin-rebuild switch --flake .#your-darwin-host
```

After that, snowglobe deploys without password prompts:

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

Any user who can activate systems without a password is root-equivalent, as with any push deployment tool. Use a dedicated deploy user and treat its SSH key like a root key.

The snowglobe module puts one fixed command in sudoers, with an exact path that sudo matches reliably.

See [activation-helper.md](activation-helper.md#security) for details.

## Troubleshooting

### "sudo: a password is required"

If you see this error:

```
sudo: a terminal is required to read the password; either use the -S option to read from standard input or configure an askpass helper
sudo: a password is required
```

**Solution**: Import the snowglobe module, add the SSH user to `snowglobe.push.allowedUsers`, and rebuild.

### Testing Sudo Configuration

Check that the deploy user may run the helper without a password:

```bash
ssh your-darwin-host sudo -n -l /run/current-system/sw/bin/snowglobe-activate
```

This prints the command's path if the rule applies, and fails without
prompting if it doesn't.

If it fails:
1. Check that the SSH user is in `snowglobe.push.allowedUsers`
2. Check that the active configuration includes the module: `command -v snowglobe-activate`
3. Check the rule in `/etc/sudoers.d/10-nix-darwin-extra-config`, where nix-darwin writes `security.sudo.extraConfig`

## Differences from NixOS

| Aspect | NixOS | nix-darwin |
|--------|-------|------------|
| Activation modes | switch, boot, test, dry-activate | switch only; `--boot`, `--test` and `--dry-run` fail |
| Reboot | Detected, and done when needed | Never; detection and `rebootPolicy` are ignored |
| Without the snowglobe module | Falls back to `switch-to-configuration` | Activation fails |

See [push.md](push.md) for full documentation on activation modes and reboot detection.

## Example Configuration

Here's a complete minimal nix-darwin host configuration for snowglobe:

```nix
{
  snowglobe.push.allowedUsers = [ "deploy" ];

  services.openssh = {
    enable = true;
    extraConfig = ''
      PermitRootLogin no
      PasswordAuthentication no
    '';
  };

  # Your other configuration...
}
```

Add snowglobe to your flake inputs and import its module once, here:

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

Example GitHub Actions workflow:

```yaml
- name: Deploy to Darwin hosts
  env:
    SSH_AUTH_SOCK: /tmp/ssh_agent.sock
  run: |
    ssh-agent -a $SSH_AUTH_SOCK > /dev/null
    ssh-add - <<< "${{ secrets.DEPLOY_SSH_KEY }}"
    snow push darwin-host-1 darwin-host-2
```

See [activation-helper.md](activation-helper.md) for complete documentation on the activation system.
