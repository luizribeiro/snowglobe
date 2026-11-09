---
title: "Deploy a cluster one node at a time"
group: "Guides"
weight: 45
description: "Use serial groups and hooks to drain, deploy, check and restore cluster nodes one after another."
---

This guide shows how to deploy the nodes of a cluster, such as Kubernetes nodes or servers behind a load balancer, so that only one node is out of service at any moment. You give the nodes a shared serial group, add hooks that take a node out before it activates and bring it back once it is healthy, and `snow deploy` handles the turn-taking.

## The problem

Inside a [phase](/snowglobe/phases/), snow deploys hosts in parallel. That is what you want for independent machines, but not for three Kubernetes nodes: if all three restart the kubelet or reboot at the same moment, the cluster loses its workloads, and possibly its quorum.

You could put each node in its own phase, but then the deployment waits for one node to build and copy before the next even starts, and a cluster of nine nodes needs nine phases. A serial group serializes only the part that disrupts the node.

## Serial groups

Hosts with the same `snowglobe.deploy.serialGroup` take turns:

```nix
{
  snowglobe.deploy.serialGroup = "kubernetes";
}
```

For each host in a group, a deployment still builds the system, copies it and plans the activation in parallel with every other host. Only then does the host wait for its turn. During its turn it runs, in order:

1. pre-deploy hooks,
2. activation,
3. the reboot, if one is needed,
4. post-reboot hooks, if it rebooted,
5. post-deploy hooks.

The next node in the group starts only after the current one has finished its post-deploy hooks. That makes a post-deploy health check the gate for the next node: if the node is not healthy, the next one does not start.

[![Two hosts in a serial group take turns](/snowglobe/serial-group.svg)](https://asciinema.org/a/DfWYNZ7K7KCfiFvg)

A few details of how turns are handed out:

- **Order.** Nodes take turns in the order they become ready, that is, finish planning. The order is not alphabetical and can change from one deployment to the next. While a node waits, the log shows `k8s-2: waiting for serial group 'kubernetes' (held by k8s-1)`.
- **Unchanged nodes never wait.** A node that already runs the new system is skipped before its turn, so it runs no hooks and is never drained.
- **Concurrency.** A node waiting for its turn does not occupy one of the `--concurrency` slots, so other hosts in the phase keep using them. The node takes a slot again when its turn starts.
- **Different groups do not wait for each other.** A `kubernetes` group and a `storage` group in the same phase each have their own turn, and hosts without a group never wait.

## Groups and phases

A serial group coordinates hosts **within one phase of one deployment**:

- Members in the same phase take turns, as described above.
- Members in different phases never overlap anyway, because phases run one after another. You can spread a cluster across phases (one canary node in `p1`, the rest in `p2`) and still give them all the same group.
- With `--ignore-phases`, every selected host is in a single phase, and the group serializes all its members.
- Two `snow deploy` commands running at the same time, from the same machine or from two, do not know about each other. Groups do not prevent that; if it is a risk, run deployments from one place, such as a CI job with a concurrency limit (see [Run from CI](/snowglobe/ci/)).

To select only the cluster, tag its nodes and deploy that tag, or name them:

```bash
snow deploy --tag kubernetes
snow deploy k8s-1 k8s-2 k8s-3
```

## Kubernetes: drain, deploy, wait, uncordon

Put the serial group and the hooks in the module that turns a host into a Kubernetes node, so every node gets the same behaviour. `features.kubernetes.enable` below stands for an option of your own:

```nix
{ config, lib, localPkgs, ... }:
let
  node = config.networking.hostName;
  kubectl = "${localPkgs.kubectl}/bin/kubectl";
in
{
  config = lib.mkIf config.features.kubernetes.enable {
    snowglobe.deploy.serialGroup = "kubernetes";
    snowglobe.deploy.tags = [ "kubernetes" ];

    snowglobe.hooks = {
      pre-deploy.drain = {
        location = "local";
        command = "${kubectl} drain ${node} --ignore-daemonsets --delete-emptydir-data --timeout=5m";
        timeout = 360;
      };

      post-deploy.wait-ready = {
        location = "local";
        command = "${kubectl} wait --for=condition=Ready node/${node} --timeout=5m";
        timeout = 360;
        priority = 50;
      };

      post-deploy.uncordon = {
        location = "local";
        command = "${kubectl} uncordon ${node}";
      };

      on-failure.uncordon = {
        location = "local";
        command = "${kubectl} uncordon ${node}";
        continueOnFailure = true;
      };
    };
  };
}
```

How the pieces fit:

- The hooks are `local`: they run on the machine running `snow`, with your environment, so `kubectl` finds your `KUBECONFIG` and credentials. Nothing about the cluster's API needs to be reachable from the node itself.
- `localPkgs.kubectl` is built for the machine running snow, from the host's own nixpkgs. `pkgs.kubectl` would be built for the node's platform, which is only the same thing when your machine and the node share a platform; a `kubectl` for another platform cannot run on your machine. The [hooks guide](/snowglobe/hooks/#packages-in-hook-commands) explains the difference.
- The hook `timeout` (in seconds, 300 by default) is longer than `kubectl`'s own `--timeout`, so `kubectl` gives a clear error before snow kills it.
- `wait-ready` has `priority = 50`, lower than the default of 100, so it runs before `uncordon`. Because it is a post-deploy hook, it runs whether or not the node rebooted, and the next node starts only once this one reports Ready.
- `post-deploy` hooks only run when everything before them succeeded. If the drain times out, the activation fails, or the node does not come back from a reboot, `on-failure.uncordon` makes the node schedulable again instead of leaving it cordoned. `continueOnFailure = true` means that if this cleanup fails, any other on-failure hooks still run. A failing on-failure hook never replaces the host's original error; snow logs it as a warning.

A run over three nodes then looks like this: all three build and copy at once; `k8s-1` is drained, activated, rebooted if needed, waited for and uncordoned; then `k8s-2`; then `k8s-3`.

Write hooks so they can safely run twice. `kubectl drain` on a node that is already drained, and `uncordon` on a schedulable node, both succeed, which matters when you re-run a deployment after a failure.

## Load balancer: remove, deploy, add back

The same pattern works for web servers behind a load balancer with an HTTP API. Here, `lb.example.com` and its endpoints stand for whatever your load balancer provides:

```nix
{ config, localPkgs, ... }:
let
  curl = "${localPkgs.curl}/bin/curl";
  body = "{\"host\": \"${config.networking.hostName}\"}";
in
{
  snowglobe.deploy.serialGroup = "web";

  snowglobe.hooks = {
    pre-deploy.remove-from-pool = {
      location = "local";
      command = "${curl} -fsS -X POST https://lb.example.com/api/pool/remove -d '${body}'";
      timeout = 30;
    };

    post-deploy.health = {
      location = "remote";
      command = "${config.systemd.package}/bin/systemctl is-active nginx";
      timeout = 30;
      priority = 50;
    };

    post-deploy.add-to-pool = {
      location = "local";
      command = "${curl} -fsS -X POST https://lb.example.com/api/pool/add -d '${body}'";
      retries = 3;
    };

    on-failure.add-to-pool = {
      location = "local";
      command = "${curl} -fsS -X POST https://lb.example.com/api/pool/add -d '${body}'";
      continueOnFailure = true;
    };
  };
}
```

The `health` check is a `remote` hook: it runs on the web server over SSH, as the deploy user, without sudo. It uses the host's own `systemd` package, which is part of the host's system, so it exists there. `add-to-pool` retries three times, five seconds apart by default, in case the load balancer's API is briefly unavailable.

Whether `on-failure` should add a host back to the pool depends on your setup. If a failed node may be serving a half-applied configuration, leave it out of the pool and let your load balancer's own health checks decide.

## When a node fails

If a node in a serial group fails at any step, including its hooks:

1. The group stops at once, and the node's on-failure hooks run.
2. Members that have not started their turn are skipped: they were built and copied, but nothing was activated on them, and they keep running their current system.
3. Hosts in the same phase that are not in the group carry on to the end.
4. The phase fails, so later phases are skipped, and `snow deploy` exits non-zero.

A node that fails before its turn, for example because its build fails, stops the group in the same way. The report shows why each skipped node did not run:

```text
✗ Phase p2 (3/4 failed in 0:52)
  ✓ web1 (switch)
  ✗ k8s-1 at pre-deploy: error: unable to drain node "k8s-1"
  ⊘ k8s-2 skipped, serial group 'kubernetes' halted after k8s-1 failed
  ⊘ k8s-3 skipped, serial group 'kubernetes' halted after k8s-1 failed
```

Fix the cause and run the deployment again. Nodes that already run the new system are reported as unchanged and skip their turn, so the deployment continues with the nodes that were left.

An [optional](/snowglobe/phases/#hosts-that-may-be-offline) node is the exception: when it fails, the group does not stop. If it failed during its turn, its on-failure hooks run while it still holds the turn, so the next node starts only after that cleanup is done.

## See also

- [Write deployment hooks](/snowglobe/hooks/) for every hook stage, environment variables and secrets.
- [Control reboots and activation modes](/snowglobe/reboots/) for how a node decides whether it must reboot during its turn.
- [How a deployment works](/snowglobe/how-deploy-works/) for the full pipeline of a host.
- [`modules/hooks.nix`](https://github.com/luizribeiro/snowglobe/blob/main/modules/hooks.nix) and [`modules/deploy.nix`](https://github.com/luizribeiro/snowglobe/blob/main/modules/deploy.nix) for the option definitions.
