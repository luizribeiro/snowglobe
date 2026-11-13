---
title: "Comparison with other tools"
group: "Get started"
weight: 15
description: "How snowglobe compares with nixos-rebuild --target-host, deploy-rs, colmena, morph, Clan, devenv Machines and pull-based tools, and when each is the better fit."
---

Several tools deploy NixOS configurations to remote machines, and they overlap a lot. This page compares snowglobe in detail with `nixos-rebuild --target-host`, [deploy-rs](https://github.com/serokell/deploy-rs) and [colmena](https://colmena.cli.rs/), then with morph, Clan and devenv Machines, and briefly with pull-based tools such as comin. Most of these tools are older and more widely used than snowglobe, which is pre-1.0.

The short version: snowglobe is built around rolling a change out across a fleet in a controlled order, with phases, serial groups, hooks and reboot detection. Some other tools also deploy in order or gate a rollout on health checks; morph and devenv Machines are the closest. snowglobe leaves out several things other tools do, such as building on the target, uploading secrets and rolling back automatically.

## At a glance

Compared with the three tools most users are choosing between:

| | `nixos-rebuild --target-host` | deploy-rs | colmena | snowglobe |
|---|---|---|---|---|
| Many hosts per run | — | ✓ | ✓ | ✓ |
| Ordered phases that stop on failure | — | — | — | ✓ |
| Serial groups (one node at a time) | — | — | — | ✓ |
| Reboot detection | — | — | `apply --reboot` reboots every node, without detection | ✓ |
| Pre/post-deploy hooks | — | — | — | ✓ |
| nix-darwin targets | — | ✓ | — | ✓ |
| Automatic rollback | — | ✓ (magic rollback) | — | — |
| Build on the target | `--build-host` | `remoteBuild` | `buildOnTarget` | — |
| Secrets upload | — | — | `deployment.keys` | — |
| Sudo password prompt on the target | `--ask-sudo-password` | `interactiveSudo` | — | — |
| Works without flakes | ✓ | partial (flake-less mode) | ✓ (`hive.nix`) | — |
| Extra flake output needed | none | `deploy.nodes` | `colmenaHive` | none |

A dash means the tool has no built-in support for that feature. In most cases you can still get it with a wrapper script.

## nixos-rebuild --target-host

`nixos-rebuild` ships with NixOS. With `--target-host` it builds a configuration locally (or on `--build-host`) and activates it on one remote machine over SSH. `--sudo` activates as a non-root user, and `--ask-sudo-password` asks for that user's password once at the start. It works with flakes or with a plain `configuration.nix`, and it needs nothing installed beyond NixOS itself.

It is the better fit when you have one or two machines, when you want to avoid adding a dependency, or when your configuration does not use flakes. It deploys one host per invocation, so ordering, parallelism and reboots across several hosts are up to you. snowglobe also offers [`snow rebuild`](/snowglobe/rebuild/) for the local machine, which works like `nixos-rebuild` with snowglobe's reboot check added.

## deploy-rs

deploy-rs deploys profiles described under a `deploy.nodes` flake output. Its main strength is magic rollback: after activating, it waits for a confirmation over a new SSH connection, and if that confirmation never arrives (for example because the change broke networking or SSH), the host rolls itself back. When one of several deployments fails, the deployments that already succeeded are rolled back too. deploy-rs can also deploy profiles other than the system profile, for any user, can build on the target with `remoteBuild`, can activate nix-darwin systems, and can prompt for a sudo password with `interactiveSudo`.

It is the better fit when automatic rollback matters more to you than staged rollout, for example for remote machines you cannot easily reach if a deployment breaks their network, or when you deploy home-manager or other non-system profiles. It has no hooks, no reboot handling and no ordering between nodes; `profilesOrder` orders profiles within one node.

## colmena

colmena reads a hive, either a `hive.nix` file without flakes or a `colmenaHive` flake output built with `colmena.lib.makeHive`. It deploys many NixOS hosts in parallel, selects them by tag, and can build on the targets with `buildOnTarget`. Its `deployment.keys` option uploads secrets to the host out of band, so they never enter the Nix store. `colmena apply --reboot` sets the new system for the next boot and reboots every selected node, whether or not the change needs it. When the SSH user is not root, colmena uses `deployment.privilegeEscalationCommand` (`sudo -H --` by default), which must work without a password. colmena deploys only to NixOS hosts, not to nix-darwin.

It is the better fit when you need secrets upload or remote builds, when your configuration does not use flakes, or when you want a mature, widely used tool and do not need staged rollout.

## snowglobe

snowglobe reads the `nixosConfigurations` and `darwinConfigurations` your flake already has, so it needs no extra flake output. Per-host settings go in the NixOS or nix-darwin configuration through a module. On top of parallel deployment it adds:

- [Phases](/snowglobe/phases/) that deploy in order and stop before the next phase if a host fails.
- [Serial groups](/snowglobe/cluster/) so cluster nodes deploy one at a time.
- [Hooks](/snowglobe/hooks/) before and after activation, after a reboot, and on failure, running either on your machine or on the host. A failed hook can [gate the rollout on a health check](/snowglobe/health-checks/).
- [Reboot detection](/snowglobe/reboot-detection/) with a per-host reboot policy.
- NixOS and [nix-darwin](/snowglobe/darwin/) hosts in the same deployment.

What it does not do:

- **No build on the target.** Every system is built on the machine running `snow`, or on remote builders that Nix itself is configured to use. To build for another architecture, see [Build for other architectures](/snowglobe/architectures/).
- **No secrets upload.** Use a tool such as agenix or sops-nix, which decrypt secrets on the host.
- **No automatic rollback.** If a deployment breaks a host, you roll it back yourself; see [Roll back a bad deployment](/snowglobe/rollback/).
- **No sudo password prompt on remote hosts.** The deploy user runs one activation helper through passwordless sudo; the [security model](/snowglobe/security/) explains why and what that means. `snow rebuild` on the local machine does ask for your password.
- **Flakes are required.**

snowglobe is the better fit when you deploy a fleet that should change gradually: low-risk hosts first, a cluster one node at a time, a host drained before it reboots, and reboots only when a change needs them. It is also the newest of these tools, with the smallest user base.

## Moving between tools

The tools above can coexist with snowglobe, since snowglobe reads ordinary `nixosConfigurations`. If you already use deploy-rs or colmena, [Coming from deploy-rs or colmena](/snowglobe/migrating/) shows how their settings map to snowglobe's.

## Corrections

This comparison was checked against each tool's documentation, but the tools change. If a cell is wrong or out of date, [open an issue](https://github.com/luizribeiro/snowglobe/issues).
