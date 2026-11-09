---
title: "Reboot detection"
group: "Concepts"
weight: 95
description: "Why some NixOS changes need a reboot, how snow detects them, and how it reboots a host and waits for it."
---

Most NixOS changes can be applied to a running system with `switch-to-configuration switch`. Some cannot, and switching to them anyway leaves the host running a mix of the old and the new system. This page explains which changes those are, how `snow` detects them before activating, and what it does when a host needs a reboot. For how to choose a policy or mode per host, see [Control reboots and activation modes](/snowglobe/reboots/).

## Why some changes can't be switched live

A switch replaces the system profile, updates `/etc`, and starts, stops, restarts or reloads systemd units. Everything that only happens at boot stays as it was:

- **A new kernel** does not run until the host boots it. After a switch, the host keeps running the old kernel under the new system, so whatever the new kernel brings, such as a security fix or a driver, is not in effect, and the system that runs no longer matches the one that will boot.
- **A new initrd** only matters at boot, so a switch silently ignores it. Changes to early boot, disk unlocking or the root file system only take effect, and only show whether they work, at the next boot.
- **Some units are risky to restart on a live system.** They set up the host early in boot: udev's sockets, kernel parameters (`systemd-sysctl`), kernel modules (`systemd-modules-load`) and scripted networking. Restarting them applies the new settings on top of a system that was set up with the old ones, and restarting networking can cut the SSH connection the deployment itself is using.

For those changes, the safe way to apply them is to set the new system for the next boot and reboot into it, which is what `nixos-rebuild boot` followed by a reboot does. `snow` detects these cases during the plan step of each host, before any hook runs and before anything is activated.

![A host that must reboot is set to boot the new system, then rebooted](demos/reboot.cast)

## The two checks

Detection runs on the host over SSH, against the new system that the copy step has just put in its store.

### Kernel and initrd

The first check compares the `kernel` and `initrd` links of the booted system, `/run/booted-system`, with those of the new system. `/run/booted-system` is the system the host booted into, not the one it currently runs, so a host that was switched to a new kernel earlier without rebooting still counts as needing a reboot.

If either link differs, a reboot is needed, and the reason names what changed:

```text
kernel linux-6.12.58 → linux-6.12.60, initrd changed
```

A kernel that was rebuilt without a version change shows as `kernel rebuilt (linux-6.12.60)`. This check is a couple of `readlink` calls, so it is fast.

### Services that would restart

If the kernel and initrd are unchanged, `snow` runs `switch-to-configuration dry-activate` for the new system, through the activation helper. `dry-activate` works out what a switch would do without doing it, and lists the units it would stop, restart, start and reload. `snow` reads the units it would stop or restart (a stopped unit is started again after activation, so it restarts too) and checks them against this list:

| Unit | What restarting it affects |
|---|---|
| `audit.service` | the kernel audit daemon |
| `kmod-static-nodes.service` | static device nodes for kernel modules |
| `network-local-commands.service` | commands from `networking.localCommands` |
| `network-setup.service` | scripted network configuration |
| `syslog.socket` | the syslog socket |
| `systemd-modules-load.service` | loading kernel modules from `boot.kernelModules` |
| `systemd-oomd.socket` | the userspace OOM killer |
| `systemd-sysctl.service` | kernel parameters from `boot.kernel.sysctl` |
| `systemd-udevd-control.socket` | udev's control socket |
| `systemd-udevd-kernel.socket` | udev's kernel event socket |

If any of them would restart, a reboot is needed, and the reason says which one: `systemd-sysctl.service would restart`. The list is fixed in snowglobe's source ([`snowlib/src/backends/reboot/mod.rs`](https://github.com/luizribeiro/snowglobe/blob/main/snowlib/src/backends/reboot/mod.rs)) and cannot be changed per host.

`dry-activate` evaluates the switch plan for the whole system, which can take a while on a large configuration, so `snow` skips it when the first check already found a reason, and gives it up to five minutes. If `dry-activate` fails, the host fails at plan. Without its output, `snow` cannot tell whether the change is safe to switch, and it does not guess that it is.

## What the result changes

The checks only run when the outcome matters:

| Situation | Detection | Result |
|---|---|---|
| `auto` policy (the default), plain `snow deploy` | runs | switch if no reboot is needed; otherwise boot and reboot |
| `never` policy | skipped | set the new system for the next boot, never reboot |
| `always` policy | skipped | set the new system for the next boot and reboot |
| `--switch`, `auto` policy | runs | switch if no reboot is needed; otherwise the host fails at plan |
| `--switch --force` | runs, result ignored | switch, whatever the policy and the checks say; never reboot |
| `--boot`, `--test`, `--dry-run` | skipped | the requested mode, never a reboot |
| `--skip-reboot` | runs | as the default, but stop before rebooting |
| nix-darwin host | skipped | switch |

A host that already runs and boots the built system is skipped before planning, so even the `always` policy does not reboot an unchanged host.

### Why a policy wins over --switch

With `--switch`, a host whose policy is `never` or `always` is still set to boot the new system rather than switched, and an `always` host is still rebooted. A host with the `auto` policy that needs a reboot is not switched either: it fails at plan with the reason, before its hooks run, so it is never drained for a deployment that cannot happen.

A reboot policy records something about the host that is easy to forget at the moment you type a command: that a host serving others should not have its network restarted under it, or that a workstation must not reboot while someone is using it. Letting a command-line flag override it by default would make the policy only as reliable as your memory. `--switch --force` is the explicit way to switch anyway; the old kernel keeps running until the next reboot.

## Waiting for the host

When a reboot is needed, `snow` activates the new system with the `boot` mode, so it becomes the default boot entry without being started now. Then it:

1. reads the host's boot ID from `/proc/sys/kernel/random/boot_id`;
2. asks the host to reboot through `sudo snowglobe-activate reboot`, falling back to `sudo systemctl reboot` on hosts without the helper (an SSH connection that drops at this point is expected);
3. polls the host every five seconds until it answers with a different boot ID.

Comparing boot IDs, rather than waiting for SSH to answer again, catches a host that was slow to go down: it may still answer on the old boot for a few seconds after the reboot command. A changed boot ID proves the host really rebooted.

The wait ends after `--reboot-timeout` seconds, 600 by default. If the host has not come back by then, it fails at the reboot step with one of two messages:

- `host did not come back online within 600s (last error: …)`, with the last SSH error, when the host stopped answering, for example because it failed to boot;
- `boot ID is unchanged, so the host never rebooted within 600s`, when it kept answering on the same boot.

Both messages are prefixed with `Deployment succeeded but reboot failed:`, because the new system is installed and set to boot, so the next successful boot runs it. If the reboot command itself is refused, for example because `sudo` asks for a password, the host fails at the reboot step straight away instead of waiting. The host's `on-failure` hooks run as for any other failure, and later phases do not.

![A rebooted host never comes back](demos/reboot-deadline.cast)

After a successful reboot, `post-reboot` hooks run, then `post-deploy` hooks. A `post-reboot` hook is the place to wait until the host's services are actually ready, for example `kubectl wait --for=condition=Ready`, before the next host in a serial group takes its turn.

## nix-darwin

nix-darwin has no equivalent of a boot-time system switch: its activation script can only switch. `snow` therefore never runs reboot detection on nix-darwin hosts, ignores their reboot policy, and never reboots them. `post-reboot` hooks never run on a Mac. If a macOS update needs a restart, that is outside what `snow` manages. See [Deploy nix-darwin hosts](/snowglobe/darwin/).

## The local machine

`snow rebuild` runs the same two checks on the machine you are on. Instead of rebooting on its own, it asks what to do when a switch needs a reboot: reboot now, use the new system from the next boot, switch anyway, or cancel. See [Rebuild the local machine](/snowglobe/rebuild/).
