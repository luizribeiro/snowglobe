---
title: "Gate a rollout on health checks"
group: "Guides"
weight: 48
description: "Stop a phased rollout when a host is unhealthy, with checks in post-deploy and post-reboot hooks."
---

This guide shows how to check that a host is healthy after it deploys, and how a failed check stops the rollout before it reaches more hosts. The checks are ordinary commands in [hooks](/snowglobe/hooks/). Combined with [phases](/snowglobe/phases/) and [serial groups](/snowglobe/cluster/), they keep a bad change on the first host it breaks.

## What snowglobe checks by itself

Without any hook, snow fails a host when:

- the build, the copy or the activation fails;
- on a NixOS switch (or `--test`), any systemd unit is failing afterwards, unless it is listed in [`snowglobe.deploy.ignoredFailedUnits`](/snowglobe/options/#snowglobedeployignoredfailedunits);
- the host does not come back after snow reboots it.

That is all. A service that starts and then serves errors is a success to snow. So is a host that reboots and boots with a failed unit: the failed-units check runs only after a switch, not after a reboot, and never on nix-darwin hosts.

snowglobe does **not** have:

- a built-in HTTP or TCP probe, or a `healthChecks` option. A check is a command you write in a hook;
- automatic rollback. A host that fails a check stays on the new system;
- a command to run the checks without deploying. Hooks only run for hosts that change, so run a check by hand to test it;
- monitoring. A check runs once, during the deployment.

## A remote check on the host

A `remote` hook runs on the host over SSH, as the user snow connects as, without sudo. Use it for checks against `localhost`:

```nix
{ pkgs, ... }:
{
  snowglobe.hooks.post-deploy.health = {
    location = "remote";
    command = "${pkgs.curl}/bin/curl -fsS --retry 5 --retry-connrefused http://localhost:8080/health";
    timeout = 60;
  };
}
```

`-f` makes curl exit non-zero on an HTTP error, which fails the hook. `--retry-connrefused` covers the seconds when the service is still starting. `${pkgs.curl}` is part of the host's system, so it is on the host before the hook runs.

## A local check from your machine

A `local` hook runs on the machine running `snow`. Use it to check the host the way its clients see it, through its firewall and the network:

```nix
{ config, localPkgs, ... }:
{
  snowglobe.hooks.post-deploy.health = {
    location = "local";
    command = "${localPkgs.curl}/bin/curl -fsS --retry 10 --retry-all-errors --retry-delay 3 http://${config.networking.hostName}:8080/health";
    timeout = 60;
  };
}
```

Use `localPkgs`, not `pkgs`, in local hooks: it is built for your machine, which may not be the host's platform. See [Packages in hook commands](/snowglobe/hooks/#packages-in-hook-commands).

The same kind of hook can check a whole service instead of one host: a request through the load balancer, or a query to your cluster's API, like the `kubectl wait` in [Deploy a cluster one node at a time](/snowglobe/cluster/#kubernetes-drain-deploy-wait-uncordon).

## Timeouts and retries

A check that fails at once is often only early. There are two ways to wait:

- **Let the command wait.** `curl --retry`, `kubectl wait` and `systemctl is-system-running --wait` wait by themselves. Set the hook's `timeout` a little longer than the command's own limit, so the command gives the error, not snow.
- **Let snow retry.** `retries = 5` reruns a failed hook up to five more times, `retryDelay` seconds apart (5 by default). Use it for commands that cannot wait.

`timeout` applies to each attempt, and defaults to 300 seconds. With `retries`, the worst case is about `(retries + 1) × timeout`, plus the delays. See [`timeout`](/snowglobe/options/#snowglobehooksstagenametimeout) and [`retries`](/snowglobe/options/#snowglobehooksstagenameretries).

## post-deploy or post-reboot

Both stages can gate a rollout. They differ in when they run:

- **`post-deploy`** runs last, whether or not the host rebooted. Put service checks here.
- **`post-reboot`** runs only after snow rebooted the host, before `post-deploy`. Put checks that only matter after a boot here.

After a reboot, snow only waits for the host to come back. It does not check failed units. This hook fills that gap:

```nix
{ config, ... }:
{
  snowglobe.hooks.post-reboot.booted = {
    location = "remote";
    command = "${config.systemd.package}/bin/systemctl is-system-running --wait";
    timeout = 300;
  };
}
```

`is-system-running --wait` waits for the boot to finish, then fails if the system is `degraded`, that is, if any unit failed. Unlike the check after a switch, it does not know about `ignoredFailedUnits`.

`post-deploy` does not always see the new system. With `--boot`, `rebootPolicy = "never"` or `--skip-reboot` on a host that needed a reboot, the new system is only set for the next boot, and the check runs against the old one. See [Stages](/snowglobe/hooks/#stages).

## Stop the rollout at the canary

A failed check fails the host, like any other failed step. What that stops depends on where the host is:

- **Later phases.** When a phase has a failed host, the later phases are skipped. Hosts in the same phase are not stopped: they deploy in parallel and finish.
- **The rest of a serial group.** In a [serial group](/snowglobe/cluster/), the next member starts only after the current one finishes its `post-deploy` hooks. If a check fails, the members that have not had their turn are skipped and keep their old system.

So put the canary alone in the first phase, and give it the check:

```nix
# web1: the canary
{ pkgs, ... }:
{
  snowglobe.deploy.phase = "p1";
  snowglobe.hooks.post-deploy.health = {
    location = "remote";
    command = "${pkgs.curl}/bin/curl -fsS --retry 5 --retry-connrefused http://localhost:8080/health";
    timeout = 60;
  };
}
```

With `web2` and `web3` in `p2` and `db1` in `p3`, a change that breaks the check stops at `web1`. Phases `p2` and `p3` are skipped, `snow deploy` exits non-zero, and only `web1` needs fixing. In practice, put the check in a module shared by all web servers, so every one of them is checked, not only the canary.

A few cases weaken the gate:

- **An [optional](/snowglobe/phases/#hosts-that-may-be-offline) host** never stops later phases or its serial group, even when its check fails.
- **`continueOnFailure = true`** turns a failed check into a warning. Leave it off for checks.
- **`--ignore-phases`** puts every host in one phase, so a failed check only stops serial groups.
- **Naming hosts**, as in `snow deploy web2`, deploys them without the canary.
- **`--dry-run`** runs no hooks.

## After a failed check

The failed host is not rolled back: it keeps running the new system. Its `on-failure` hooks run, so a node a `pre-deploy` hook took out of a pool or drained can be put back; see [Failure and reruns](/snowglobe/hooks/#failure-and-reruns).

Then either fix the change and deploy again, or return the host to its previous system as in [Roll back a bad deployment](/snowglobe/rollback/). A host that already runs the new system is reported as unchanged and runs no hooks, so after fixing the host by hand, deploy with `--redeploy` to run its checks again.

## Coming from morph

morph declares health checks per host under `deployment.healthChecks`. In snowglobe, each one becomes a `post-deploy` hook:

| morph | snowglobe |
|---|---|
| `healthChecks.http`, requested from the deploying machine | a `local` hook running `curl` against the host |
| `healthChecks.cmd`, run on the host | a `remote` hook |
| repeating a check until it passes | `curl --retry`, or `retries` and `retryDelay` |
| a check's `timeout` | the hook's `timeout`, per attempt |
| `morph check-health` | no equivalent; run the command by hand |
| `--skip-health-checks` | no equivalent |

A check in snowglobe also decides what happens next: it stops later phases and the rest of its serial group.

## Related pages

- [Write deployment hooks](/snowglobe/hooks/): every hook stage, environment variables and secrets.
- [Roll out in phases](/snowglobe/phases/): ordering hosts by risk.
- [Deploy a cluster one node at a time](/snowglobe/cluster/): drain, deploy, check and restore nodes in turn.
- [Module options](/snowglobe/options/#hooks): every hook option.
