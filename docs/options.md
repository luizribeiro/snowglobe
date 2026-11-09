---
title: "Module options"
group: "Reference"
weight: 115
description: "Every snowglobe.* option of the NixOS and nix-darwin modules, with types, defaults and examples."
---

This page lists every option the snowglobe module adds under `snowglobe.*`. It is generated from the module source, [`modules/*.nix`](https://github.com/luizribeiro/snowglobe/tree/main/modules) on GitHub, so the descriptions are the ones the module itself declares.

You set these options in each host's configuration, next to the rest of its NixOS or nix-darwin settings. They exist only once the host imports the snowglobe module, `nixosModules.default` on NixOS and `darwinModules.default` on nix-darwin (see [Installation](/snowglobe/installation/)):

```nix
nixosConfigurations.web1 = nixpkgs.lib.nixosSystem {
  system = "x86_64-linux";
  modules = [
    snowglobe.nixosModules.default
    {
      snowglobe.deploy.phase = "p1";
      snowglobe.deploy.tags = [ "web" ];
    }
    ./hosts/web1.nix
  ];
};
```

Every option is optional: a host that sets none of them is still deployed by `snow deploy`, with no phase, no tags and the `auto` reboot policy. To check what `snow` reads for each host, run `snow list --json`.

Both modules declare the same options. Where the nix-darwin module behaves differently, the option says so. In particular:

- `snowglobe.deploy.allowedUsers` grants its sudo rule through `security.sudo.extraRules` on NixOS and through `security.sudo.extraConfig` on nix-darwin. The rule is the same on both: the listed users may run `/run/current-system/sw/bin/snowglobe-activate` as root without a password. On nix-darwin the helper accepts only the `switch` mode and cannot reboot the host.
- `snowglobe.deploy.rebootPolicy`, `snowglobe.deploy.ignoredFailedUnits` and `post-reboot` hooks have no effect on nix-darwin hosts, because `snow` never reboots them and nix-darwin has no systemd units.

For how these options fit together, see [How a deployment works](/snowglobe/how-deploy-works/), [Roll out in phases](/snowglobe/phases/), [Write deployment hooks](/snowglobe/hooks/) and [Control reboots and activation modes](/snowglobe/reboots/).

## deploy

Options that decide whether, when and how `snow deploy` deploys a host.

### `snowglobe.deploy.phase`

The phase this host deploys in. Phases deploy one after another,
in name order with numbers by value (p2 before p10), and hosts
without a phase deploy last. If a host in a phase fails, later phases
are skipped.

Hosts in the phase `never` are deployed (or built) only when named, as
in `snow deploy db1`, or selected with `--phase never`.

**Type:** null or phase name (letters, digits, “-” and “_”), other than “default”

**Default:** `null`

**Example:** `"p1"`

### `snowglobe.deploy.tags`

Tags that select this host with `snow deploy --tag` or `--tags`.

**Type:** list of tag (letters, digits, “-” and “_”)

**Default:** `[ ]`

**Example:**

```nix
[
  "web"
  "database"
]
```

### `snowglobe.deploy.rebootPolicy`

When `snow deploy` reboots this host: `auto` when the new system needs a
reboot (a new kernel, initrd or critical service), `always` after
every deploy, and `never` not at all, setting the new system for the
next boot instead when it needs one. snow never reboots nix-darwin
hosts, so it has no effect on them.

**Type:** one of “auto”, “never”, “always”

**Default:** `"auto"`

### `snowglobe.deploy.serialGroup`

Hosts that share a serial group deploy one at a time within a
phase. Only the disruptive part is serialized: pre-deploy hooks,
activation, any reboot, post-reboot hooks and post-deploy hooks.
Build, copy and planning stay parallel, and hosts with nothing to
deploy never wait.

If a host in the group fails, its on-failure hooks run before the
next host may start, and group members that haven’t started are
skipped.

Only hosts deployed by the same `snow deploy` invocation coordinate.

**Type:** null or string matching the pattern .+

**Default:** `null`

**Example:** `"kubernetes"`

### `snowglobe.deploy.optional`

Whether `snow deploy` may go on without this host. An optional host that
fails, for example because it is offline, is reported as failed, but
its phase doesn’t fail: later phases still run, and the deployment succeeds
if only optional hosts failed.

**Type:** boolean

**Default:** `false`

### `snowglobe.deploy.ignoredFailedUnits`

Units that may be failing after `snow deploy` activates this host
without failing it.

A NixOS switch fails when any unit is failing afterwards, even one
the deployment didn’t touch: the switch starts its targets again, which
reruns units that were already failing, such as a crash-looping
service. snow then fails the host at activate, saying that the new
system is active. If the only failing units are listed here, the
host is deployed instead, and the report names the units.

The host still fails if any other unit is failing, or if the switch
fails for another reason, such as a unit that failed to reload.
Listed units are ignored even if this deployment broke them. nix-darwin
hosts aren’t affected.

**Type:** list of systemd unit name with its type, as in “vault.service”

**Default:** `[ ]`

**Example:**

```nix
[
  "vault.service"
]
```

### `snowglobe.deploy.sshPort`

The port snow connects to over ssh to deploy to this host: for
`nix copy`, activation, reboots and remote hooks. When null, ssh
picks the port, from the deployer’s ssh config or the default 22.

**Type:** null or 16 bit unsigned integer; between 0 and 65535 (both inclusive)

**Default:** `null`

**Example:** `2222`

### `snowglobe.deploy.allowedUsers`

Users allowed to run snowglobe-activate without password.

These users can activate any system from the Nix store, and reboot the
host, without a password. This is meant for push deployments.

Users in this list are effectively root on this host: anyone who can
use the Nix daemon can add a store path with their own activation
script and run it as root through snowglobe-activate. Use a dedicated
deploy user, and guard its SSH key like a root key.

**Type:** list of string

**Default:** `[ ]`

**Example:**

```nix
[
  "deploy"
]
```

## hooks

Commands that `snow deploy` runs around each host's deployment. Each of the four stages is an attribute set of hooks keyed by a name of your choice; every hook takes the same options, listed once below as `snowglobe.hooks.<stage>.<name>.*`.

### `snowglobe.hooks.pre-deploy`

Hooks to run after copying the new system but before activation.
They only run for hosts that change: a host already running and
booting the built system is skipped entirely.

Common use cases:

 - Drain cluster nodes (Kubernetes, Nomad)
 - Remove from load balancer pool
 - Stop accepting new connections
 - Gracefully leave cluster (Consul, etcd)

These hooks run before the system is activated, so the old
configuration is still running.

**Type:** attribute set of (submodule)

**Default:** `{ }`

**Example:**

```nix
{
  drain-node = {
    location = "local";
    command = "kubectl drain --ignore-daemonsets ${config.networking.hostName}";
    timeout = 360;
    continueOnFailure = false;
    priority = 50; # Run before other hooks
  };
}
```

### `snowglobe.hooks.post-reboot`

Hooks to run after the host reboots (only if a reboot occurred).

Common use cases:

 - Verify services started correctly
 - Run post-boot health checks
 - Re-enable monitoring
 - Update external state

These hooks only run if the deployment triggered a reboot.
They run after the host comes back online.

**Type:** attribute set of (submodule)

**Default:** `{ }`

**Example:**

```nix
{
  wait-system-running = {
    location = "remote";
    command = "systemctl is-system-running --wait";
    timeout = 120;
    continueOnFailure = false;
  };
}
```

### `snowglobe.hooks.post-deploy`

Hooks to run last, once the host runs the new system: after
activation, after any reboot, and after post-reboot hooks.
They run whether or not a reboot happened, but only if the
deploy (including any reboot) succeeded.

Common use cases:

 - Uncordon cluster nodes
 - Add back to load balancer pool
 - Gate on cluster health before the next host proceeds
 - Update service discovery

**Type:** attribute set of (submodule)

**Default:** `{ }`

**Example:**

```nix
{
  uncordon-node = {
    location = "local";
    command = "kubectl uncordon ${config.networking.hostName}";
    timeout = 60;
    continueOnFailure = true;
  };
}
```

### `snowglobe.hooks.on-failure`

Hooks to run if the deployment fails at any stage.

Common use cases:

 - Rollback external state changes
 - Send failure notifications
 - Restore cluster membership
 - Emergency uncordon/drain cleanup

These hooks run if any deployment step fails (build, copy,
activation, or other hooks). Useful for cleanup and rollback.

**Type:** attribute set of (submodule)

**Default:** `{ }`

**Example:**

```nix
{
  uncordon-on-failure = {
    location = "local";
    command = "kubectl uncordon ${config.networking.hostName}";
    timeout = 60;
    continueOnFailure = true;
  };
  send-notification = {
    location = "local";
    command = "notify-send 'Deployment failed for ${config.networking.hostName}'";
    timeout = 10;
    continueOnFailure = true;
  };
}
```

### `snowglobe.hooks.<stage>.<name>.location`

Where to execute the hook:

 - “local”: Execute on the machine running `snow deploy`
 - “remote”: Execute on the target host via SSH

**Type:** one of “local”, “remote”

**Example:** `"local"`

### `snowglobe.hooks.<stage>.<name>.command`

Command to execute. Can reference NixOS/nix-darwin config values
using standard Nix string interpolation.

Environment variables available during execution:

 - SNOW_HOSTNAME: Target hostname
 - SNOW_PHASE: Deployment phase (if configured)
 - SNOW_STORE_PATH: Path to the new system in /nix/store

Additional environment variables can be set via the `environment`
and `environmentFiles` options.

**Type:** string

**Example:** `"kubectl drain ${config.networking.hostName}"`

### `snowglobe.hooks.<stage>.<name>.timeout`

Maximum time (in seconds) to wait for the hook to complete.
If the hook exceeds this timeout, it will be terminated and
treated as a failure (unless continueOnFailure is true).

Default: 300 seconds (5 minutes)

**Note:** evaluation fails for a timeout outside 1 to 7200 seconds.

**Type:** positive integer, meaning >0

**Default:** `300`

### `snowglobe.hooks.<stage>.<name>.retries`

Number of times to retry the hook if it fails.

The total number of attempts is retries + 1 (initial attempt plus retries).
Set to 0 (default) for no retries.

Useful for hooks that depend on external services that may be
temporarily unavailable (e.g., Kubernetes API after node reboot).

**Note:** evaluation fails for more than 20 retries.

**Type:** unsigned integer, meaning >=0

**Default:** `0`

**Example:** `5`

### `snowglobe.hooks.<stage>.<name>.retryDelay`

Seconds to wait between retry attempts.

Only used when retries > 0.

**Note:** evaluation fails for a delay outside 1 to 300 seconds.

**Type:** positive integer, meaning >0

**Default:** `5`

**Example:** `30`

### `snowglobe.hooks.<stage>.<name>.continueOnFailure`

Whether to continue the deployment if this hook fails.

 - false (default): Hook failure aborts the deployment for this host
 - true: Hook failure is logged but deployment continues

Useful for cleanup hooks that should always run (e.g., uncordoning
nodes even if activation failed).

**Type:** boolean

**Default:** `false`

### `snowglobe.hooks.<stage>.<name>.priority`

Execution priority for hooks within the same stage.

Hooks are executed in order of priority (lower values first).
Hooks with the same priority are executed in alphabetical order
by name.

Default: 100

Common patterns:

 - Use negative values (-10, -5) for hooks that must run first
 - Use 0-50 for early hooks (e.g., pre-checks, validations)
 - Use 50-150 for normal hooks (default range)
 - Use 150-200 for late hooks (e.g., cleanup, notifications)

**Type:** signed integer

**Default:** `100`

**Example:** `50`

### `snowglobe.hooks.<stage>.<name>.environment`

Environment variables to set when executing the hook.

These are merged with the default Snow environment variables
(SNOW_HOSTNAME, SNOW_PHASE, SNOW_STORE_PATH).

**Type:** attribute set of string

**Default:** `{ }`

**Example:**

```nix
{
  KUBECONFIG = "/home/user/.kube/config";
  LOG_LEVEL = "debug";
}
```

### `snowglobe.hooks.<stage>.<name>.environmentFiles`

Files of KEY=VALUE lines whose variables are added to the hook’s
environment, for secrets kept out of the Nix store.

snow reads them on the machine running snow, for local and
remote hooks alike, so they must exist there, not on the host
being deployed. They are parsed, not sourced: blank lines and
lines starting with `#` are skipped, and values are only trimmed:
quotes are kept and variables are not expanded.

Later files override earlier ones, and `environment` overrides
them all. Give paths as strings: a Nix path such as `./deploy.env`
is copied into the world-readable Nix store.

**Note:** each line is split at the first `=`, and keys and values are trimmed. Local hooks get these variables on top of snow's own environment; remote hooks don't inherit snow's environment, and get the `SNOW_*` variables and yours on top of what the host gives a non-interactive SSH session. If a file can't be read, the host fails at that hook's stage without running it; with `continueOnFailure = true` the hook runs anyway without that file's variables, so a hook needing a secret from it should check the variable is set.

**Type:** list of absolute path

**Default:** `[ ]`

**Example:**

```nix
[
  "/home/deployer/.config/snowglobe/slack-webhook.env"
]
```

## localSystem

### `snowglobe.localSystem`

Platform of the machine running `snow deploy`, which is where local
hooks run. snow sets this when it reads hooks; you normally leave it
alone. Hooks get a matching package set as the `localPkgs` module
argument: the host’s nixpkgs, instantiated for this platform with the
host’s `nixpkgs.config` and `nixpkgs.overlays`. Overlays baked into an
externally supplied `nixpkgs.pkgs` aren’t carried over.

**Type:** string

**Default:** `pkgs.stdenv.hostPlatform.system`

**Example:** `"aarch64-darwin"`
