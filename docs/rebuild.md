---
title: "Rebuild the local machine"
group: "Guides"
weight: 78
description: "Use snow rebuild to build and activate the configuration of the NixOS or macOS machine you are on."
---

`snow rebuild` builds the configuration of the machine you are working on and activates it, like `nixos-rebuild switch --flake .` or `darwin-rebuild switch --flake .`. This guide explains what it builds, how it uses sudo, and what it does when the new system needs a reboot.

It is meant for the machine you are sitting at, such as a workstation or a laptop. To deploy other hosts, use `snow deploy`.

## Prerequisites

- A flake whose `nixosConfigurations` or `darwinConfigurations` includes this machine.
- An account that can use sudo for any command, or a root shell. Unlike `snow deploy`, `snow rebuild` doesn't go through the activation helper; it runs the activation with your own sudo rights, so it works whether or not the machine imports the snowglobe module.
- A configuration named after this machine, or the `--host` flag to name it (see [Which configuration it builds](#which-configuration-it-builds)).

## Rebuild

From the flake's directory, or any directory below it:

```bash
snow rebuild
```

snow looks for `flake.nix` in the current directory and its parents. To use a flake elsewhere, pass `-f /path/to/flake` or set `FLAKE_PATH`.

A rebuild goes through these steps:

1. **sudo.** If sudo needs your password, snow asks for it once, before anything else, with a line saying the host needs sudo. Asking first means a long build doesn't stop halfway to wait for you.
2. **Build.** snow builds the system for this machine.
3. **Reboot check.** For a switch on NixOS, snow checks whether the new system needs a reboot (see [below](#when-the-new-system-needs-a-reboot)).
4. **Activate.** snow records the new system in the system profile, which makes it a generation you can roll back to, then runs its activation: `switch-to-configuration` on NixOS, which also makes it the default boot entry, or the `activate` script on macOS.

After asking for sudo if it needs to, snow shows a single host line, as in `snow deploy`, ending with the result and how long it took:

![snow rebuild asks for the sudo password once, before it starts](demos/rebuild-sudo.cast)

In a terminal, the output of the build and the activation doesn't appear on screen. snow writes it to a full log, `/tmp/snow-<timestamp>.log`, and prints that path when it starts. Look there when something fails, or to see what an activation did.

## Which configuration it builds

By default, the configuration is named after this machine:

- On NixOS, the contents of `/etc/hostname`, so `nixosConfigurations.<hostname>`. NixOS writes that file from `networking.hostName`.
- On macOS, the output of `hostname -s`, so `darwinConfigurations.<hostname>`.

Whether to look in `nixosConfigurations` or `darwinConfigurations` depends on the operating system snow runs on. If the configuration has a different name, select it with `--host`:

```bash
snow rebuild --host your-mac
```

## sudo

snow checks whether sudo already runs commands without asking, for example because you used sudo a moment ago or have a `NOPASSWD` rule. If it doesn't, snow pauses its output and runs `sudo -v` so that sudo can ask for your password on the terminal. If you run snow as root, it skips sudo entirely.

While the build runs, snow refreshes sudo's timestamp every minute in the background. sudo forgets a password after five minutes by default, and without the refresh a build longer than that would end at a second password prompt. Every command snow then runs as root uses `sudo -n`, which never prompts, so nothing can ask for a password underneath the progress display.

![snow rebuild keeps sudo from forgetting the password during a long build](demos/rebuild-sudo-keepalive.cast)

If sudo is configured never to remember a password (`timestamp_timeout=0`), snow can't keep it, and sudo may ask again at each step that needs root.

If sudo needs a password and standard input isn't a terminal, for example in a script or a CI job, `snow rebuild` fails before building, saying there is no terminal to ask on. Either run it from a terminal, or let the user run sudo without a password.

## When the new system needs a reboot

Some changes can't take effect on a running system: a new kernel or initrd, or a change that would restart a service that is unsafe to restart live, such as udev or the network setup. On NixOS, before switching, snow runs the same check that `snow deploy` uses: it compares the booted kernel and initrd with the new ones and, if they match, runs the new system's `dry-activate` to see which services would restart. [Reboot detection](/snowglobe/reboot-detection/) explains the check.

If no reboot is needed, snow switches. If one is, it lists the reasons and asks what to do, as the recordings below show:

- **Reboot into it now** sets the new system as the boot default without activating it, then reboots. The result line reads `(boot, rebooting)`.
- **Use it from the next boot** sets the new system as the boot default and leaves the running system alone, like `--boot`. The result line reads `(boot)`.
- **Switch now, reboot later** shows a warning that, until you reboot, the running system mixes old and new parts, and asks you to confirm (the default is no). If you confirm, snow switches and the result line reads `(switch, reboot to finish)`.
- **Cancel** changes nothing. The system was built but isn't activated or set for the next boot, and the result line reads `(cancelled)`. snow exits with status 0.

![snow rebuild sets the new system to start on the next boot](demos/rebuild-boot.cast)

![snow rebuild reboots into the new system](demos/rebuild-reboot.cast)

![snow rebuild warns before switching without a reboot, and is cancelled](demos/rebuild-cancel.cast)

Two flags answer the question in advance, which is useful when you already know what you want:

```bash
snow rebuild --skip-reboot   # if a reboot is needed, use the new system from the next boot
snow rebuild --force         # if a reboot is needed, switch now anyway
```

Both only matter when a switch needs a reboot; otherwise snow switches as usual. A host's `snowglobe.deploy.rebootPolicy` doesn't apply to `snow rebuild`: you make that choice at the menu, or with these flags.

## Other modes

The same activation modes as `snow deploy` are available on NixOS:

```bash
snow rebuild --boot      # set the new system for the next boot, don't activate it
snow rebuild --test      # activate now, but boot the old system next time
snow rebuild --dry-run   # run dry-activate, change nothing
```

`--boot`, `--test` and `--dry-run` skip the reboot check, since none of them switches the running system permanently. `--test` and `--dry-run` don't add a generation to the system profile. The list of units a dry run would start, stop or restart is in the log file, not on screen; the screen only shows the result line, `(dry-run)`. On macOS, only switching is supported, and these flags fail before anything is built. Run `snow rebuild --help` for every flag.

## On macOS

On a Mac, `snow rebuild` builds `darwinConfigurations.<hostname -s>`, records it in the system profile and runs its `activate` script. There is no reboot check: snow assumes a nix-darwin system never needs one, so the menu never appears. See [Deploy nix-darwin hosts](/snowglobe/darwin/) for setting up Macs as deploy targets.

## What it doesn't do

`snow rebuild` activates one machine, the one it runs on. It deliberately leaves out the parts of a deployment that coordinate several hosts or need to read the host's snowglobe settings:

- **No hooks.** `snowglobe.hooks` are for `snow deploy`, and `snow rebuild` runs none of them.
- **No copy or SSH.** Everything happens locally.
- **No phases, tags, serial groups or reboot policy.** These only order or select hosts within a deployment.
- **`ignoredFailedUnits` isn't read.** A NixOS switch fails when any unit is failing afterwards. `snow rebuild` then fails with the same message as a deployment, saying the new system is active and naming the failing units, but it doesn't consult `snowglobe.deploy.ignoredFailedUnits`, since reading it would mean another evaluation of the host. The new system is active in that case; fix or stop the failing unit.
- **It doesn't skip an unchanged system.** Each run activates the system it built, even when it is already running.

## When to use it instead of nixos-rebuild

`snow rebuild` and `nixos-rebuild switch --flake .` end in the same place: a new system profile generation, activated with `switch-to-configuration`. Use `snow rebuild` when you want:

- to be told before a switch that the new system needs a reboot, and to choose then whether to reboot, wait for the next boot, or switch anyway;
- one sudo prompt at the start, kept alive through a long build;
- the same output and log as `snow deploy`, and the same command on NixOS and macOS.

Use `nixos-rebuild` or `darwin-rebuild` for what `snow rebuild` doesn't cover: configurations without flakes, building or deploying with `--build-host` and `--target-host`, `nixos-rebuild build-vm`, listing generations, or `--rollback`. Both tools write the same system profile, so you can switch between them freely. To undo a rebuild, see [Roll back a bad deployment](/snowglobe/rollback/), which applies to local rebuilds too.
