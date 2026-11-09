---
title: "Write deployment hooks"
group: "Guides"
weight: 50
description: "Run commands before and after each host deploys: drain a node, leave a load balancer, gate on a health check, clean up after a failure."
---

Hooks are commands that `snow deploy` runs at fixed points in a host's deployment: draining a Kubernetes node before it switches, taking a web server out of a load balancer pool, checking that a service is healthy before the next host goes. They are declared in the host's own NixOS or nix-darwin configuration, under `snowglobe.hooks`, so they are versioned and reviewed with the rest of the host.

## Prerequisites

- The host imports the snowglobe module (`nixosModules.default` or `darwinModules.default`). See [Installation](/snowglobe/installation/).
- You can already deploy to the host. See the [tutorial](/snowglobe/tutorial/).

## A first hook

This configuration drains a Kubernetes node before it activates and uncordons it after the deployment, or if anything fails:

```nix
{ config, localPkgs, ... }:
let
  kubectl = "${localPkgs.kubectl}/bin/kubectl";
  node = config.networking.hostName;
in
{
  snowglobe.hooks = {
    pre-deploy.drain = {
      location = "local";
      command = "${kubectl} drain --ignore-daemonsets --delete-emptydir-data --timeout=5m ${node}";
      timeout = 360;
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
}
```

The hooks are `local`, so `kubectl` runs on the machine running `snow`, with your own kubeconfig, from `localPkgs`, that machine's package set (see [Packages in hook commands](#packages-in-hook-commands)).

Each hook lives under a stage and has a name made of letters, digits, `-` and `_`. The same name can be used in different stages, as `uncordon` is here. To see the hooks a host ends up with in a stage, evaluate them:

```bash
nix eval .#nixosConfigurations.k8s-1.config.snowglobe.hooks.pre-deploy --json
```

## Stages

A host goes through build, copy, plan, activate and, when needed, reboot. Hooks attach to four points:

| Stage | Runs | The host is running |
|---|---|---|
| `pre-deploy` | after the closure is copied and the plan is made, right before activation | the old system |
| `post-reboot` | after snow rebooted the host and it came back, and only then | the new system |
| `post-deploy` | last, after activation, any reboot and any post-reboot hooks, only if all of them succeeded | usually the new system (see below) |
| `on-failure` | whenever a step of the host fails, including a hook | whatever it was left on |

Consequences of this order:

- **Hooks only run for hosts that change.** A host already running and booting the new system is skipped before the copy, so a node with nothing to deploy is never drained. `snow deploy --redeploy` deploys it anyway, hooks included.
- **Planning comes before `pre-deploy`.** A host that cannot deploy as asked (say `--switch` on a host that needs a reboot) fails at the plan step, before it is drained.
- **`post-deploy` does not always mean "running the new system".** With `--boot`, `rebootPolicy = "never"`, or `--skip-reboot` on a host that needed a reboot, activation only sets the new system for the next boot, and `post-deploy` runs on the old one.
- **`post-reboot` only runs after a reboot that snow performed**, so it never runs on nix-darwin hosts.
- **`snow deploy --dry-run` and `snow rebuild` run no hooks.**

A [serial group](/snowglobe/cluster/) serialises the stretch from `pre-deploy` through `post-deploy`: the next host starts its `pre-deploy` hooks only after the previous one finished `post-deploy`, so a `post-deploy` health check decides when the next node may go. Build, copy and plan still run in parallel; [How a deployment works](/snowglobe/how-deploy-works/) shows the whole sequence.

Within a stage, hooks run one at a time, ordered by `priority` (lowest first, default 100), then by name:

```nix
{
  snowglobe.hooks.pre-deploy = {
    check-quorum = { location = "local"; command = "etcdctl endpoint health --cluster"; priority = 10; };
    drain = { location = "local"; command = "kubectl drain k8s-1"; };  # priority 100
  };
}
```

## Local or remote

Every hook sets `location`:

- **`local`** runs on the machine running `snow`, through `sh -c`, with snow's environment (`PATH`, `KUBECONFIG`, cloud credentials) and working directory. Use it for orchestration: cluster APIs, load balancers, notifications, anything needing credentials the host should not have.
- **`remote`** runs on the host over SSH, through `sh -c`, as the user snow connects as (your SSH config, or `--user`), without sudo. Use it for checks that only make sense on the host, such as `systemctl is-active` or a request to `localhost`.

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

Remote hooks use the deployment's SSH options (batch mode, no agent forwarding) but no keepalives, since a hook may restart the network; `timeout` bounds them instead.

If a host fails before snow has copied its closure (at build, the unchanged-system check, or the copy), its remote on-failure hooks cannot run, and the first one stops that host's on-failure stage: hooks sorted after it are skipped, local ones included, even with `continueOnFailure = true`. Put cleanup that must always happen in local hooks, and give remote on-failure hooks a higher `priority` number so the local ones run first.

## Packages in hook commands

A program called by name (`kubectl`) is looked up on `PATH` where the hook runs. More reproducibly, interpolate a package into the command, and snow makes sure the store path exists there. The package set depends on where the hook runs, since the machine running snow and the host may be different platforms (say, a Mac deploying to an aarch64 Linux board):

| Hook | Package set | Built for | How it gets there |
|---|---|---|---|
| `location = "local"` | `localPkgs` | the machine running snow | built or fetched on that machine before the deployment starts |
| `location = "remote"` | `pkgs` | the host | included in the host's system closure and copied with it |

```nix
{ config, pkgs, localPkgs, ... }:
{
  snowglobe.hooks = {
    pre-deploy.leave-pool = {
      location = "local";
      command = "${localPkgs.curl}/bin/curl -fsS -X POST https://lb.example.com/pool/web/remove -d host=${config.networking.hostName}";
    };
    post-deploy.check-nginx = {
      location = "remote";
      command = "${pkgs.curl}/bin/curl -fsS http://localhost/";
    };
  };
}
```

`localPkgs` is a module argument added by the snowglobe module ([`modules/local-pkgs.nix`](https://github.com/luizribeiro/snowglobe/blob/main/modules/local-pkgs.nix)): the host's own nixpkgs, with its `nixpkgs.config` and `nixpkgs.overlays`, instantiated for the platform of the machine running snow. It is the same nixpkgs pin, so the `kubectl` in a local hook matches the host's version; on the same platform it is simply `pkgs`. snow sets the platform itself (the `snowglobe.localSystem` option). If your flake passes a ready-made package set through `nixpkgs.pkgs`, overlays baked into it are not carried over to `localPkgs`.

Remote hooks' packages are referenced from `/etc/snowglobe/remote-hooks` in the host's system, so they arrive with the normal copy, before `pre-deploy`, and can be used even though the new system isn't active yet.

## Checks before the deployment starts

Before touching any host, snow validates every hook (see [Module options](/snowglobe/options/#hooks) for the allowed values) and builds or fetches the packages local hooks reference. A local hook whose `/nix/store` path doesn't exist on the machine running snow, typically a `pkgs` package for another platform, stops the deployment before anything is built, with an error naming the host, hook and path and suggesting `localPkgs`.

![A local hook's program is missing, so the deployment stops before building](demos/missing-hook.cast)

## Environment variables

Every hook gets these variables:

| Variable | Value |
|---|---|
| `SNOW_HOSTNAME` | the host's name in the flake (the attribute under `nixosConfigurations` or `darwinConfigurations`) |
| `SNOW_PHASE` | the host's `snowglobe.deploy.phase`; unset when the host has none |
| `SNOW_STORE_PATH` | the store path of the new system; unset in on-failure hooks of a host whose build failed |

`SNOW_HOSTNAME` is not the address snow connects to; for that, interpolate `config.networking.hostName` (and `config.networking.domain`) into the command.

Add your own variables with `environment` and `environmentFiles`:

```nix
{ localPkgs, ... }:
{
  snowglobe.hooks.pre-deploy.notify = {
    location = "local";
    command = ''${localPkgs.curl}/bin/curl -fsS -X POST "$WEBHOOK_URL" -d "deploying $SNOW_HOSTNAME"'';
    environment.CHANNEL = "deploys";
    environmentFiles = [ "/home/alice/.config/snowglobe/webhook.env" ];
    continueOnFailure = true;
  };
}
```

where `webhook.env` contains:

```bash
# Read by snow at deploy time; not part of any Nix store path
WEBHOOK_URL=https://hooks.example.com/services/T000/B000/XXXX
```

Environment files are parsed, not sourced: quotes are kept and `$VAR` is not expanded. [`environmentFiles`](/snowglobe/options/#snowglobehooksstagenameenvironmentfiles) has the full rules and what happens when a file can't be read.

## Secrets

- **`environment` values end up in the Nix store**, which is world-readable. Never put a secret there.
- **`environmentFiles` are read at deploy time on the machine running snow**, for local and remote hooks alike, so the file must exist there; a secret that agenix or sops-nix decrypts on the host only works if it also exists on your machine.
- **Write environment file paths as strings.** A path literal such as `./webhook.env` is copied into the store, which defeats the purpose.
- **A remote hook's command and `environment` are part of the host's system**, readable by any user on the host.
- **Variables sent to a remote hook travel on the SSH command line**, visible in the process list on both machines while it runs. Prefer local hooks for anything secret.

## Timeouts, retries and failures

A hook fails when it exits non-zero or outlives its `timeout` (300 seconds by default, including the SSH connection for remote hooks), in which case snow stops it, possibly half-way.

`retries` reruns a failed hook, waiting `retryDelay` seconds (5 by default) between attempts; `retries = 3` means up to four attempts. For a hook that waits on a condition, prefer a command that waits by itself (`kubectl wait`, `curl --retry`) with a matching `timeout`.

After the last attempt, `continueOnFailure` decides:

- **`false` (the default):** the host fails at that hook's stage. Its on-failure hooks run, the rest of its serial group is skipped, and later [phases](/snowglobe/phases/) don't run unless the host is optional, so a `post-deploy` check gates riskier phases. Other hosts in the phase carry on.
- **`true`:** the failure is logged as a warning and the host carries on. Use it for notifications, and for on-failure hooks so one failing cleanup doesn't stop the rest.

A failing `pre-deploy` hook means the host is never activated:

![A pre-deploy hook fails, so its host is never activated](demos/hook-failure.cast)

A failing `post-reboot` or `post-deploy` hook fails the host, but the new system is already active (or set for the next boot). snow does not roll anything back; see [Roll back a bad deployment](/snowglobe/rollback/).

## Failure and reruns

`post-deploy` does not run after a failure, so whatever a `pre-deploy` hook changes must also be undone in an `on-failure` hook, or a failed deploy leaves the node drained or out of the pool:

```nix
{ config, ... }:
let
  pool = action: "curl -fsS -X POST https://lb.example.com/pool/web/${action} -d host=${config.networking.hostName}";
in
{
  snowglobe.hooks = {
    pre-deploy.leave-pool = { location = "local"; command = pool "remove"; timeout = 30; };
    post-deploy.join-pool = { location = "local"; command = pool "add"; retries = 3; };
    on-failure.join-pool  = { location = "local"; command = pool "add"; continueOnFailure = true; };
  };
}
```

On-failure hooks run for any failure, even at the build before `pre-deploy` ran (remote ones have a limit, see [Local or remote](#local-or-remote)), so they must make sense either way; uncordoning a node that was never drained is harmless. Hosts skipped because their serial group or an earlier phase failed were never started and run no hooks.

A hook can also run more than once for the same change: with `retries`, a re-run deployment, or `--redeploy`. Write hooks so a second run finds the work done and succeeds: prefer idempotent commands (`kubectl drain` and `uncordon` succeed on a node already in that state), treat "already done" as success, and avoid hooks that toggle state or fail when what they create already exists.

## Example: wait for a node after a reboot

```nix
{ config, localPkgs, ... }:
{
  snowglobe.hooks.post-reboot.wait-ready = {
    location = "local";
    command = "${localPkgs.kubectl}/bin/kubectl wait --for=condition=Ready node/${config.networking.hostName} --timeout=5m";
    timeout = 360;
  };
}
```

A complete one-node-at-a-time Kubernetes setup is in [Deploy a cluster one node at a time](/snowglobe/cluster/).

## Troubleshooting hooks

- **A hook didn't run.** Check whether the host was `unchanged` or the deployment a `--dry-run`. `post-reboot` only runs after snow rebooted the host, `post-deploy` only if nothing failed, remote on-failure hooks only once the closure was copied.
- **A hook failed.** The error names the hook, stage and command, with exit code and stderr; the log file snow prints at start has the full output. Run the command by hand where it runs. For a remote hook, check that `ssh -o BatchMode=yes` works as the deploy user, and remember it runs without sudo.
- **"references ... which is not present on this machine".** A local hook uses a package from `pkgs` instead of `localPkgs`, or a hard-coded store path. Switch it to `localPkgs`.

Every hook option is listed under [Module options](/snowglobe/options/#hooks); more symptoms are in [Troubleshooting](/snowglobe/troubleshooting/).
