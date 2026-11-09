# nix-darwin hosts

snow pushes to Macs running nix-darwin too. It works like NixOS, minus the
reboot stuff. The machine running `snow` can be a Mac as well.

Less tested than NixOS, though: Darwin only has a module test, no VM tests.

## Differences

| | NixOS | nix-darwin |
|---|---|---|
| Modes | switch, boot, test, dry-activate | switch only. `--boot`, `--test` and `--dry-run` fail before anything is built |
| Reboots | detected, done when needed | never. Detection and `rebootPolicy` don't apply |
| `post-reboot` hooks | after a reboot | never run |
| Address | `networking.hostName` + `networking.domain` | the same, but `networking.domain` is unset by default, so usually just the hostName |
| Without the snowglobe module | falls back to `switch-to-configuration` | fails (see below) |

## Setup

`flake.nix`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    darwin.url = "github:nix-darwin/nix-darwin";
    darwin.inputs.nixpkgs.follows = "nixpkgs";
    snowglobe.url = "github:luizribeiro/snowglobe";
  };

  outputs = { nixpkgs, darwin, snowglobe, ... }: {
    darwinConfigurations.your-mac = darwin.lib.darwinSystem {
      modules = [
        snowglobe.darwinModules.default
        ./configuration.nix
      ];
    };
  };
}
```

`configuration.nix`:

```nix
{
  nixpkgs.hostPlatform = "aarch64-darwin";
  networking.hostName = "your-mac";   # snow connects to this

  snowglobe.deploy.allowedUsers = [ "deploy" ];
  # nix copy as a non-root user needs a trusted user (or signed paths)
  nix.settings.trusted-users = [ "deploy" ];

  services.openssh = {
    enable = true;
    extraConfig = ''
      PermitRootLogin no
      PasswordAuthentication no
    '';
  };
}
```

`allowedUsers` lets those users run `snowglobe-activate` with sudo and no
password, and nothing else. snow can't type a password over SSH, so on a Mac
the module is required. Those users are root-equivalent on the Mac: use a
dedicated one and guard its SSH key like a root key. More in
[activation-helper.md](activation-helper.md#security).

Apply it once by hand (this one asks for your password):

```bash
sudo darwin-rebuild switch --flake .#your-mac
```

Then:

```bash
snow push your-mac
```

### Determinate Nix

With Determinate (`nix.enable = false`), the helper uses
`/nix/var/nix/profiles/default/bin/nix-env`. Nothing to set for that.
`nix.settings` isn't available then, so add the deploy user to
`trusted-users` in `/etc/nix/nix.custom.conf` instead.

### Without the module

snow runs `sudo nix-env --profile … --set <new system>` first, and only then
fails, since nix-darwin systems have no `switch-to-configuration`. If that
sudo works without a password, the profile is left pointing at a system that
was never activated. Install the module before pushing.

## snow rebuild

On a Mac, `snow rebuild` builds `darwinConfigurations.<hostname -s>` and
switches. No reboot check, no hooks. `--host` to pick another config.

## From CI

1. Put the CI user in `allowedUsers`.
2. Give the runner an SSH key for it.
3. Add the Mac's host key to the runner's `known_hosts`.

```yaml
- name: Deploy Macs
  env:
    SSH_AUTH_SOCK: /tmp/ssh_agent.sock
  run: |
    ssh-agent -a $SSH_AUTH_SOCK > /dev/null
    ssh-add - <<< "${{ secrets.DEPLOY_SSH_KEY }}"
    snow push mac-1 mac-2
```

## Troubleshooting

`sudo: a password is required` means the module isn't active on the Mac or
the SSH user isn't in `allowedUsers`. Fix the config and run `darwin-rebuild`
by hand again. To check the rule, and for the rest:
[activation-helper.md](activation-helper.md#troubleshooting).

Modes and reboots on NixOS: [push.md](push.md).
