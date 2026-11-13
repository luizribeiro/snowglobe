---
title: snowglobe
description: Deploy NixOS and nix-darwin hosts, in phases.
groups: ["Get started", "Guides", "Concepts", "Reference", "Project"]
---

snowglobe deploys the NixOS and nix-darwin hosts defined in a flake, over
SSH, from the machine you run it on. Its command-line tool, `snow`, builds
each host's system, copies it to the host, activates it, and reboots the host
when the new system needs a reboot. Hosts roll out in phases, and a failure
stops the deployment before later phases are touched.

![Twelve hosts in four phases switch to a new system](demos/switch.cast)

It is meant for people who manage several NixOS or nix-darwin machines from
one flake, from a handful of servers to a mixed fleet of Linux machines and
Macs, and want deployments that respect the order and pace their hosts
need.

Each recording in this documentation is a NixOS VM test: `snow` runs
against real virtual machines, and the test checks that the hosts end up
the way the recording shows.

snowglobe is pre-1.0. The CLI, the module options and the flake interface may
change between minor releases, so read the [changelog](/snowglobe/changelog/)
before upgrading.

## Features

- **Phases.** Give hosts a phase, and `snow deploy` deploys the phases in
  order, stopping at the first failure. Low-risk hosts go first.
  See [Roll out in phases](/snowglobe/phases/).
- **Serial groups.** Hosts in the same serial group take turns, so the nodes
  of a cluster are drained, switched and rebooted one at a time.
  See [Deploy a cluster one node at a time](/snowglobe/cluster/).
- **Reboot detection.** Before activating, `snow` checks whether the new
  system needs a reboot (a new kernel or initrd, or a service such as udev
  that should not be restarted live), and per-host reboot policies decide
  what happens then. See [Control reboots and activation
  modes](/snowglobe/reboots/) and [Reboot detection](/snowglobe/reboot-detection/).
- **Hooks.** Run commands before and after a host deploys, after it reboots,
  and when it fails: drain a node, take a host out of a load balancer, check
  its health. See [Write deployment hooks](/snowglobe/hooks/) and [Gate a
  rollout on health checks](/snowglobe/health-checks/).
- **NixOS and nix-darwin together.** Macs running nix-darwin deploy in the
  same deployment as NixOS hosts. See [Deploy nix-darwin hosts](/snowglobe/darwin/).
- **No extra flake output.** `snow` reads your existing
  `nixosConfigurations` and `darwinConfigurations`; per-host settings live
  in each host's own configuration.
- **A readable sudo rule.** The deploy user gets passwordless sudo for one
  helper, `snowglobe-activate`, instead of `NOPASSWD: ALL`; the deploy user
  is still effectively root. See [Security model](/snowglobe/security/).
- **Local rebuilds.** `snow rebuild` works like `nixos-rebuild` or
  `darwin-rebuild` on the machine you are on, with the same reboot check.
  See [Rebuild the local machine](/snowglobe/rebuild/).

## Key terms

> **Host**: one entry in your flake's `nixosConfigurations` or
> `darwinConfigurations`, and the machine it describes.
>
> **Deployment**: one run of `snow deploy`. It builds each selected host, copies the
> result over SSH, activates it, and reboots the host if needed.
>
> **Phase**: a named group of hosts, set with `snowglobe.deploy.phase`. Phases
> deploy one after another; hosts within a phase deploy in parallel.
>
> **Serial group**: a set of hosts, set with `snowglobe.deploy.serialGroup`,
> that take turns within a phase: one host at a time runs its hooks,
> activates and, if needed, reboots.
>
> **Activation helper**: `snowglobe-activate`, a small program the snowglobe
> module installs on each host. It is the only command the deploy user may
> run as root without a password.
>
> **Reboot policy**: a host's rule for reboots, set with
> `snowglobe.deploy.rebootPolicy`: `auto` (reboot only when needed), `never`
> or `always`.
>
> **Hook**: a command, defined in a host's configuration, that `snow` runs
> on your machine or on the host at a fixed point in that host's deployment.

## Where to go next

- New to snowglobe? [Install it](/snowglobe/installation/), then follow the
  [tutorial](/snowglobe/tutorial/) to deploy to your first host.
- Choosing a tool? [Comparison with other tools](/snowglobe/comparison/)
  sets snowglobe against `nixos-rebuild`, deploy-rs, colmena, morph, Clan,
  devenv Machines and pull-based tools, including what snowglobe does not do.
- Already using deploy-rs or colmena? See [Coming from deploy-rs or
  colmena](/snowglobe/migrating/).
- Setting up a real fleet? Start with [Set up a deploy user](/snowglobe/deploy-user/)
  and [Connect to your hosts](/snowglobe/connect/), then the other guides.
- To understand what happens during a deployment, read [How a deployment
  works](/snowglobe/how-deploy-works/).
