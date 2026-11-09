---
title: "Deploy nix-darwin hosts"
group: "Guides"
weight: 60
description: "Deploy to Macs running nix-darwin, alongside or instead of NixOS hosts."
---

This guide sets up a Mac running [nix-darwin](https://github.com/nix-darwin/nix-darwin) so that `snow deploy` can deploy to it over SSH. Once it is set up, a Mac is one more host in your flake: it can share a deployment, a phase and tags with NixOS hosts.

Darwin support is less exercised than NixOS support: the darwin activation helper has a module test, but there are no VM tests for nix-darwin hosts, since macOS can't run in the NixOS test driver.

## Prerequisites

- A Mac with Nix and nix-darwin installed, and a nix-darwin configuration in a flake.
- A deploying machine that can build the Mac's system. snow builds every closure on the machine it runs on, never on the target, so this is either a Mac or a Linux machine with a Darwin remote builder. See [Build for other architectures](/snowglobe/architectures/).
- SSH key access from the deploying machine to the Mac, with the Mac's host key in your `known_hosts`. `snow` runs ssh in batch mode and never prompts for a password or a host key. See [Connect to your hosts](/snowglobe/connect/).
- A user on the Mac for deployments. A dedicated account is best; see [Set up a deploy user](/snowglobe/deploy-user/).

## How it differs from NixOS

nix-darwin has no equivalent of `switch-to-configuration`. A nix-darwin system closure has a single `activate` script that switches to the new system immediately, and macOS has nothing like NixOS's booted-system link to compare against. That shapes what snowglobe can do on a Mac:

| | NixOS | nix-darwin |
|---|---|---|
| Activation modes | switch, boot, test, dry-run | switch only |
| `--boot`, `--test`, `--dry-run` | supported | the whole deployment fails before anything is built |
| Reboot detection and `rebootPolicy` | used | not applied; snow never reboots a Mac |
| `post-reboot` hooks | run after a reboot | never run |
| Failing units after a switch | fail the host unless ignored | not checked |
| Address snow connects to | `networking.hostName` + `networking.domain` | the same; `networking.domain` is usually unset, so it is just `networking.hostName` |
| Without the snowglobe module | falls back to plain `sudo`, with a warning; needs passwordless sudo | fails (see [below](#why-the-module-is-required)) |

The mode check runs before the build so that a deployment mixing NixOS and nix-darwin hosts doesn't build everything and then stop. If you need `--boot` or `--dry-run` for your NixOS hosts, select them without the Macs, for example with a tag (`snow deploy --dry-run --tag servers`). Everything else in a deployment works the same on a Mac: phases, serial groups, tags, `optional`, hooks and `sshPort`.

## Set up the flake

Add snowglobe as an input and import `darwinModules.default` into each nix-darwin configuration:

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
        ./hosts/your-mac.nix
      ];
    };
  };
}
```

snowglobe reads `darwinConfigurations` as it is; no extra flake output is needed. The attribute name (`your-mac` here) is the name you pass to `snow deploy`.

## Configure the Mac

In the host's configuration:

```nix
{
  nixpkgs.hostPlatform = "aarch64-darwin";

  # snow connects to this name; nix-darwin leaves it unset by default
  networking.hostName = "your-mac";

  snowglobe.deploy.allowedUsers = [ "deploy" ];

  # nix copy as a non-root user only imports unsigned paths from a trusted user
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

What each part does:

- **`networking.hostName`** is required. snow reads it to know where to connect, and nix-darwin has no default for it. If the name doesn't resolve from the deploying machine, map it to an address in your `~/.ssh/config` (see [Connect to your hosts](/snowglobe/connect/)). If SSH listens on another port, set `snowglobe.deploy.sshPort`.
- **`snowglobe.deploy.allowedUsers`** adds a sudoers rule (through `security.sudo.extraConfig`) that lets those users run `/run/current-system/sw/bin/snowglobe-activate` as root without a password, and nothing else.
- **`nix.settings.trusted-users`** lets `nix copy` import the closure that snow sends. snow copies unsigned paths, and the Nix daemon accepts those only from trusted users.
- **`services.openssh`** turns on the SSH server. You can instead enable Remote Login in System Settings and manage `sshd_config` yourself.

Users in `allowedUsers` are effectively root on the Mac: anyone who can add a path to the Nix store can build a "system" whose activation script does anything, and run it through the helper. Use a dedicated user and protect its SSH key as you would a root key. The [Security model](/snowglobe/security/) explains this in detail.

## Apply it once by hand

The sudoers rule and the helper only exist once the configuration is active, so the first switch has to happen locally, with your own password:

```bash
sudo darwin-rebuild switch --flake .#your-mac
```

After that, from the flake's directory on the deploying machine:

```bash
snow list               # your-mac should appear, alongside any NixOS hosts
snow deploy your-mac
```

A successful deployment ends with a report line such as `✓ your-mac (switch)`. Later deployments skip the Mac when it already runs the system that was built, unless you pass `--redeploy`.

## Why the module is required

On NixOS hosts, snowglobe can fall back to running `sudo switch-to-configuration` over SSH when the activation helper is missing, which works if the SSH user has passwordless sudo. On a Mac there is no such fallback, for two reasons:

- snow can't type a sudo password over SSH. Every remote command runs non-interactively, so the deploy user needs a sudo rule that asks for no password. The module provides one that is limited to the helper, instead of full passwordless sudo.
- nix-darwin systems have no `switch-to-configuration`. The helper knows to run the closure's `activate` script, and to record the new system in `/nix/var/nix/profiles/system` first, which `activate` doesn't do on its own. Without that profile entry, the new system is not a generation: rollbacks, and activation at the next boot, would still see the previous one.

Without the helper, snow takes the NixOS path: it runs `sudo nix-env --profile /nix/var/nix/profiles/system --set <new system>` and then fails, because there is no `switch-to-configuration` to run. If the SSH user happens to have passwordless sudo, the system profile is left pointing at a system that was never activated. Install the module and switch once by hand before the first deployment.

## Determinate Nix

If the Mac runs [Determinate Nix](https://docs.determinate.systems/), nix-darwin is configured with `nix.enable = false` and doesn't manage Nix itself. The snowglobe module handles that case: the helper then uses the system-wide `nix-env` at `/nix/var/nix/profiles/default/bin/nix-env`, since the Nix package nix-darwin would otherwise provide isn't there. There is nothing to configure for it.

What does change is how you make the deploy user trusted. With `nix.enable = false`, nix-darwin doesn't write `nix.conf`, so `nix.settings.trusted-users` has no effect. Add the user to Determinate's custom configuration file on the Mac instead, `/etc/nix/nix.custom.conf`:

```ini
trusted-users = root deploy
```

Then restart the Nix daemon (or reboot) for the change to take effect.

## Rebuild a Mac locally

`snow rebuild` works on a Mac as well. It builds `darwinConfigurations.<name>` for this machine, where the name is the output of `hostname -s`, and switches to it. Use `--host` if the configuration is named differently. Only switching is supported, there is no reboot check, and it uses your own sudo rather than the activation helper, so it works without the snowglobe module. See [Rebuild the local machine](/snowglobe/rebuild/).

## Troubleshooting

**`sudo: a password is required`** at activation means the sudoers rule isn't in place for the SSH user: the module isn't imported, the configuration hasn't been switched to since adding it, or the user snow connects as isn't listed in `allowedUsers`. Check which user snow connects as (`--user`, then your SSH config, then your local user name), fix the configuration, and run `darwin-rebuild switch` by hand again. To check the rule on the Mac:

```bash
sudo -l -U deploy
```

The output should include `(root) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate`.

**A warning that `snowglobe-activate` was not found, followed by a failure** means snow ran `command -v snowglobe-activate` over SSH and the deploy user's non-interactive shell didn't find it. Either the module isn't active, or `/run/current-system/sw/bin` isn't on that shell's `PATH`.

**`nix-darwin can only switch, so your-mac can't boot`** (or `test`, `dry-activate`) means the deployment selected a Mac together with `--boot`, `--test` or `--dry-run`. Select only NixOS hosts for those modes.

**`Failed to parse Nix command output as JSON`** while snow reads the host's settings usually means `networking.hostName` is unset: nix-darwin leaves it `null`, and snow needs a name to connect to. Set it in the host's configuration.

**`nix copy` fails with a message about signatures or untrusted paths** means the deploy user isn't a trusted user. On Determinate Nix, set it in `/etc/nix/nix.custom.conf` as shown above.

For other errors, see [Troubleshooting](/snowglobe/troubleshooting/). The activation modes and reboot behaviour of NixOS hosts are covered in [Control reboots and activation modes](/snowglobe/reboots/), and deploying to Macs from CI in [Run from CI](/snowglobe/ci/).
