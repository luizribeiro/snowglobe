# Hooks

Hooks are commands snow runs during a push. Drain a Kubernetes node before
it activates, pull a host out of the load balancer, check it's healthy after.
You put them in the host's config, NixOS or nix-darwin.

Where they run in a push, and with serial groups:
[push.md](push.md#what-happens-to-each-host). Every option and its default is
in the option descriptions, in [`modules/hooks.nix`](../modules/hooks.nix).

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

| Stage | When |
|---|---|
| `pre-deploy` | after copy and plan, right before activation |
| `post-reboot` | after the host reboots and is back, only if it rebooted |
| `post-deploy` | last, on the new system, only if everything before worked |
| `on-failure` | when any step of the host fails |

Unchanged hosts run no hooks, and `--dry-run` runs none at all. Hooks in a
stage run one at a time, lowest `priority` first, then by name.

Undo pre-deploy work in `on-failure` too: `post-deploy` doesn't run after a
failure, so a drained node would otherwise stay drained.

## Local or remote

`local` runs on the machine running snow, with snow's environment
(`KUBECONFIG` and so on). That's where your cluster tools and credentials
are, so use it for orchestration and API calls.

`remote` runs on the host over SSH, as the user snow connects as, without
sudo. Use it for checks on the host itself.

## Packages in hook commands

Interpolate packages and snow makes sure they exist where the hook runs. The
catch is which package set:

- local hooks: `localPkgs`, built for the machine running snow
- remote hooks: `pkgs`, from the host's own system

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

A Mac pushing to a Raspberry Pi gets a Mac `kubectl` in local hooks and an
aarch64 `curl` in remote ones. `localPkgs` is the host's own nixpkgs, same
pin, so versions match.

If a local hook uses `pkgs` by mistake, its store path won't exist on your
machine. snow catches that before touching any host and tells you which hook.

Plain commands from your `PATH` (just `kubectl`) work too.

## Environment and secrets

Hooks get `SNOW_HOSTNAME` (the host's name in the flake), `SNOW_PHASE` (if it
has one) and `SNOW_STORE_PATH` (the new system). Add your own with
`environment` and `environmentFiles`.

Be careful with secrets:

- `environment` values end up in the Nix store. Don't put secrets there.
- `environmentFiles` are read at push time, on the machine running snow.
  Write their paths as strings: a path literal like `./webhook.env` gets
  copied into the world-readable store. They're plain `KEY=VALUE` lines, not
  shell.
- Secrets decrypted on the host (agenix, sops-nix) only work in
  `environmentFiles` if the same file is on your machine too.
- A remote hook's command and `environment` are part of the host's system,
  readable by anyone on it. Values from env files are sent as ssh arguments
  and show up in `ps`.

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

## When a hook fails

A hook fails when it exits non-zero or runs past its `timeout`, after any
`retries`.

By default that fails the host: its on-failure hooks run, the rest of its
serial group is skipped, and later phases don't run (unless the host is
[optional](push.md#hosts-that-may-be-off)). Other hosts in the phase keep
going.

With `continueOnFailure = true`, the failure is a warning and the host
carries on. Use it for notifications and for on-failure hooks, so one failing
cleanup doesn't skip the others.

Make hooks safe to run twice. With retries, or a push you re-run, a hook can
find its work already done.

A `remote` on-failure hook can't run when the host failed before snow could
reach it (build or copy). Use `local` for anything that must always run.

## More examples

### Load balancer

```nix
{ config, ... }:
{
  snowglobe.hooks = {
    pre-deploy.remove-from-pool = {
      location = "local";
      command = "curl -fsS -X POST https://lb.example.com/api/pool/remove -d '{\"host\": \"${config.networking.hostName}\"}'";
      timeout = 30;
    };
    post-deploy.add-to-pool = {
      location = "local";
      command = "curl -fsS -X POST https://lb.example.com/api/pool/add -d '{\"host\": \"${config.networking.hostName}\"}'";
      retries = 3;
    };
    on-failure.add-to-pool = {
      location = "local";
      command = "curl -fsS -X POST https://lb.example.com/api/pool/add -d '{\"host\": \"${config.networking.hostName}\"}'";
      continueOnFailure = true;
    };
  };
}
```

### Kubernetes, one node at a time

Put the hooks and a serial group in the module that enables Kubernetes:

```nix
{ config, lib, ... }:
{
  config = lib.mkIf config.features.kubernetes.enable {  # an option of your own
    snowglobe.deploy.serialGroup = "kubernetes";
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
      };
      on-failure.uncordon = {
        location = "local";
        command = "kubectl uncordon ${config.networking.hostName}";
        continueOnFailure = true;
      };
    };
  };
}
```

`snow deploy node1 node2 node3` drains, deploys and uncordons each node in
turn. If one fails, the rest of the group isn't touched.

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

## Troubleshooting

A failing hook's error names the hook, stage and command, with the exit code
and stderr. Run the command by hand where it runs, and check the variables it
needs. For a remote hook, check that `ssh -o BatchMode=yes` to the host works
as the push user.

A hook that doesn't run: is the host unchanged (`--redeploy`)? Is it
`--dry-run`? `post-reboot` only runs after a reboot, and `post-deploy` only if
nothing failed. To see a host's hooks:

```bash
nix eval .#nixosConfigurations.web01.config.snowglobe.hooks.pre-deploy --json
```
