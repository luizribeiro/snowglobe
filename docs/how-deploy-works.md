---
title: "How a deployment works"
group: "Concepts"
weight: 90
description: "What snow deploy does to each host, in what order, and why it is built that way."
---

This page explains what happens between typing `snow deploy` and reading its final report: the checks that run before any host is touched, the steps each host goes through, how phases and serial groups fit around those steps, and why the order is what it is. The guides cover how to set each feature up; this page is about the model behind them.

[![Three hosts in two phases switch to a new system](/snowglobe/switch.svg)](https://asciinema.org/a/6J8p7HDK0Df1tPSx)

## Before any host is touched

`snow deploy` starts by working out which hosts to deploy, from the names, `--phase`, `--tag` or `-e` filter you gave, and reads each host's settings from the flake. Reading the settings also builds each host's hooks on your machine, including every package a local hook refers to. Because this reads every selected host's settings and hooks up front, a host whose snowglobe settings or hooks fail to evaluate or build stops the deployment before any host is touched.

It then prints the phases it is about to run and which hosts are in each:

```text
Deploying 3 hosts in 2 phases:
  p1  web1
  p2  db1, web2
```

Three checks follow, and each one stops the whole deployment if it fails:

1. **The requested mode suits every host.** nix-darwin hosts can only switch, so `--boot`, `--test` or `--dry-run` with a Mac selected fails here, before anything is built.
2. **Every package a local hook needs exists on this machine.** A local hook that interpolates `pkgs` instead of `localPkgs` points at a store path built for the host, which does not exist on your machine. `snow` names the hook and the missing path. A dry run skips this check, since it runs no hooks.
3. **The SSH agent answers.** If `ssh-agent` hangs, every `ssh` call would hang with it, so `snow` checks the agent first and stops if it does not respond. An agent that is simply not running only produces a warning, since `ssh` can still use key files.

These checks exist so that a mistake on your side cannot leave the fleet half deployed. It is much cheaper to find a missing hook package before the first host is drained than after.

[![A local hook's program is missing, so the deployment stops before building](/snowglobe/missing-hook.svg)](https://asciinema.org/a/uH01eTmwST7t8kNE)

## Phases run one after another

Hosts are grouped by their `snowglobe.deploy.phase`. Phases run in name order, comparing numbers by value, so `p2` runs before `p10`. Hosts without a phase run last. Hosts in the phase `never` are left out unless you name them or select that phase. `--ignore-phases` puts every selected host in a single phase.

A phase starts only after the previous phase has finished, and only if every host in the previous phase succeeded or was marked [optional](/snowglobe/phases/#hosts-that-may-be-offline). When a host fails, the other hosts in its phase keep going, since interrupting a host halfway through activation or a reboot would leave it in a worse state than letting it finish, but no later phase starts. The hosts in those phases are listed as skipped and are not touched at all.

This is what makes phases useful for rolling out by risk: you put low-risk hosts in the first phase, let a post-deploy hook check them, and the hosts that matter most are only touched after the earlier ones are known to work. See [Roll out in phases](/snowglobe/phases/).

## Each host runs its own pipeline

Inside a phase, hosts do not wait for each other. Each host goes through these steps on its own, as soon as it can:

| Step | What happens | Runs on |
|---|---|---|
| build | `nix build` the host's system | your machine |
| check | read the host's running and next-boot systems; stop here if both are already the built system | host (read-only) |
| copy | `nix copy` the system's closure to the host | host |
| plan | choose the activation mode, running [reboot detection](/snowglobe/reboot-detection/) if needed | host (read-only) |
| pre-deploy | run `pre-deploy` hooks | your machine or host |
| activate | run the activation helper with the chosen mode | host |
| reboot | reboot and wait for a new boot, if the plan calls for it; then run `post-reboot` hooks | host |
| post-deploy | run `post-deploy` hooks | your machine or host |

The step names in this table are the ones the progress board and the final report use, so a failure reads as, for example, `✗ web2 at activate`.

Because hosts are independent, a fast host can be activating while a slow one is still building, and one host's failure does not interrupt the others. `--concurrency` (3 by default) limits how many hosts are busy at once. A host waiting for its turn in a serial group does not count toward that limit, so it never blocks an unrelated host from making progress.

If any step fails, the host stops there, its `on-failure` hooks run, and the report shows the step and the error. Remote `on-failure` hooks need a host that `snow` has reached, so they only run for failures from the plan step onward; use local hooks for clean-up that must always run. [Local or remote](/snowglobe/hooks/#local-or-remote) explains how this affects the order of on-failure hooks.

### Unchanged hosts are skipped

The check step compares the system `snow` just built with two links on the host: `/run/current-system`, the system it runs now, and `/nix/var/nix/profiles/system`, the system it boots next. If both already point at the built system, there is nothing to deploy, and the host is reported as `unchanged`. It runs no hooks, so it is not drained, and it is not rebooted, even with the `always` reboot policy.

This makes a full `snow deploy` cheap to repeat: after a deployment that failed halfway, running it again only touches the hosts that still need the change. `--redeploy` turns the check off, which is useful to retry a host whose last activation partly failed, since NixOS records the new system as current before it starts services.

If the host's systems cannot be read for some other reason, `snow` deploys it rather than skipping it. A timeout is the exception: whatever stalled `ssh` would also stall the copy, so the host fails instead.

### Planning comes before any hook

The plan step decides how the host will be activated: switch now, or set the new system for the next boot and reboot. It looks at the requested mode, the host's reboot policy and, for the `auto` policy, whether the change needs a reboot. Some combinations cannot go ahead. For example, `--switch` on a host whose new kernel needs a reboot fails at plan, with the reason (`kernel linux-6.12.58 → linux-6.12.60`).

Planning runs before the `pre-deploy` hooks on purpose. Those hooks are usually disruptive: they drain a Kubernetes node, take a server out of a load balancer, or stop a replica. If planning came later, a host that cannot be deployed would be drained for nothing and left that way until an `on-failure` hook undid it. With planning first, a host that cannot deploy is never disturbed.

### Activation

Activation runs `snowglobe-activate` on the host through `sudo`, with the new system's store path and the mode. On NixOS the helper sets the system profile and runs `switch-to-configuration`; on nix-darwin it sets the profile and runs the system's `activate` script. [Security model](/snowglobe/security/) explains why a helper is used instead of calling the activation script directly.

A NixOS switch can also fail because units are failing afterwards, including units the deployment did not change. By then the new system is active, and the report says so. Units listed in `snowglobe.deploy.ignoredFailedUnits` do not fail the host; see [Module options](/snowglobe/options/) and [Troubleshooting](/snowglobe/troubleshooting/).

### Reboot

If the plan set the new system for the next boot because the change needs a reboot, `snow` reboots the host through the helper and waits until it answers with a new boot ID. [Reboot detection](/snowglobe/reboot-detection/#waiting-for-the-host) covers the details. Hosts set to boot by `--boot` or by the `never` policy are not rebooted, and nix-darwin hosts never are.

## Serial groups

Hosts that share a `snowglobe.deploy.serialGroup` take turns through the disruptive part of the pipeline: pre-deploy hooks, activation, any reboot, post-reboot hooks and post-deploy hooks. The next host in the group starts only after the previous one has finished its post-deploy hooks, so a health check in a post-deploy hook decides when the next node goes.

```text
            build → check → copy → plan   ┆  pre-deploy → activate → reboot → post-deploy
k8s-1       ████████████████████████████  ┆  ████████████████████████████████████
k8s-2       ██████████████████████        ┆  waiting ...........................  ████████████████████████████████████
k8s-3       ████████████████████████      ┆  waiting ....................................................................  ███...
            parallel                      ┆  one at a time
```

Build, check, copy and plan still run in parallel for the whole group, so by the time a node's turn comes, its system is already on the host and its plan is known. Unchanged hosts never wait.

If a host in the group fails, at any step, the group is halted at once, so no other member starts its turn. The failed host keeps its turn while its `on-failure` hooks run; after that, every member that has not had its turn is skipped. A skipped member may already have built, copied and planned its system, but no hook runs on it and nothing is activated. The report says which host halted the group. The rest of the phase keeps going, but the phase counts as failed, so later phases do not run. An optional host's failure does not stop its group; the next member waits for its `on-failure` hooks, then goes.

Serial groups only coordinate hosts within one `snow deploy`. Two deployments running at the same time do not know about each other. See [Deploy a cluster one node at a time](/snowglobe/cluster/).

[![Two hosts in a serial group take turns](/snowglobe/serial-group.svg)](https://asciinema.org/a/DfWYNZ7K7KCfiFvg)

## Dry runs

`--dry-run` goes through the same pipeline with the `dry-activate` mode: it builds, checks, copies and plans, then runs `switch-to-configuration dry-activate` on each host, which prints what would start, stop or restart. It runs no hooks and changes nothing on the host apart from adding the copied closure to its store. Reboot policies and reboot detection are not applied.

## The report

When a phase ends, `snow` prints its report, one line per host, with the activation mode and what else happened:

```text
✓ Phase p1 (1 host in 0:38)
  ✓ web1 (switch)
```

A failed host shows the step it failed at and the line of the error that matters. Here `web2` switched, but `nginx.service` was failing afterwards:

```text
✗ Phase p2 (1/2 failed in 0:52)
  ✓ db1 (boot, rebooted)
  ✗ web2 at activate: activated, but nginx.service is failing
```

When the error has more output, such as a Nix build error or a command that failed over SSH, the report also shows the few lines leading up to the error, and for a failed command the command itself. Longer output is cut short, ending with a line such as `… 7 more lines in /tmp/snow-1762255380.log` that points to the full log.

Labels you may see on a successful host are the mode (`switch`, `boot`, `test`, `dry-run`), `rebooted`, `unchanged`, and `ignored failing <units>`. A host skipped because its serial group halted shows as `⊘ k8s-3 skipped, serial group 'kubernetes' halted after k8s-2 failed`. A failed optional host is marked `(optional)`, and a phase whose only failures are optional hosts is shown as a warning rather than a failure.

After the last phase, `snow` lists any phases it skipped, then a total:

```text
⊘ Phase p3 (2 hosts skipped)
  ⊘ k8s-1
  ⊘ k8s-2

✗ 2/5 deployed, 1 failed, 2 skipped
```

`snow deploy` exits with a non-zero status if any host that is not optional failed. [Commands](/snowglobe/commands/#logs) describes where the full log goes and the [exit codes](/snowglobe/commands/#exit-codes).

[![One host fails to activate while the others succeed](/snowglobe/activation-failure.svg)](https://asciinema.org/a/NRfbtSt4AyH53Xsy)
