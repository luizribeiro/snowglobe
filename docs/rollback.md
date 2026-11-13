---
title: "Roll back a bad deployment"
group: "Guides"
weight: 75
description: "Return a host to its previous system after a deployment that broke it, from your machine, from the host, or from the boot menu."
---

This guide shows how to return a host to the system it ran before a bad deployment, and how to limit how many hosts a bad change can reach in the first place.

snowglobe has no automatic rollback. If a host activates a broken system, snow reports what failed and leaves the host as it is; it does not switch it back. (deploy-rs's "magic rollback", by contrast, reverts a host by itself when the deployer cannot reach it again after activation.) Rolling back is a deliberate step you take with the tools below. snowglobe is pre-1.0, and this may change, but it is the current behaviour.

> **Before a risky change, deploy it with `--test`.** A `--test` activation is not recorded as a generation or boot entry, so if the change cuts the host off, power-cycling it brings back the previous system, even without a console. Once the change works, deploy it again normally. See [If the host is unreachable after a switch](#if-the-host-is-unreachable-after-a-switch).

## What a deployment leaves behind

Rolling back works because every deployment records a normal NixOS or nix-darwin generation. When snow activates a host in switch or boot mode, the activation helper (`snowglobe-activate`) first points the host's system profile, `/nix/var/nix/profiles/system`, at the new system, then runs the activation. Each deployment therefore adds a generation, exactly as `nixos-rebuild switch` or `darwin-rebuild switch` would, and the previous generations stay in place until they are [garbage-collected](#prune-old-generations).

Two details matter when something goes wrong:

- **The profile is updated before activation runs.** If activation fails half-way, for example because a service fails to start, the new system is still the current generation and the default boot entry. The host is now running a partly activated new system, not the old one.
- **`--test` and `--dry-run` add no generation.** A `snow deploy --test` activates the new system without recording it, so rebooting the host returns it to the previous system. See [Control reboots and activation modes](/snowglobe/reboots/).

You can list a host's generations on the host:

```bash
sudo nixos-rebuild list-generations
# or, with an older nixos-rebuild:
sudo nix-env --list-generations --profile /nix/var/nix/profiles/system
```

## Option 1: deploy the previous commit

If snow can still reach the host, the most predictable rollback is to deploy the configuration you know was good. snow takes a local flake directory, so check the old commit out next to your working copy and point snow at it with `-f`:

```bash
git worktree add ../infra-rollback HEAD~1
snow -f ../infra-rollback deploy web1
```

Replace `HEAD~1` with the last commit you know was good. `-f` points snow at a flake directory, the same way the `SNOW_FLAKE` environment variable does.

snow builds the old system (quickly, if it is still in your Nix store), copies it, and activates it like any other deployment. Reboot detection and the host's [reboot policy](/snowglobe/reboots/#reboot-policies) apply as usual: if the bad deployment changed the kernel, going back changes it again, so the host may reboot. Hooks run too, so a node is drained and uncordoned as on any deployment.

When you are done, remove the worktree:

```bash
git worktree remove ../infra-rollback
```

The alternative is to fix forward: revert the bad commit (`git revert`), and deploy as normal. This keeps your repository and your hosts in agreement, which the other options below do not.

## Option 2: roll back on the host

If you can log in to the host, NixOS can switch back to the previous generation by itself, without your flake:

```bash
sudo nixos-rebuild switch --rollback
```

This points the system profile back at the previous generation, activates it, and makes it the default boot entry. To go back further than one generation, switch the profile to a specific generation and activate it:

```bash
sudo nix-env --profile /nix/var/nix/profiles/system --switch-generation 41
sudo /nix/var/nix/profiles/system/bin/switch-to-configuration switch
```

On a nix-darwin host, the equivalent is:

```bash
sudo darwin-rebuild --list-generations
sudo darwin-rebuild switch --rollback
```

A rollback on the host does not change your repository. The next `snow deploy` from the same commit sees that the host no longer runs the built system and deploys the broken one again. Revert or fix the change before deploying to that host.

## Option 3: the boot menu

If the host no longer boots, or boots but is unreachable over the network, use its console: a physical keyboard and screen, a serial console, IPMI, or your hypervisor's or cloud provider's console. NixOS keeps one boot entry per generation, so at the boot menu (systemd-boot or GRUB) pick the entry before the bad one. The previous system boots exactly as it was.

Booting an older entry is a one-off. The default entry is still the bad generation, so the next reboot would return to it. Once the host is up, make the rollback permanent. If you booted the entry directly before the bad one:

```bash
sudo nixos-rebuild switch --rollback
```

If you booted an older entry, switch the profile to that generation with `--switch-generation`, as shown in Option 2.

Then check that the default boot entry points where you expect:

```bash
readlink /nix/var/nix/profiles/system
```

## If the host is unreachable after a switch

A switch that breaks networking or SSH, such as a wrong firewall rule or interface name, cuts snow off in the middle of the activation. The activation runs over SSH without keepalives, because it may restart the host's network on purpose, so snow can keep waiting on that connection for a long time. If it is clear the host is not coming back, stop snow with Ctrl-C.

The host is then running the new system, which is also its default boot entry. Use the console:

- If you can log in on the console, run `sudo nixos-rebuild switch --rollback`.
- Otherwise, reboot it and choose the previous generation in the boot menu, as above, then make the rollback permanent.

This is why the `--test` deployment at the top of this page is worth it for changes that may cut you off: power-cycling the host undoes it.

Ctrl-C stops only snow: an activation it already started keeps running on the host to the end. snow does not run the host's post-deploy or on-failure hooks then, so a host that a pre-deploy hook drained stays drained. Before exiting, snow lists the hosts it left like that on stderr, and also in its log file when it runs in a terminal:

```text
Interrupted. Host left mid-deploy:
  web1  switch (still running on the host), pre-deploy hooks ran, post-deploy hooks didn't
To finish it: snow deploy --redeploy web1
```

A host is listed while its activation or reboot is still running, when its hooks were cut short, or when its pre-deploy hooks ran and the post-deploy or on-failure hooks that would undo them did not. Hosts snow had not touched yet are left out: those still building, copying, planning or waiting for their turn, and every host of a dry run. A plain `snow deploy` deploys those as usual.

Once the listed hosts have finished activating or rebooting, run the command snow gave. It repeats the flags that change how hosts are deployed, such as `--boot` or `--skip-reboot`. Use it rather than a plain `snow deploy`: a host that already runs the new system would be skipped as unchanged, and none of its hooks would run.

## Prune old generations

Every `snow deploy` in switch or boot mode, and every `snow rebuild` that switches or sets the next boot, adds a generation to the system profile, and snowglobe never deletes one. Each generation keeps its system's store paths from being garbage-collected, so a host that is deployed often fills its disk unless something prunes them. snowglobe leaves that to the host's own settings.

On NixOS:

```nix
{
  nix.gc = {
    automatic = true;
    dates = "weekly";
    options = "--delete-older-than 30d";
  };

  boot.loader.systemd-boot.configurationLimit = 10;
}
```

`nix.gc` runs the garbage collector as root on a schedule. With `--delete-older-than 30d`, it first deletes the profile generations older than 30 days, except the one that was current 30 days ago, then the store paths nothing refers to any more. The current generation is never deleted.

`configurationLimit` caps the number of boot menu entries; GRUB has the same option, `boot.loader.grub.configurationLimit`. It deletes no generations: older ones stay in the profile, and Option 2 can still switch to them until `nix.gc` removes them. The boot menu is rewritten at the next deployment or rebuild in switch or boot mode, so generations the collector deletes leave the menu then.

On nix-darwin, `nix.gc.automatic` and `nix.gc.options` work the same way, with `nix.gc.interval` (a launchd calendar interval) in place of `dates`. A Mac has no boot menu, so there is no `configurationLimit`. `nix.gc.automatic` requires `nix.enable`; with Determinate Nix, which turns it off, schedule the collection yourself or run it by hand.

To prune by hand, run `sudo nix-collect-garbage --delete-older-than 30d` on the host. Without `sudo` it can't delete the system profile's generations.

Pruning sets how far back you can roll back. Options 2 and 3 can only reach generations that are still there: with `--delete-older-than 30d`, the ones from the last 30 days and the one that was current before them. Pick a period longer than it might take you to notice a bad change. Option 1 doesn't depend on the host's generations: snow builds the old commit again, from your Nix store or from source.

## Limit the blast radius

A rollback is easier when few hosts need one. snow gives you three tools for that:

- **[Phases](/snowglobe/phases/).** Put low-risk hosts in an early phase and important ones in later phases. If any host in a phase fails (and is not marked optional), snow does not start the later phases, so a change that breaks the first hosts never reaches the rest.
- **Health checks.** A host that activates cleanly but serves errors is still a success to snow. A check in a hook turns that into a failure, which stops the rollout at that host. The failed host itself is not rolled back. See [Gate a rollout on health checks](/snowglobe/health-checks/).
- **Serial groups.** Hosts in a serial group deploy one at a time. If one fails, the members that haven't started are skipped and left on their old system. An [optional](/snowglobe/phases/#hosts-that-may-be-offline) member's failure skips no one.

## Related pages

- [Troubleshooting](/snowglobe/troubleshooting/): reading a failed deployment's report and log.
- [Rebuild the local machine](/snowglobe/rebuild/): the same generations apply to the machine you run `snow rebuild` on.
- [How a deployment works](/snowglobe/how-deploy-works/): the order of build, copy, activate and reboot.
