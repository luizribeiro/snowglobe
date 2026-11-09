---
title: "Roll out in phases"
group: "Guides"
weight: 40
description: "Order a deployment by risk, so a failure on low-risk hosts stops it before it reaches the rest."
---

This guide shows how to split your hosts into phases, so that `snow deploy` deploys low-risk hosts first and stops before the important ones if anything goes wrong. It also covers hosts you only deploy on purpose, hosts that may be offline, and how to deploy a subset of the fleet.

## Why phases

Deploying every host at once is fast, but a mistake in a shared module then breaks every host at the same moment. Phases trade a little time for a smaller blast radius. A typical order:

1. a canary, or a host nobody depends on;
2. stateless services that can be rebuilt or failed over;
3. stateful services, such as databases and storage;
4. shared infrastructure that everything else depends on.

snow deploys one phase at a time. If any host in a phase fails, the later phases do not start, so a change that breaks the canary never reaches the database. A post-deploy [hook](/snowglobe/hooks/) can turn "activated without an error" into "actually healthy": if the hook's check fails, the host fails, and the deployment stops there.

## Assign phases

A host's phase is a string in its configuration:

```nix
{
  snowglobe.deploy.phase = "p1";
}
```

Phase names may contain ASCII letters, digits, `-` and `_`. The name `default` is reserved: snow uses it internally for hosts without a phase, and the module rejects it.

Here is a small fleet in four phases. Each line goes in the configuration of the host named in the comment:

```nix
snowglobe.deploy.phase = "p1";   # web1: canary
snowglobe.deploy.phase = "p2";   # web2, web3
snowglobe.deploy.phase = "p3";   # db1
                               # nas1: no phase
```

A convenient way to keep this readable is to set the phase in a role module that several hosts import, for example `snowglobe.deploy.phase = "p2";` in the module that configures web servers.

## The order phases run in

Phases run in name order, with two refinements:

- **Numbers compare by value.** Wherever two names both have digits at the same place, the digits are compared as numbers, so `p2` runs before `p10`, and `k8s-p2` before `k8s-p10`. Text compares alphabetically: `canary` runs before `production`, which runs before `staging`.
- **Hosts without a phase run last**, after every named phase. This keeps phases opt-in: a flake that sets none deploys all its hosts together. In a phased fleet, leaving a host unphased is a way to deploy it after everything else, as `nas1` is above; `snow list` shows `-` for hosts that have no phase by accident.

For the fleet above, the deployment runs `p1`, then `p2`, then `p3`, then `nas1`. snow prints this plan before it starts, so a host in an unexpected phase shows up before anything happens:

```text
$ snow deploy
Deploying 5 hosts in 4 phases:
  p1          web1
  p2          web2, web3
  p3          db1
  (no phase)  nas1
```

Inside a phase, hosts do not wait for each other: each one builds, copies, activates and, if it needs to, reboots at its own pace. At most `--concurrency` hosts (3 by default) are busy at the same time.

[![Three hosts in two phases switch to their new systems](/snowglobe/switch.svg)](https://asciinema.org/a/6J8p7HDK0Df1tPSx)

## What happens when a host fails

When a host fails at any step, its on-failure hooks run and it is marked failed. The other hosts in the same phase keep going to the end; snow does not interrupt a host halfway through an activation because another one failed. When the phase is over, it is reported as failed, and every later phase is skipped:

```text
✗ Phase p2 (1/2 failed in 0:41)
  ✓ web2 (switch)
  ✗ web3 at activate: Job for nginx.service failed because the control process exited with error code.

⊘ Phase p3 (1 host skipped)
  ⊘ db1

⊘ Hosts without a phase (1 host skipped)
  ⊘ nas1

✗ 2/5 deployed, 1 failed, 2 skipped
```

Skipped hosts are untouched: snow has not built, copied or activated anything on them. `snow deploy` exits non-zero. Once you have fixed the cause, run the same command again. Hosts that already run the new system are reported as `(unchanged)` without being deployed again, so the deployment effectively picks up where it stopped.

[![One host fails to activate](/snowglobe/activation-failure.svg)](https://asciinema.org/a/NRfbtSt4AyH53Xsy)

## Hosts that may be offline

A laptop or a test machine is not always reachable, and its absence should not hold up the rest of the fleet. Mark it optional:

```nix
{
  snowglobe.deploy.optional = true;
}
```

An optional host is deployed like any other when it is reachable. When it fails, for whatever reason:

- its on-failure hooks still run, and the report marks it `(optional)`;
- its phase is reported with a warning instead of a failure, and later phases still run;
- it does not stop its [serial group](/snowglobe/cluster/);
- `snow deploy` exits zero if optional hosts were the only failures.

```text
! Phase p1 (1/1 failed in 0:12)
  ✗ laptop (optional) at copy: ...
✓ Phase p2 (1 host in 0:35)
! 1/2 deployed, 1 failed (optional)
```

Optional means "may fail", not "may be skipped": a broken configuration on an optional host also does not stop the deployment, so read the report.

## Hosts you only deploy on purpose

Some hosts should never be deployed by accident: a machine you rebuild by hand, a host under maintenance, or one where a deployment needs someone at the console. Put them in the phase `never`:

```nix
{
  snowglobe.deploy.phase = "never";
}
```

A host in `never` is left out of a bare `snow deploy`, and of any selection by tag, by filter expression or by other phases, even if it matches. It is deployed only when you name it, or when you select the phase explicitly. For example, if you moved `nas1` from the fleet above into `never`, a bare `snow deploy` would deploy only the web and database hosts, and `nas1` only when you ask for it:

```bash
snow deploy nas1        # nas1 is included because it is named
snow deploy --phase never # every host in the never phase
```

`snow list` with no arguments still shows these hosts, so you can see them. Apart from that, `never` is an ordinary phase name and sorts like one: if you name a `never` host together with others, it runs in its alphabetical place among the phases.

## Select which hosts to deploy

`snow deploy`, `snow build` and `snow list` share the same ways of selecting hosts:

```bash
snow deploy web1 web2                 # these hosts, by flake name
snow deploy --phase p2                # one phase
snow deploy --phases p1,p2            # any of these phases
snow deploy --tag web                 # hosts with this tag
snow deploy --tags web,database       # hosts with any of these tags
snow deploy -e 'config: config.services.postgresql.enable or false'
```

Hosts are named by their attribute in `nixosConfigurations` or `darwinConfigurations`. snow checks the names before it starts and lists the available hosts if one is misspelled.

Tags are set per host, with the same character rules as phases:

```nix
{
  snowglobe.deploy.tags = [ "web" "public" ];
}
```

The `-e` filter is a Nix function from a host's `config` to a boolean, so you can select on anything in the configuration. Write `or false` after attribute paths that only some hosts define, as above.

Only one filter (phase, phases, tag, tags or expression) can be used at a time. If you also name hosts, snow deploys only the named hosts that match the filter: `snow deploy web1 db1 --tag web` deploys `web1` alone. Selecting a subset does not change the order: the selected hosts still run phase by phase. See `snow deploy --help` for the complete list of options.

## Ignore phases for one deployment

`--ignore-phases` puts every selected host into a single phase and deploys them in parallel, still limited by `--concurrency` and still honouring serial groups:

```bash
snow deploy --ignore-phases --tag web
```

This is useful for a change you have already tested, where the phased rollout would only add waiting. A failure then no longer protects the remaining hosts, so use it deliberately.

## Clusters and serial groups

Phases order whole groups of hosts. To deploy the nodes of one cluster one at a time within a phase, so that only one node is ever drained or rebooting, give them a serial group. The [cluster guide](/snowglobe/cluster/) explains how serial groups and phases work together.

The full list of `snowglobe.deploy` options, with defaults, is on the [Module options](/snowglobe/options/) page.
