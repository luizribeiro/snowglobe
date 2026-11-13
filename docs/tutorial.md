---
title: "Tutorial: your first deployment"
group: "Get started"
weight: 20
description: "Set up one NixOS host for snowglobe, deploy to it with snow deploy, then add a second host in a later phase."
---

In this tutorial you prepare an existing NixOS host for snowglobe, preview a
change with `snow deploy --dry-run`, deploy it with `snow deploy`, and then add a
second host that deploys in a later phase. At the end, a single `snow deploy`
in your flake's directory deploys both hosts in order, skipping any host that
already runs its new system.

The examples use two hosts called `web1` and `web2`, deployed as a user
called `deploy`. Use your own host and user names throughout.

## Before you start

You need `snow` installed on the machine you deploy from; see
[Installation](/snowglobe/installation/). You also need a NixOS host that
already builds from your flake. Check each of the following before you go
on; every step below assumes they work.

1. **You can log in as the deploy user with a key, without any prompt.**
   `snow` runs `ssh` in batch mode, as this command does:

   ```bash
   ssh -o BatchMode=yes deploy@web1 true
   ```

   It must exit without printing anything and without asking for a password
   or a passphrase. If your key has a passphrase, load it into your SSH agent
   first with `ssh-add`. If the `deploy` user does not exist yet, create it
   first: [Set up a deploy user](/snowglobe/deploy-user/) shows how.

2. **The host key is already in `known_hosts`.** In batch mode, `ssh`
   refuses to connect to a host it has never seen. If the command above
   failed with `Host key verification failed`, connect once interactively
   with `ssh deploy@web1`, check the fingerprint, and accept it.

3. **The name you connect to is the one `snow` will use.** `snow` connects
   to the host's `networking.hostName`, followed by `.` and
   `networking.domain` when that is set, not to the name of the flake
   attribute. Check what that is:

   ```bash
   nix eval .#nixosConfigurations.web1.config.networking.hostName
   nix eval .#nixosConfigurations.web1.config.networking.domain
   ```

   If the domain is `null`, `snow` connects to `web1`; if it is
   `"example.com"`, it connects to `web1.example.com`, and that is the name
   to test with `ssh`. A `Host` entry in `~/.ssh/config` can map the name to
   an address; see [Connect to your hosts](/snowglobe/connect/).

4. **The host already builds and switches from your flake.** For example,
   this works from your machine:

   ```bash
   nixos-rebuild switch --flake .#web1 --target-host admin@web1 --sudo --ask-sudo-password
   ```

   where `admin` is any account that can already run `sudo` on `web1`, or,
   on `web1` itself from a checkout of the flake,
   `sudo nixos-rebuild switch --flake .#web1`.

   `snow` uses the same `nixosConfigurations.web1` output, so if
   `nixos-rebuild` cannot build it, `snow` cannot either.

5. **Your machine can build the host's system.** `snow` builds on the
   machine you run it on. If `web1` has a different architecture from your
   machine, set up a remote builder or emulation first; see [Build for other
   architectures](/snowglobe/architectures/).

If you do not have a NixOS host yet, install one first. The [NixOS
manual's installation chapter](https://nixos.org/manual/nixos/stable/#ch-installation)
covers installing from the ISO, and
[nixos-anywhere](https://github.com/nix-community/nixos-anywhere) installs
NixOS over SSH onto a machine that runs another Linux.

## Step 1: Add snowglobe to your flake

Add snowglobe as an input and import its NixOS module into the host. Hosts
will share some snowglobe settings, so put them in a common module from the
start:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    snowglobe.url = "github:luizribeiro/snowglobe";
    snowglobe.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { nixpkgs, snowglobe, ... }: {
    nixosConfigurations.web1 = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        snowglobe.nixosModules.default
        ./hosts/common.nix
        ./hosts/web1/configuration.nix
      ];
    };
  };
}
```

Keep whatever `nixpkgs` input and host modules you already have; the two
additions are the `snowglobe` input and the `snowglobe.nixosModules.default`
line. The `follows` line keeps a single nixpkgs in your lock file; the module
uses each host's own nixpkgs either way. Then update the lock file so it records the new input:

```bash
nix flake lock
```

If your flake is a Git repository, add new files with `git add` before you
build. Nix only sees files that Git tracks.

## Step 2: Let the deploy user activate the host

Create `hosts/common.nix` with the two settings the deploy user needs:

```nix
{
  snowglobe.deploy.allowedUsers = [ "deploy" ];
  nix.settings.trusted-users = [ "deploy" ];
}
```

The two lines do different jobs:

- `snowglobe.deploy.allowedUsers` lets `deploy` run the activation helper,
  `snowglobe-activate`, through `sudo` without a password. The helper is the
  only command this rule covers: it activates a system from the Nix store,
  or reboots the host.
- `nix.settings.trusted-users` lets `deploy` add the paths `snow` copies to
  the host's Nix store. `snow deploy` copies with `nix copy`, and the Nix
  daemon accepts unsigned paths only from trusted users.

Both make `deploy` effectively root on the host, so guard its SSH key like a
root key. [Set up a deploy user](/snowglobe/deploy-user/) explains why, and
the alternatives; [Security model](/snowglobe/security/) covers the
details.

Finally, tell `ssh` which user to log in as. `snow` uses your SSH
configuration, so add an entry to `~/.ssh/config` on the deploying machine:

```text
Host web1 web2
  User deploy
```

The `Host` line must match the name `snow` connects to: if your hosts set
`networking.domain`, write the full names, such as `web1.example.com`, or
a pattern such as `*.example.com`. Without a `User` entry, `ssh` logs in as
your local user name. You can also pass `--user deploy` to each
`snow deploy` instead. Check the entry:

```bash
ssh -o BatchMode=yes web1 id -un
```

It should print `deploy`.

## Step 3: Apply the configuration once by hand

`snow` cannot use the activation helper until the host runs a system that
contains it, so apply this configuration once the way you already deploy.
For example, from your machine:

```bash
nixos-rebuild switch --flake .#web1 --target-host admin@web1 --sudo --ask-sudo-password
```

Use an account that can already run `sudo` on `web1`; `--ask-sudo-password`
prompts for its password. The deploy user cannot do this step yet, because
its passwordless rule only exists once this system is active. Older `nixos-rebuild` versions spell
`--sudo` as `--use-remote-sudo`. Running
`sudo nixos-rebuild switch --flake .#web1` on `web1` works too.

Then confirm, from the deploying machine, that the deploy user can run the
helper without a password:

```bash
ssh web1 sudo -n snowglobe-activate
```

The helper prints its usage and exits with an error, because it was given
no arguments:

```text
Usage: snowglobe-activate <store-path> <mode>
       snowglobe-activate reboot
Modes: switch, boot, test, dry-activate
```

That error is the expected result. If you see `sudo: a password is
required` instead, the new system is not active yet or `deploy` is not in
`allowedUsers`. If you see `command not found`, the module is not imported.

From now on you do not need `nixos-rebuild` for this host.

## Step 4: List your hosts

In the flake's directory, or any directory below it, run:

```bash
snow list
```

`snow list` reads `nixosConfigurations` and `darwinConfigurations` and prints
one row per host, with its phase and tags. You have not set either yet, so
both show `-`:

![snow list shows web1, with no phase or tags yet](demos/tutorial-list.cast)

If your flake defines more hosts, they are listed too. `snow list` only
evaluates the flake; it does not connect to any host.

## Step 5: Preview a change with a dry run

Make a change to `web1` so there is something to deploy. For example, enable
the Prometheus node exporter, which serves the host's metrics on port 9100,
in `hosts/web1/configuration.nix`:

```nix
{
  services.prometheus.exporters.node = {
    enable = true;
    openFirewall = true;
  };
}
```

Then preview it:

```bash
snow deploy --dry-run web1
```

A dry run builds the new system, copies it to `web1`, and asks the host
which units a switch would stop, start or restart, without changing what
the host runs or boots. [Activation modes](/snowglobe/reboots/#--dry-run-show-what-would-change)
describes exactly what it does and skips.

![snow deploy --dry-run builds web1's new system and asks it what would change](demos/tutorial-dry-run.cast)

While the deployment runs, `snow` shows a live board in place of the report, with
each host under the step it is in. The `(dry-run)` label says nothing was
activated; the last line only counts hosts that finished without an error.

The first line names the log file. Every `snow` command run in a terminal
prints it. The output of `dry-activate` is in that log file; open it to see
what the switch would do. Among the log lines is this one:

```text
would reload the following units: dbus-broker.service, firewall.service
```

The firewall reloads to open port 9100. The exporter's own service is not
listed: it is new, and `dry-activate` only reports changes to units the host
already has.

## Step 6: Deploy

Now deploy the change for real:

```bash
snow deploy web1
```

This time, after copying, `snow` plans the activation: it compares the
kernel and initrd `web1` booted with the new ones, and runs `dry-activate`
to see whether a service that should not be restarted on a live system,
such as udev, would restart. Enabling the exporter needs neither, so `snow`
switches `web1` to the new system, as `nixos-rebuild switch` would:

![snow deploy switches web1 to its new system](demos/tutorial-deploy.cast)

Check the result on the host, for example with
`ssh web1 systemctl is-active prometheus-node-exporter`, which should print
`active`.

Some changes that look as small do need a reboot. Enabling nginx, for
example, loads a new kernel module, which restarts
`systemd-modules-load.service`. For such a change, `snow` sets the new
system as the system for the next boot, reboots the host, and waits for it
to come back. The report then reads `(boot, rebooted)`, as in this
recording of `web1`:

![A host that must reboot is set to boot the new system, then rebooted](demos/reboot.cast)

[Control reboots and activation modes](/snowglobe/reboots/) explains how to
change this per host, for example to never reboot a host automatically.

> **If something goes wrong.** The report names the step the host failed
> at and the error line that matters; the full output is in the log file.
> One case surprises people: if any systemd unit is failing after the
> switch, even one this change did not touch, the host fails at activate
> although the new system is active. See [A host fails with "activated,
> but … is failing"](/snowglobe/troubleshooting/#a-host-fails-with-activated-but--is-failing)
> and the other entries in [Troubleshooting](/snowglobe/troubleshooting/).

## Step 7: Deploy again

Run the same command again without changing anything:

```bash
snow deploy web1
```

`snow` builds `web1`, sees that the host already runs and boots that exact
system, and stops there:

![web1 already runs its new system, so snow leaves it alone](demos/tutorial-unchanged.cast)

Unchanged hosts are not copied to, activated or rebooted, and their hooks do
not run, so running `snow deploy` for all hosts is cheap when only some have
changed. To deploy a host anyway, for example to retry an activation that
partly failed, pass `--redeploy`.

This is the whole loop: edit the flake, optionally preview with
`--dry-run`, and deploy.

## Step 8: Add a second host in a later phase

Phases let you deploy hosts in order: all hosts in the first phase deploy,
and the next phase starts only if none of them failed. Here `web1` will go
first and `web2` second, so a change that breaks `web1` never reaches
`web2`.

Before adding `web2`, go through the [checklist](#before-you-start) for it:
key login as `deploy`, host key in `known_hosts`, and a configuration that
builds. Then add it to the flake with the same modules:

```nix
{
  outputs = { nixpkgs, snowglobe, ... }: {
    nixosConfigurations.web1 = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        snowglobe.nixosModules.default
        ./hosts/common.nix
        ./hosts/web1/configuration.nix
      ];
    };

    nixosConfigurations.web2 = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        snowglobe.nixosModules.default
        ./hosts/common.nix
        ./hosts/web2/configuration.nix
      ];
    };
  };
}
```

Give each host a phase, in its own configuration. In
`hosts/web1/configuration.nix`:

```nix
{
  snowglobe.deploy.phase = "p1";
}
```

and in `hosts/web2/configuration.nix`:

```nix
{
  snowglobe.deploy.phase = "p2";
}
```

Phases run in name order, with numbers compared by value, so `p2` comes
before `p10`. Hosts without a phase deploy after all the others.

Like `web1` in step 3, `web2` needs one manual switch so that the helper and
the sudo rule exist on it:

```bash
nixos-rebuild switch --flake .#web2 --target-host admin@web2 --sudo --ask-sudo-password
```

Deploy settings such as `phase` are read by `snow` from the configuration;
they do not change the system that is built. So if you deploy now, both hosts
report `unchanged`. Make a change that affects both, for example add
a package to `environment.systemPackages` in `hosts/common.nix`. Then check
the phases with `snow list`, and deploy without naming any host:

```bash
snow list
snow deploy
```

`snow deploy` with no host names deploys every host in the flake, except
hosts in the special phase `never`. It first lists the phases it is about
to run, so you can stop it with Ctrl-C if a host you did not expect shows
up, then reports each phase as it ends:

![snow list shows both hosts and their phases, then snow deploy deploys them in two phases](demos/tutorial-phases.cast)

If `web1` had failed, phase `p2` would have been skipped, `web2` left
untouched, and `snow deploy` would exit with a non-zero status.

You can still deploy one host by name, as before, or select hosts with
`--phase`, `--tag` or a Nix expression; `snow deploy --help` lists the
options.

## Next steps

- [Roll out in phases](/snowglobe/phases/): plan phases by risk, and handle
  hosts that may be offline or that you only deploy on purpose.
- [Gate a rollout on health checks](/snowglobe/health-checks/): check a
  host's health after it deploys, so a broken change stops in the first phase.
- [Write deployment hooks](/snowglobe/hooks/): drain a node or leave a load
  balancer around each host's deployment.
- [Control reboots and activation modes](/snowglobe/reboots/): reboot
  policies, `--boot`, `--switch` and `--test`.
- [Deploy nix-darwin hosts](/snowglobe/darwin/): add Macs to the same flake.
- [How a deployment works](/snowglobe/how-deploy-works/): every step of a deployment, and
  why it is in that order.
- [Module options](/snowglobe/options/): every `snowglobe.*` option.
