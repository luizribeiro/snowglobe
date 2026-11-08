# Hooks

Hooks are commands snow runs during a push. Drain a Kubernetes node before
it activates, pull a host out of the load balancer, check it's healthy after.
You put them in the host's config, NixOS or nix-darwin.

Where they run in a push, and with serial groups:
[push.md](push.md#what-happens-to-each-host).

## Example

```nix
{ config, localPkgs, ... }:
{
  snowglobe.hooks = {
    pre-deploy.drain = {
      location = "local";   # the machine running snow
      command = "${localPkgs.kubectl}/bin/kubectl drain ${config.networking.hostName}";
      timeout = 360;
    };
    post-deploy.uncordon = {
      location = "local";
      command = "${localPkgs.kubectl}/bin/kubectl uncordon ${config.networking.hostName}";
      timeout = 60;
    };
    on-failure.uncordon = {
      location = "local";
      command = "${localPkgs.kubectl}/bin/kubectl uncordon ${config.networking.hostName}";
      continueOnFailure = true;
    };
  };
}
```

## Stages

| Stage | When | Typical use |
|---|---|---|
| `pre-deploy` | after copy and plan, before activation | drain, leave a pool |
| `post-reboot` | after the host reboots and is back. Only if it rebooted | wait for the node to be Ready |
| `post-deploy` | last, on the new system, rebooted or not. Only if everything before worked | uncordon, rejoin, health gate |
| `on-failure` | when any step of the host fails | undo pre-deploy, alert |

Unchanged hosts run no hooks. `--dry-run` runs none either, on-failure
included. `post-reboot` never runs on nix-darwin (no reboots) or with
`--skip-reboot`.

Each stage is an attrset of named hooks, `{}` by default. Names match
`[a-zA-Z0-9_-]+`. In a stage, hooks run one at a time, lowest `priority`
first, then by name.

## Options

| Option | Type | Default | |
|---|---|---|---|
| `location` | `"local"` or `"remote"` | required | where it runs |
| `command` | string | required | run with `sh -c`, can't be empty |
| `environment` | attrs of strings | `{}` | extra env vars |
| `environmentFiles` | list of paths | `[]` | `KEY=VALUE` files, read on the machine running snow |
| `timeout` | seconds | `300` | 1 to 7200 |
| `continueOnFailure` | bool | `false` | if true, a failure doesn't fail the host |
| `priority` | int | `100` | lower runs first |
| `retries` | int | `0` | extra attempts, up to 20 |
| `retryDelay` | seconds | `5` | 1 to 300 |

## Local or remote

**`local`** runs `sh -c <command>` on the machine running snow (your laptop,
a CI runner), with snow's environment (`KUBECONFIG` and so on). For cluster
tools, API calls, anything that needs credentials the host doesn't have.

**`remote`** runs on the host over SSH, as the user snow connects as
(`--user`, or your SSH config). No sudo. Roughly:

```
ssh -o BatchMode=yes -o ConnectTimeout=10 -o ForwardAgent=no [-p sshPort] [user@]host 'env K=V… sh -c …'
```

For host checks. The command must be on the host's `PATH`, or a store path.

## Packages in hook commands

Reference packages with Nix interpolation and snow makes sure they exist
where the hook runs. Use the right package set:

| Location | Runs on | Use | Comes from |
|---|---|---|---|
| `local` | the machine running snow | `localPkgs` | snow builds it there before the push |
| `remote` | the host | `pkgs` | the host's system closure |

```nix
{ pkgs, localPkgs, config, ... }:
{
  snowglobe.hooks.pre-deploy.drain = {
    location = "local";
    command = "${localPkgs.kubectl}/bin/kubectl drain ${config.networking.hostName}";
  };
  snowglobe.hooks.post-deploy.verify = {
    location = "remote";
    command = "${pkgs.curl}/bin/curl -fsS http://localhost:8080/health";
  };
}
```

`localPkgs` is the host's own nixpkgs, same pin, built for
`snowglobe.localSystem`. That defaults to the host's platform, and snow sets
it to the platform of the machine running it when they differ. A Mac pushing
to a Raspberry Pi gets a Mac `kubectl` in local hooks and an aarch64 `curl`
in remote ones. The host's `nixpkgs.config` and `nixpkgs.overlays` come
along. Overlays baked into an externally supplied `nixpkgs.pkgs` don't, when
the platforms differ.

Plain commands from the deploying machine's `PATH` (just `kubectl`) work
too.

snow builds every host's hooks before the push starts, so interpolations
like `${config.networking.hostName}` are already resolved. If a local hook's
`command` points at a store path your machine doesn't have (usually `pkgs`
instead of `localPkgs`), `snow push` stops before touching any host and names
the hook and the path. That check looks at `command` only, not
`environment`, and is skipped on `--dry-run`. `--dry-run` still builds the
hooks. `snow list` and `snow build` don't.

## Environment

| Variable | |
|---|---|
| `SNOW_HOSTNAME` | the host's flake attribute name (not `networking.hostName` or the FQDN) |
| `SNOW_PHASE` | its phase, only if it has one (also under `--ignore-phases`) |
| `SNOW_STORE_PATH` | the built system. Not set for on-failure after a failed build |

Then `environmentFiles` in order (later files win), then `environment`, which
wins over everything.

- `environment` values end up in the Nix store. No secrets there.
- `environmentFiles` are read when the hook runs, on the machine running
  snow, for local and remote hooks alike. They're parsed, not sourced: one
  `KEY=VALUE` per line. Blank lines, `#` lines and lines without `=` are
  skipped. Quotes stay part of the value. No `export`.
- Write the paths as strings. A path literal like `./webhook.env` gets copied
  into the world-readable store.
- A missing or unreadable file fails the hook, without retries. With
  `continueOnFailure = true` it's logged and the hook runs without that file.
- Secrets decrypted on the host (agenix, sops-nix paths) only work here if
  the same file exists on the deploying machine.

```nix
{ localPkgs, ... }:
{
  snowglobe.hooks.pre-deploy.notify = {
    location = "local";
    command = "${localPkgs.curl}/bin/curl -fsS -X POST \"$WEBHOOK_URL\" -d \"deploying $SNOW_HOSTNAME\"";
    environmentFiles = [ "/home/me/.config/snow/webhook.env" ];  # WEBHOOK_URL=...
    continueOnFailure = true;
  };
}
```

Remote hooks get the variables above, plus whatever sshd, PAM, the login
shell and your ssh `SendEnv` add.

## Secrets and remote hooks

- A remote hook's command and `environment` are written into
  `/etc/snowglobe/remote-hooks` in the host's closure, which anyone on the
  host can read.
- Values from `environmentFiles` for a remote hook travel as ssh arguments.
  They show up in `ps` on both machines.

## Failures

A hook fails when it exits non-zero or times out, after any retries.

`continueOnFailure = false` (default):
- The host fails and its `on-failure` hooks run.
- In a serial group, the hosts that haven't started are skipped.
- Other hosts in the phase keep going. Later phases don't run.

`continueOnFailure = true`:
- Logged as a warning, the host carries on, no on-failure hooks.

Use `true` for notifications and for on-failure hooks.

The report shows a failed hook's step as `pre-deploy` or `post-deploy`. A
failed post-reboot hook shows as `reboot`. The error reads
`Hook '<name>' (<stage>): <command>`, with the exit code and stderr.

### Retries

`retries = N` runs a failing hook up to N more times, `retryDelay` seconds
apart. Only the last attempt counts. Retries cover a non-zero exit or a
timeout, not an env file error or a command that fails to start.

### Timeouts

1. snow sends `SIGTERM` to the hook's `sh`.
2. 5 seconds later, `SIGKILL`.
3. The attempt fails with `Hook timed out after N seconds` as stderr. stdout
   is kept.

The signals go to `sh` only, not its children. For a remote hook, the
command on the host may keep going.

### on-failure hooks

- Build, unchanged-check and copy failures happen before snow has an SSH
  target. A `remote` on-failure hook errors then, and the rest of the stage
  is skipped. Use `local` for anything that must always run.
- The stage stops at the first on-failure hook that fails with
  `continueOnFailure = false`. Set it to `true` on on-failure hooks.
- A failing on-failure hook is logged as an error. The host has already
  failed.

## More examples

### Load balancer

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

### Kubernetes, one node at a time

```nix
{ config, lib, ... }:
{
  config = lib.mkIf config.features.kubernetes.enable {  # an option of your own
    snowglobe.push.serialGroup = "kubernetes";
    snowglobe.hooks = {
      pre-deploy.drain = {
        location = "local";
        command = "kubectl drain --delete-emptydir-data --ignore-daemonsets --timeout=5m ${config.networking.hostName}";
        timeout = 360;
      };
      post-reboot.wait-ready = {
        location = "local";
        command = "kubectl wait --for=condition=Ready node/${config.networking.hostName} --timeout=5m";
        timeout = 360;
      };
      post-deploy.uncordon = {
        location = "local";
        command = "kubectl uncordon ${config.networking.hostName}";
        timeout = 60;
      };
      on-failure.uncordon = {
        location = "local";
        command = "kubectl uncordon ${config.networking.hostName}";
        timeout = 60;
        continueOnFailure = true;
      };
    };
  };
}
```

`snow push node1 node2 node3` then drains, deploys and uncordons each node in
turn, and a failure stops the rest of the group.

### Health check

```nix
{
  snowglobe.hooks.post-deploy.check-services = {
    location = "remote";
    command = "systemctl is-active nginx postgresql";
    timeout = 30;
  };
}
```

## Tips

- Strict before the deploy, lenient after: leave `continueOnFailure = false`
  on pre-deploy checks, set it on notifications and on-failure hooks.
- Undo pre-deploy work in `on-failure` too. `post-deploy` doesn't run after a
  failure.
- Keep hooks in the same module as the feature they're for.
- Make them idempotent, especially with `retries`.
- Try a new hook on one host first.

## Troubleshooting

**A hook fails.** Run the command by hand where it runs. Check the env vars
it needs (`KUBECONFIG`…). Raise the timeout if it's slow. For remote hooks,
check you can `ssh -o BatchMode=yes` to the host as the push user, with the
host key already known.

**A hook doesn't run.** Is the host unchanged (`--redeploy`)? Is it
`--dry-run`? `post-reboot` only runs after a reboot, `post-deploy` only if
nothing failed. To see a host's hooks:

```bash
nix eval .#nixosConfigurations.web01.config.snowglobe.hooks.pre-deploy --json
```

A misspelled stage (`preDeploy`) fails evaluation with "option does not
exist".

**Logs.** Each attempt is logged:

```
Executing pre-deploy hook 'drain' for web01 (attempt 1/1): kubectl drain web01
Hook 'drain' completed successfully for web01 in 12.345678s
```

Hook logs use the `snowlib::push::hooks` target at info, which the default
log level (`snow=info,snowlib=info`) includes. If you set `RUST_LOG`, keep
`snowlib=info` in it.
