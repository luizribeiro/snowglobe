---
title: "Control reboots and activation modes"
group: "Guides"
weight: 55
description: "Choose per host whether snow may reboot it, and pick how a single deployment activates: switch, boot, test or dry run."
---

This guide explains how to decide, per host, whether `snow deploy` may reboot it, and how to override the activation for a single deployment with `--switch`, `--boot`, `--test`, `--dry-run`, `--force` and `--skip-reboot`. It covers NixOS hosts; nix-darwin hosts are never rebooted and only switch (see [Deploy nix-darwin hosts](/snowglobe/darwin/)).

## What snow does by default

With no flags, snow decides for each NixOS host whether the new system can be switched to live. Before activating, it checks whether the new kernel or initrd differs from the one the host booted with, and whether activation would restart one of a few services that are not safe to restart on a running system (udev, sysctl and the like). [Reboot detection](/snowglobe/reboot-detection/) explains these checks in detail.

- If no reboot is needed, snow **switches**: the new system is active immediately and becomes the default boot entry.
- If a reboot is needed, snow sets the new system as the default boot entry **without activating it**, reboots the host, and waits for it to come back on a new boot.

This is what you want for most hosts. A per-host policy changes it when you need something else.

[![A host that must reboot is set to boot the new system, then rebooted](/snowglobe/reboot.svg)](https://asciinema.org/a/CRvXGdOvXPBkkiDV)

## Reboot policies

Set a policy in the host's configuration:

```nix
{
  snowglobe.deploy.rebootPolicy = "auto";  # the default; or "never" or "always"
}
```

| Policy | Activation | Reboot |
|---|---|---|
| `auto` | switch, or boot when a reboot is needed | only when needed |
| `never` | always boot, even when a switch would be safe | never; you reboot when it suits you |
| `always` | always boot | after every deployment |

### When to pick each

- **`auto`** suits most servers and appliances: web servers, file servers, small services. Changes apply right away, and the host reboots only for a new kernel or another change that needs one.
- **`never`** suits hosts where an unplanned reboot is worse than a delayed change: a workstation someone is using, a database you fail over by hand, a hypervisor with VMs on it. Note that `never` means the host never switches live either. Every change, even a one-line config edit, takes effect at the next boot. The report marks such a host `(boot)`, and the change waits until you reboot it.
- **`always`** suits hosts where a fresh boot is the only state you trust, or where you want every deployment to prove that the host still boots, such as members of a cluster that you drain anyway. Hosts that are already up to date are still skipped, so `always` does not reboot a host that had nothing to deploy.

Policies are set per host, so the usual pattern is to set a default in a shared module and override it on the few hosts that differ:

```nix
# modules/common.nix
{ lib, ... }:
{
  snowglobe.deploy.rebootPolicy = lib.mkDefault "auto";
}
```

```nix
# hosts/db1.nix
{
  snowglobe.deploy.rebootPolicy = "never";
}
```

To check what a host ends up with:

```bash
nix eval .#nixosConfigurations.db1.config.snowglobe.deploy.rebootPolicy
```

## A policy wins over flags

A host's policy is a statement about that host, so it overrides the flags of a deployment. In particular, `--switch` does not switch a host whose policy says otherwise:

- with `rebootPolicy = "never"`, `--switch` sets the new system for the next boot and does not reboot;
- with `rebootPolicy = "always"`, `--switch` sets it for the next boot and reboots.

The point is safety: a host that must never be switched live isn't, even if you type `snow deploy --switch` for the whole fleet. `--switch --force` is the explicit override, described below.

`--boot`, `--test` and `--dry-run` are different: they ask for a specific, non-rebooting mode, and they apply to every host regardless of its policy.

## Activation modes

Plain `snow deploy` is the right choice most of the time. These flags change the activation for one deployment. Only one of `--switch`, `--boot`, `--test` and `--dry-run` can be given at a time.

### `--switch`: switch now, never reboot

```bash
snow deploy --switch web1 web2
```

Every `auto` host is switched live. A host that would need a reboot is not switched and not rebooted: it fails at the plan step, before its `pre-deploy` hooks run (its `on-failure` hooks do run), and the error says why (for example `kernel linux-6.12.58 → linux-6.12.60`). Hosts with a `never` or `always` policy follow their policy, as described above.

Use `--switch` when you want changes live now and would rather be told about a host that needs a reboot than have it rebooted.

### `--switch --force`: switch even if a reboot is needed

```bash
snow deploy --switch --force web1
```

Switches every host live, ignoring both reboot detection and reboot policies, and reboots none of them. A host with a new kernel keeps running the old kernel until its next reboot, and a critical service may be restarted on the running system. `--force` is only accepted together with `--switch`.

### `--boot`: set for the next boot, never reboot

```bash
snow deploy --boot db1
```

Like `nixos-rebuild boot`: every host gets the new system as its default boot entry, but nothing is activated and nothing is rebooted, whatever the policy. The report marks such hosts `(boot)`. Use it to stage a change and reboot later, during a maintenance window.

[![snow deploy --boot sets the new system for the next boot, without rebooting](/snowglobe/deploy-boot.svg)](https://asciinema.org/a/8BSwYPCHAGjHvwxO)

Remember that `post-deploy` [hooks](/snowglobe/hooks/) still run after a `--boot` activation, while the host is still running the old system.

### `--test`: activate until the next reboot

```bash
snow deploy --test nas1
```

Like `nixos-rebuild test`: the new system is activated now, but it is not recorded as a new generation and does not become the default boot entry. Rebooting the host returns it to the previous system. This makes `--test` useful for risky changes, such as firewall or network settings, on a host you can power-cycle: if the change cuts you off, a reboot undoes it. `--test` does no reboot detection and never reboots.

### `--dry-run`: show what would change

```bash
snow deploy --dry-run
```

Builds each host, copies its closure, and runs the activation in dry-activate mode, which lists the units that would be started, stopped, restarted or reloaded, without changing anything. No hooks run. The copy does happen, so a dry run is also a way to pre-load closures onto hosts before a real deployment.

### `--skip-reboot`: deploy as usual, but don't reboot

```bash
snow deploy --skip-reboot
```

Planning works exactly as in a plain deployment, including policies, but snow stops short of rebooting. A host that needs a reboot is set to boot the new system and left running the old one, and an `always` host is not rebooted either. Unlike `--boot`, hosts that don't need a reboot are still switched live. Use it to deploy now and do the reboots yourself.

### Summary

| Command | `auto` host, no reboot needed | `auto` host, reboot needed | `never` host | `always` host |
|---|---|---|---|---|
| `snow deploy` | switch | boot, reboot | boot | boot, reboot |
| `--skip-reboot` | switch | boot | boot | boot |
| `--switch` | switch | fails at plan | boot | boot, reboot |
| `--switch --force` | switch | switch | switch | switch |
| `--boot` | boot | boot | boot | boot |
| `--test` | test | test | test | test |
| `--dry-run` | dry-activate | dry-activate | dry-activate | dry-activate |

Hosts that already run and boot the built system are skipped in every mode, unless you pass `--redeploy`. For nix-darwin hosts, only plain `snow deploy` and `--switch` apply; `--boot`, `--test` and `--dry-run` fail before any system is built when the selection includes a nix-darwin host.

## Waiting for a reboot

When snow reboots a host, it asks the host to reboot through the activation helper and waits until the host answers over SSH with a new boot ID, which proves that it really rebooted. [Reboot detection](/snowglobe/reboot-detection/#waiting-for-the-host) describes the wait step by step.

The wait is bounded by `--reboot-timeout`, 600 seconds by default:

```bash
snow deploy --reboot-timeout 1200 k8s-1
```

Raise it for hosts that boot slowly, for example servers with long firmware checks or hosts that unlock their disks over the network.

If the host is not back in time, it fails at the reboot step. The failure behaves like any other: the host's on-failure hooks run, the rest of its serial group is skipped, and later phases don't run, unless the host is marked optional. The new system was already set as the default boot entry before the reboot, so if the host does come back later, it runs the new system. [Troubleshooting](/snowglobe/troubleshooting/#a-rebooted-host-never-comes-back) lists the messages and what to do about each.

[![A rebooted host never comes back before the deadline](/snowglobe/reboot-deadline.svg)](https://asciinema.org/a/eFy1uI5b9rnfI0jt)

While a host in a [serial group](/snowglobe/cluster/) reboots, it keeps its turn, so the next member of the group waits until this one is back and has run its `post-reboot` and `post-deploy` hooks. A `post-reboot` hook is the place to wait for the host to be fully usable again, for example until its Kubernetes node reports `Ready`; see [Write deployment hooks](/snowglobe/hooks/).

## Related pages

- [Reboot detection](/snowglobe/reboot-detection/): what exactly snow checks before it decides.
- [Roll back a bad deployment](/snowglobe/rollback/): what to do when a host boots into a broken system.
- [Commands](/snowglobe/commands/) and `snow deploy --help` for every flag; [Module options](/snowglobe/options/) for `rebootPolicy` and the other host settings.
