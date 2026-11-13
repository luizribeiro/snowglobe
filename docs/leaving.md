---
title: "Stop using snowglobe"
group: "Guides"
weight: 85
description: "What the snowglobe module adds to a host, how to take it out, and which tools to deploy with instead."
---

This guide shows how to remove snowglobe from your flake and your hosts, and what to deploy with afterwards. Nothing about it is one-way: snowglobe deploys ordinary NixOS and nix-darwin systems, and your hosts keep running whatever system they have until you deploy a new one.

## What snowglobe adds to a host

The module (`snowglobe.nixosModules.default` or `snowglobe.darwinModules.default`) adds only configuration. It runs no service or agent on the host.

- **The activation helper.** `snowglobe-activate` is installed in every host's system packages, whether or not anyone may use it.
- **A sudo rule.** For each user in `snowglobe.deploy.allowedUsers`, a passwordless sudo rule for `/run/current-system/sw/bin/snowglobe-activate`: through `security.sudo.extraRules` on NixOS and `security.sudo.extraConfig` on nix-darwin. With an empty list, no rule is added. [Set up a deploy user](/snowglobe/deploy-user/#the-sudo-rule-the-module-writes) shows the rule.
- **The `snowglobe.*` options.** Phases, tags, reboot policy, hooks and the rest; see [Module options](/snowglobe/options/). They only matter to `snow`.
- **`/etc/snowglobe/remote-hooks`**, when the host has remote hooks. It keeps what those hooks reference in the system closure.

The deploy user itself, its SSH key and `nix.settings.trusted-users` are your own configuration, not the module's. Keep them if the next tool logs in as the same user.

## Switch to another tool first

Get the replacement working before you take snowglobe out, so you always have a way to deploy. All of these build the same system configurations (deploy-rs and colmena need their own flake output too) and write the same system profile, `/nix/var/nix/profiles/system`, so the hosts' generations and rollbacks carry over.

- **`nixos-rebuild --target-host`**, which ships with NixOS and deploys one host per run. With `--sudo --ask-sudo-password` it can use an account that needs a password for sudo.
- **[deploy-rs](https://github.com/serokell/deploy-rs)**, for automatic rollback, non-system profiles, or nix-darwin hosts.
- **[colmena](https://github.com/zhaofengli/colmena)**, for parallel deployments of NixOS hosts, tags, and secrets upload.
- **`darwin-rebuild switch`** on each Mac.

[Comparison with other tools](/snowglobe/comparison/) sets these side by side; phases, serial groups, hooks and reboot detection have no direct equivalent in them. The `snowglobe.deploy.*` settings have rough equivalents in deploy-rs and colmena; the tables in [Coming from deploy-rs or colmena](/snowglobe/migrating/#setting-mappings) map them in the other direction.

The new tool needs its own way to become root on the host. The snowglobe sudo rule only permits `snowglobe-activate`, which other tools don't call. Give the deploy user the access the tool expects, such as a root login or passwordless sudo (colmena), `interactiveSudo` (deploy-rs), or an account that already has sudo.

## Remove the module

In one change to your flake:

1. Remove `snowglobe.nixosModules.default` and `snowglobe.darwinModules.default` from each host's modules.
2. Remove every `snowglobe.*` setting from the hosts' configurations. Once the module is gone, any that remain fail evaluation with an error that the option does not exist.
3. Remove the `snowglobe` input from `flake.nix`, along with anything else that uses it, such as `snowglobe.packages.<system>.snow` in a development shell. Then run `nix flake lock` to drop it from `flake.lock`.

Deploy this change with the new tool. Once a host activates it, the helper and the sudo rule are gone from that host. Hosts you have not deployed to yet keep running their current system, helper and rule included, until you do.

Older generations still contain the helper and the rule, so rolling a host back to one of them brings both back until you deploy again. If that matters to you, [prune the old generations](/snowglobe/rollback/#prune-old-generations) once the change is everywhere.

To uninstall the CLI, remove `snow` from wherever you installed it, for example with `nix profile remove` (find the entry's name with `nix profile list`).
