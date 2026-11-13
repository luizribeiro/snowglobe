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
| Health check gates the next host | — | — | — | through a [`post-deploy` hook](/snowglobe/rollback/#limit-the-blast-radius) |
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

## More push tools

These three push tools are also worth knowing. morph and devenv Machines can deploy one host at a time and stop when a health check fails, which is the closest any tool here comes to snowglobe's staged rollout. Clan is a whole fleet framework, with deployment as one part.

| | [morph](https://github.com/DBCDK/morph) | [Clan](https://clan.lol/) | [devenv Machines](https://devenv.sh/machines/) | snowglobe |
|---|---|---|---|---|
| Many hosts per run | ✓ | ✓ | ✓ | ✓ |
| Ordered phases that stop on failure | hosts in tag order (`--order-by-tags`), stopping at the first failure | — | batches in name order; later batches don't start after a failure | ✓ |
| Serial groups (one node at a time) | always one host at a time | — | one at a time by default; `--max-concurrent` for batches | ✓ |
| Health check gates the next host | `deployment.healthChecks` | — | `deploy.healthCheck` | through a [`post-deploy` hook](/snowglobe/rollback/#limit-the-blast-radius) |
| Reboot detection | `--reboot` reboots every host, without detection | stops and asks you to reboot when a switch inhibitor blocks the switch | — | ✓ |
| Pre/post-deploy hooks | `preDeployChecks` and `healthChecks` only | — | — | ✓ |
| nix-darwin targets | — | ✓ | ✓ | ✓ |
| Automatic rollback | — | — | ✓ (NixOS only) | — |
| Build on the target | — | by default; `--build-host` to change | `--use-machines-as-builders` | — |
| Secrets upload | `deployment.secrets` | ✓ (Clan vars) | at install only | — |
| Sudo password prompt on the target | `--passwd` / `--passcmd` | ✓ (askpass) | — | — |
| Works without flakes | ✓ (flakes not supported) | — | ✓ (`devenv.nix`) | — |
| Extra flake output needed | n/a (a network file) | the `clan-core.lib.clan` outputs | none (`machines` in `devenv.nix`) | none |

## morph

morph deploys a nixops-style network file, such as `morph deploy network.nix switch`. It does not support flakes. It builds every selected host first, then deploys them strictly one at a time: push the closure, upload secrets, run `preDeployChecks`, activate, optionally reboot, then run `healthChecks`. A health check is either a command on the host or an HTTP request from the deployer, retried until it passes. If a check fails, morph stops and leaves the remaining hosts untouched. With the default `--timeout 0`, though, a failing check retries forever, so set a timeout. morph also uploads secrets out of band with `deployment.secrets` and can prompt for a sudo password. `--reboot` reboots every host, whether or not it needs it.

It is the better fit when your configuration does not use flakes and you want each host health-checked before the next one is touched. Because every host deploys in turn, a large fleet takes as long as all its hosts added together; snowglobe deploys in parallel and goes one at a time only within a serial group. morph does not detect whether a host needs a reboot, has no rollback and does not deploy to nix-darwin. It has had no release since October 2024.

## Clan

Clan is a framework for managing a whole fleet, not only a deployment tool. It covers an inventory of machines, shared services such as a VPN, backups and SSH certificates, generated secrets ("vars") and installation. Its flake is built with `clan-core.lib.clan`. `clan machines update` deploys the selected machines over SSH, all at once, and a failure on one machine does not stop the others. By default each target builds its own system; `--build-host` moves the build elsewhere. Clan uploads secrets on every update, can prompt for a sudo password and can update nix-darwin machines.

Since 26.05, Clan sets the new system as the boot default before switching to it. When a NixOS switch inhibitor blocks the live switch, Clan stops and asks you to reboot the machine. It does not decide on its own that a reboot is needed, and it does not reboot.

It is the better fit when you want one framework for the whole fleet, with secrets, networking, backups and installation configured through a shared inventory. It has no ordering between machines, hooks or health checks. If you need those, you can still deploy a Clan flake with another tool, because Clan also exports ordinary `nixosConfigurations`.

## devenv Machines

devenv 2.4 (September 2026) added `devenv machines`, which its release notes mark as experimental. Machines are configured in `devenv.nix` rather than in a flake output, and can be NixOS, nix-darwin or home-manager targets. devenv builds and copies every machine before any of them activates, shows a plan and asks for confirmation; a plan can also be saved and applied later. Machines then activate one at a time in name order, or in batches with `--max-concurrent`. If a batch fails, the machines already activating finish, but later batches do not start. After activation, `deploy.healthCheck` runs. On NixOS, a watchdog restores the previous system if activation or the health check fails, or if success is not confirmed in time. A `machines check` command also blocks a deployment that would disable SSH or root login.

It is the better fit when you already use devenv, or when you want a failed health check to roll that host back automatically as well as stop the rest of the deployment. It also deploys home-manager profiles and can install new machines. It has no hooks, so there is nowhere to drain a node before it changes, and it does not reboot after a kernel change. NixOS targets need root SSH and nix-darwin targets need passwordless sudo. It is newer than snowglobe.

## comin and pull-based deployment

Every tool above pushes from a deploying machine over SSH. [comin](https://github.com/nlewo/comin) works the other way: an agent on each host polls one or more Git remotes, then evaluates, builds and switches the host's own `nixosConfigurations.<hostname>` or `darwinConfigurations.<hostname>`. A `testing-<hostname>` branch deploys a change in test mode. comin can check commit signatures, and it reports when the running kernel differs from the booted one, but it does not reboot.

comin is the better fit when hosts should pull rather than accept SSH from a deployer, such as hosts behind NAT, or when merging to `main` should be the deployment. Each host deploys on its own schedule, so there is no way to stop a rollout at a canary or to deploy a cluster one node at a time, and every host needs the resources to build its own system. snowglobe is push-only for now; a pull mode is on the [roadmap](https://github.com/luizribeiro/snowglobe#roadmap).

## Other tools

- **[NixFleet](https://github.com/arcanesys/nixfleet)** does pull-based rollouts for NixOS and nix-darwin, in canary waves gated on health checks, with automatic rollback. It needs an agent on each host, a control plane, a binary cache and a CI signing key.
- **[Cachix Deploy](https://docs.cachix.org/deploy/)** is a hosted service: agents on NixOS, nix-darwin or home-manager hosts fetch prebuilt systems from a Cachix cache, and each host can roll itself back. It does not order hosts or stop a rollout when one fails.
- **[lollypops](https://github.com/pinpox/lollypops)** generates go-task tasks that copy your flake to each host, upload secrets from a local command such as `pass`, and run `nixos-rebuild` there.
- **[bento](https://github.com/rapenne-s/bento)** has a central server build the configurations, and each host pulls and applies its own on a timer. It can reboot when the kernel or initrd changed. It has been inactive since December 2024.
- **[nixos-anywhere](https://github.com/nix-community/nixos-anywhere)** installs NixOS on a machine over SSH, wiping its disk. It is for the first install; afterwards you deploy with one of the tools above.

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

snowglobe is the better fit when you deploy a fleet that should change gradually: low-risk hosts first, a cluster one node at a time, a host drained before it reboots, and reboots only when a change needs them. Other tools cover parts of this. What snowglobe adds is the combination: hosts build, copy and activate in parallel, except within a serial group; several phases and groups run in one deployment; hooks drain and restore a node around its change; and a host reboots only when the new system needs it, then snow waits for it to come back. It is pre-1.0 and has a small user base.

## Moving between tools

The tools above can coexist with snowglobe, since snowglobe reads ordinary `nixosConfigurations`. If you already use deploy-rs or colmena, [Coming from deploy-rs or colmena](/snowglobe/migrating/) shows how their settings map to snowglobe's.

## Corrections

This comparison was checked against each tool's documentation, but the tools change. If a cell is wrong or out of date, [open an issue](https://github.com/luizribeiro/snowglobe/issues).
