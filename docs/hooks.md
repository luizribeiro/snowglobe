# Lifecycle Hooks

Lifecycle hooks run your commands at set points of a push: drain a Kubernetes
node before it activates, take a host out of a load balancer, check its health
afterwards.

## Overview

Hooks are options of the snowglobe NixOS and nix-darwin modules. They run
during `snow push` (see [Host Pipeline](push.md#host-pipeline) for the full
per-host order and serial groups):

```
Build → (skip if unchanged) → Copy → Plan → [pre-deploy] → Activate → Reboot? → [post-reboot]? → [post-deploy]
                                                       ↓ (on any failure)
                                                  [on-failure]
```

- Hooks are declared in each host's config, next to the rest of the system.
- Each host runs its own hooks. Hosts in a serial group run their hooks one
  at a time, since hooks are part of the serialized deploy section.
- A hook runs either on the machine running `snow` (`local`) or on the host
  over SSH (`remote`).
- Hooks have a timeout, optional retries, and a choice of whether their
  failure fails the host.
- Hooks get the hostname, phase and store path in environment variables.

## Configuration

```nix
{ config, ... }:
{
  snowglobe.hooks = {
    pre-deploy = {
      drain-node = {
        location = "local";  # or "remote"
        command = "kubectl drain ${config.networking.hostName}";
        timeout = 300;
        continueOnFailure = false;
      };
    };
    post-deploy = { /* ... */ };
    post-reboot = { /* ... */ };
    on-failure = { /* ... */ };
  };
}
```

### Options

Each hook is an attribute of its stage. Its name must match `[a-zA-Z0-9_-]+`.

| Option | Type | Default | Meaning |
|--------|------|---------|---------|
| `location` | `"local"` or `"remote"` | required | Where the command runs |
| `command` | string | required | Shell command, run with `sh -c`; must not be empty |
| `environment` | attrs of strings | `{}` | Extra environment variables |
| `environmentFiles` | list of paths | `[]` | `KEY=VALUE` files read on the machine running `snow` |
| `timeout` | seconds | `300` | 1 to 7200 |
| `continueOnFailure` | bool | `false` | If true, the hook's failure doesn't fail the host |
| `priority` | int | `100` | Hooks in a stage run lowest priority first, then by name |
| `retries` | int | `0` | Extra attempts after a failure, at most 20 |
| `retryDelay` | seconds | `5` | Wait between attempts, 1 to 300 |

### Hook Stages

#### `pre-deploy`

Runs after the new system is copied and planned, before activation. It runs
only for hosts that change: a host already running and booting the built
system skips every hook.

This stage was called `pre-activate`. Defining `snowglobe.hooks.pre-activate`
fails evaluation with a pointer to `pre-deploy`.

Use it to drain nodes, remove a host from a load balancer pool, or leave a
cluster.

```nix
pre-deploy = {
  drain-node = {
    location = "local";
    command = "kubectl drain --delete-emptydir-data --ignore-daemonsets --timeout=5m ${config.networking.hostName}";
    timeout = 360;
  };
};
```

#### `post-deploy`

Runs last, once the host runs the new system: after activation, any reboot,
and `post-reboot` hooks. It runs whether or not the host rebooted, but only if
everything before it succeeded. To clean up after a failure, use `on-failure`.

This replaces `post-activate`, which ran before the reboot. Defining
`snowglobe.hooks.post-activate` fails evaluation with a pointer here.

Use it to uncordon nodes, add a host back to a pool, or gate on cluster health
before the next host in a serial group starts.

```nix
post-deploy = {
  uncordon-node = {
    location = "local";
    command = "kubectl uncordon ${config.networking.hostName}";
    timeout = 60;
    continueOnFailure = true;  # don't fail the host if uncordon fails
  };
};
```

#### `post-reboot`

Runs after the host reboots and comes back, only if it rebooted. nix-darwin
hosts are never rebooted, and neither are hosts pushed with `--skip-reboot`,
so it never runs for them.

```nix
post-reboot = {
  wait-node-ready = {
    location = "local";
    command = "kubectl wait --for=condition=Ready node/${config.networking.hostName} --timeout=5m";
    timeout = 360;
  };
};
```

#### `on-failure`

Runs when a step of the host fails, or when one of its hooks fails with
`continueOnFailure = false`.

- Failures during build, the unchanged check or copy happen before snow has an
  SSH target for the host. A `remote` on-failure hook can't run then: it
  errors, and the rest of the on-failure stage is skipped. Use `local`
  on-failure hooks for anything that must always run.
- The stage stops at the first on-failure hook that fails with
  `continueOnFailure = false`. Set `continueOnFailure = true` on on-failure
  hooks so one failing doesn't skip the others.
- An on-failure hook failing is logged as a warning. The host has failed
  already.

```nix
on-failure = {
  send-alert = {
    location = "local";
    command = "curl -X POST https://alerts.example.com/webhook -d '{\"host\": \"${config.networking.hostName}\", \"status\": \"failed\"}'";
    timeout = 30;
    continueOnFailure = true;
  };
};
```

## Execution Location

### Local (`location = "local"`)

Runs on the machine running `snow`: your workstation or a CI runner. Use it
for cluster orchestration (`kubectl`, `consul`, `nomad`), API calls, and
commands that need credentials the host doesn't have.

### Remote (`location = "remote"`)

Runs on the host over SSH, as the same user `snow push` connects as
(`--user`, or else your SSH config). Use it for host-specific checks. The
command must be on the remote `PATH`, or referenced by store path.

## Packages in Hook Commands

Hook commands can reference packages through Nix interpolation, and snow makes
sure those packages exist where the hook runs. Which package set to use depends
on where the hook runs:

| Location | Runs on | Use | Shipped by |
|----------|---------|-----|------------|
| `local` | the machine running `snow push` | `localPkgs` | snow builds them on that machine before deploying |
| `remote` | the target host | `pkgs` | the host's system closure, copied by `snow push` |

This covers both the `command` and the `environment` values of a hook.

```nix
{ pkgs, localPkgs, config, ... }:
{
  snowglobe.hooks.pre-deploy.drain-node = {
    location = "local";
    command = "${localPkgs.kubectl}/bin/kubectl drain ${config.networking.hostName}";
  };

  snowglobe.hooks.post-deploy.verify = {
    location = "remote";
    command = "${pkgs.curl}/bin/curl -fsS http://localhost:8080/health";
  };
}
```

`localPkgs` is the host's own nixpkgs (same pinned version) built for
`snowglobe.localSystem`, which snow sets to the platform of the machine running
it. A Mac pushing to an aarch64 Raspberry Pi gets a Mac `kubectl` in local
hooks and an aarch64 `curl` in remote hooks. It includes the host's
`nixpkgs.config` and `nixpkgs.overlays`. When the platforms differ, overlays
baked into an externally supplied `nixpkgs.pkgs` aren't carried over.

Commands that don't reference store paths (e.g. plain `kubectl` from the
deploying machine's `PATH`) work too.

If a local hook's command references a store path that isn't present on the
deploying machine, typically because it used `pkgs` instead of `localPkgs`,
`snow push` stops before touching any host, with an error naming the hook and
the path. `--dry-run` skips this check, since it runs no hooks.

## Error Handling

### `continueOnFailure`

A hook fails when it exits non-zero or times out, after any retries.

With `continueOnFailure = false` (the default):
- The host fails and its `on-failure` hooks run.
- Hosts in its serial group that haven't started are skipped.
- Other hosts in the phase keep going, but later phases are skipped.

With `continueOnFailure = true`:
- The failure is logged as a warning and the host carries on.
- `on-failure` hooks don't run.

Use `true` for best-effort work like notifications, and for `on-failure`
hooks.

### Retries

With `retries = N`, a failing hook runs up to N more times, `retryDelay`
seconds apart. Only the last attempt counts.

### Timeout

When a hook runs past its `timeout`:

1. snow sends `SIGTERM` to the hook's shell.
2. If it hasn't exited after 5 seconds, snow sends `SIGKILL` and waits for it
   to exit.
3. The attempt fails. Its stderr is replaced by `Hook timed out after N
   seconds`; its stdout is kept.

The signals go to the `sh` process only, not to its children. For a remote
hook, they stop the local `ssh` client; the command on the host may keep
running.

## Environment Variables

### Default Environment

| Variable | Description | Example |
|----------|-------------|---------|
| `SNOW_HOSTNAME` | Target hostname | `web01` |
| `SNOW_PHASE` | Push phase, set only when the host has one (`snowglobe.push.phase`) | `production` |
| `SNOW_STORE_PATH` | Nix store path being deployed | `/nix/store/...` |

Local hooks also inherit the environment `snow` runs in (e.g. `KUBECONFIG`).
Remote hooks get only the variables above, `environment`, and the contents of
`environmentFiles`.

### Custom Environment Variables

```nix
{ localPkgs, ... }:
{
  snowglobe.hooks.pre-deploy.notify-deploy = {
    location = "local";
    command = "${localPkgs.curl}/bin/curl -fsS -X POST \"$WEBHOOK_URL\" -d \"deploying $SNOW_HOSTNAME\"";
    environment = {
      CHANNEL = "#deploys";
    };
    environmentFiles = [
      "/home/me/.config/snow/webhook.env"  # WEBHOOK_URL=...
    ];
    continueOnFailure = true;
  };
}
```

- **`environment`** sets variables directly. Its values end up in the Nix
  store, so don't put secrets here.
- **`environmentFiles`** are read when the hook runs, on the machine running
  `snow`, for local and remote hooks alike. For a remote hook, their values
  are sent to the host on the ssh command line. Each line is `KEY=VALUE`;
  blank lines, lines starting with `#` and lines without `=` are skipped. Quotes are kept as
  part of the value and `export` isn't understood. A missing or unreadable
  file fails the hook.
- Write file paths as strings. A path literal like `./webhook.env` is copied
  into the world-readable Nix store.
- Secrets that are decrypted on the host, like agenix or sops-nix paths, only
  work here if the same file exists on the deploying machine.

Environment files are loaded first, then `environment` is applied, so
`environment` overrides file contents.

## Variable Substitution

Nix interpolation in a command, like `${config.networking.hostName}`, is
resolved when snow evaluates the host. snow writes every hook to the host's
`snowglobe.hooksFile`, a JSON file it builds before the push starts, so the
command it runs already holds the actual hostname.

## Complete Examples

### Kubernetes Cluster

Rolling updates, one node at a time. `features.kubernetes.enable` stands for
an option of your own:

```nix
{ config, lib, ... }:
{
  config = lib.mkIf config.features.kubernetes.enable {
    snowglobe.push.serialGroup = "kubernetes";

    snowglobe.hooks = {
      pre-deploy.drain-node = {
        location = "local";
        command = "kubectl drain --delete-emptydir-data --ignore-daemonsets --timeout=5m ${config.networking.hostName}";
        timeout = 360;
      };

      post-reboot.wait-node-ready = {
        location = "local";
        command = "kubectl wait --for=condition=Ready node/${config.networking.hostName} --timeout=5m";
        timeout = 360;
      };

      post-deploy.uncordon-node = {
        location = "local";
        command = "kubectl uncordon ${config.networking.hostName}";
        timeout = 60;
      };

      on-failure.uncordon-node = {
        location = "local";
        command = "kubectl uncordon ${config.networking.hostName}";
        timeout = 60;
        continueOnFailure = true;
      };
    };
  };
}
```

### Load Balancer Integration

```nix
{ config, ... }:
{
  snowglobe.hooks = {
    pre-deploy = {
      remove-from-pool = {
        location = "local";
        command = "curl -fsS -X POST https://lb.example.com/api/pool/remove -d '{\"host\": \"${config.networking.hostName}\"}'";
        timeout = 30;
        priority = 10;
      };
      drain-connections = {
        location = "local";
        command = "sleep 10";
        timeout = 15;
        priority = 20;
      };
    };

    post-deploy.add-to-pool = {
      location = "local";
      command = "curl -fsS -X POST https://lb.example.com/api/pool/add -d '{\"host\": \"${config.networking.hostName}\"}'";
      timeout = 30;
      retries = 3;
    };

    on-failure.add-to-pool = {
      location = "local";
      command = "curl -fsS -X POST https://lb.example.com/api/pool/add -d '{\"host\": \"${config.networking.hostName}\"}'";
      timeout = 30;
      continueOnFailure = true;
    };
  };
}
```

### Custom Validation

Fail the host if its services aren't healthy after the deploy:

```nix
{
  snowglobe.hooks.post-deploy.check-services = {
    location = "remote";
    command = "systemctl is-active nginx postgresql";
    timeout = 30;
  };
}
```

## Best Practices

- **Fail fast before the deploy, be lenient after it.** Leave
  `continueOnFailure = false` on pre-deploy checks. Set it to `true` on
  notifications and on-failure hooks.
- **Set realistic timeouts.** A health check needs seconds; a drain may need
  minutes.
- **Clean up in `on-failure`.** `post-deploy` doesn't run after a failure, so
  undo pre-deploy work (uncordon, re-add to a pool) there too.
- **Keep hooks next to the feature they support,** in the same module.
- **Try hooks on one host first:** `snow push test-host`, then
  `snow push --phase production`.
- **Prefer local hooks for cluster orchestration,** and remote hooks for
  host-specific checks.
- **Make hooks idempotent,** especially with `retries`: a retried or
  re-pushed hook may find its work already done.

## Troubleshooting

### Hook Failures

A failed hook fails the host. Its error names the hook, stage and command,
with the exit code and stderr (a timed-out hook shows exit code `-1`). snow
also logs a line like:

```
web01: hook 'drain-node' (pre-deploy) failed with exit code 1 after 3 attempts: <stderr>
```

For a timed-out hook, the log line reads `failed without an exit code`. To
debug:

1. Run the command by hand where the hook runs.
2. Check the environment variables it needs are set (`KUBECONFIG`, etc.).
3. Raise the timeout if the command is slow.
4. For remote hooks, check SSH access as the push user. snow runs ssh with
   `BatchMode=yes`, so the key must work without a prompt and the host key
   must already be known.

### Hooks Not Running

Look at a host's hooks with:

```bash
nix eval .#nixosConfigurations.web01.config.snowglobe.hooks.pre-deploy --json
```

Evaluate one stage at a time: the removed `pre-activate` and `post-activate`
stages throw when read.

Common causes:
- The host is unchanged. Unchanged hosts run no hooks; use `--redeploy` to
  deploy them anyway.
- `--dry-run` runs no hooks.
- `post-reboot` only runs after a reboot.
- `post-deploy` doesn't run if an earlier step failed.

A misspelled stage name (`preDeploy` instead of `pre-deploy`) fails evaluation
with "option does not exist".

## Migration from External Wrappers

Before, a wrapper around `snow push`:

```bash
push-k8s:
  kubectl drain node1 node2 node3
  snow push node1 node2 node3
  kubectl uncordon node1 node2 node3
```

After, with hooks and a serial group in the host config:

```nix
{ config, ... }:
{
  snowglobe.push.serialGroup = "kubernetes";
  snowglobe.hooks = {
    pre-deploy.drain-node = { location = "local"; command = "kubectl drain ${config.networking.hostName}"; };
    post-deploy.uncordon-node = { location = "local"; command = "kubectl uncordon ${config.networking.hostName}"; };
  };
}
```

```bash
snow push node1 node2 node3
```

Each node is drained, deployed and uncordoned in turn, and a failure stops the
rest of the group.

## Logging

snow logs each hook attempt and its result:

```
Executing pre-deploy hook 'drain-node' for web01 (attempt 1/1): kubectl drain web01
Hook 'drain-node' completed successfully for web01 in 12.345678s
```

Interactive pushes write the log to `/tmp/snow-<timestamp>.log`; other pushes
log to stderr. Hook logs use
the `snowlib::push::hooks` target at info level, which the default log level
includes. Setting `RUST_LOG` replaces the default, so if you set it, include
`snowlib=info`.

## FAQ

**Can hooks change the system before it's built?**
No. Hooks run during the push, after the build. Change the system through its
NixOS or nix-darwin config.

**Do hooks run in parallel?**
Hooks of different hosts run in parallel, up to `--concurrency`, except within
a serial group, where hosts take turns. A host's hooks in one stage run one at
a time, by `priority` and then name.

**Can hooks access secrets?**
Yes, through `environmentFiles` on the machine running `snow`, or through files
the command reads itself. For remote hooks, a file the command reads on the
host works too.

**What happens if a hook times out?**
The attempt fails like a non-zero exit, and is retried if `retries` allows.

**Can I skip hooks for one push?**
Only with `--dry-run`, which runs no hooks. Otherwise every changed host runs
its hooks.

**Do hooks run with `--dry-run`?**
No.
