# snow push

`snow push` builds each host's system, copies it over SSH, activates it, and
reboots the host if it needs it. This page is the full reference.

- [Basics](#basics)
- [Phases](#phases)
- [What happens to each host](#what-happens-to-each-host)
- [Serial groups](#serial-groups)
- [Hooks](#hooks)
- [Reboots](#reboots)
- [Modes](#modes)
- [Reports and logs](#reports-and-logs)
- [All flags](#all-flags)

## Basics

```bash
snow push hydrogen oxygen        # these two
snow push                        # every host (except phase "never")
snow push --phase p1             # one phase
snow push --tags web,database    # any of these tags
snow push -e 'config: config.services.nginx.enable or false'
snow push --user deploy          # SSH user, if your SSH config doesn't set one
snow push --concurrency 5        # default 3
```

snow looks for `flake.nix` in the current directory and up. `-f <path>` or
`FLAKE_PATH` to point somewhere else.

`--filter`/`-e`, `--phase(s)` and `--tag(s)` can't be combined. With host
names, they narrow those names down. An unknown name is an error, and the
help lists the hosts. Hosts without a phase can't be picked with `--phase`;
use `-e`.

### SSH

snow connects to `networking.hostName`, plus `networking.domain` if set,
through your SSH config, so your `Host` entries apply. `nix copy` also
honors `NIX_SSHOPTS`.

Every ssh call runs with `BatchMode=yes`. There are no password prompts, so
you need key auth (an agent or a key file) and the host key already in
`known_hosts`.

Different port:

```nix
snowglobe.push.sshPort = 2222;
```

That port is used for everything: copy, activation, reboot, remote hooks.

snow never forwards your ssh-agent (`-o ForwardAgent=no`). A compromised host
can't use it to get into the others.

### nix-darwin

Darwin hosts always switch. No reboot detection, no reboots, and `--boot`,
`--test` and `--dry-run` fail before anything is built. See
[darwin.md](darwin.md).

## Phases

```nix
snowglobe.push.phase = "p1";
```

- Phases run one after another, in name order. Numbers sort by value, so `p2`
  runs before `p10`.
- Hosts without a phase run last, together.
- If a host in a phase fails, later phases don't run and `snow push` exits
  non-zero. Unless the host is [optional](#optional-hosts).
- `--ignore-phases` puts everything in one phase.
- Names: letters, digits, `-` and `_`. `default` is reserved for hosts
  without a phase.

### The `never` phase

Hosts in `never` are only pushed when you name them (`snow push curium`) or
ask for the phase (`--phase never`, or `--phases` with `never` in it). A bare
`snow push`, `--tag`, `-e` or another `--phase` skips them, even if they
match. `snow build` and `snow plan` work the same way. `snow list` too, except
a bare `snow list` shows everything.

### Optional hosts

For hosts that are often off, like a laptop:

```nix
snowglobe.push.optional = true;
```

If it fails, at any step, the report marks it `(optional)` and the phase
still passes. Later phases run, and `snow push` exits zero if only optional
hosts failed. Its on-failure hooks still run. It doesn't halt its serial group
either: the next host in the group starts once those hooks are done.

```
! Phase p1 (1/2 failed in 0:41)
  ✓ boron (switch)
  ✗ curium (optional) at check: ssh: connect to host curium port 22: Connection timed out
```

## What happens to each host

First snow prints an overview, `Pushing N hosts in M phases: …`, with
`(no phase)` for hosts without one. Then, inside a phase, every host goes
through this on its own:

```
1. build
2. unchanged? ── yes ──▶ done, reported as "unchanged"
3. copy
4. plan
   ┌─ deploy section (one host at a time per serial group) ─────────┐
5. │ pre-deploy hooks                                               │
6. │ activate                                                       │
7. │ reboot (if needed)                                             │
8. │ post-reboot hooks (only after a reboot)                        │
9. │ post-deploy hooks                                              │
   └────────────────────────────────────────────────────────────────┘

A failure at any step stops the host and runs its on-failure hooks.
```

- **Before anything starts**, snow reads every host's settings and builds its
  hooks on your machine, including the packages local hooks use (see
  [hooks.md](hooks.md#packages-in-hook-commands)). If that fails for any
  host, nothing gets deployed. It also fails if a local hook's command
  points at a `/nix/store` path your machine doesn't have (not on
  `--dry-run`), or if your ssh-agent hangs (`ssh-add -l` doesn't answer in 10
seconds). An agent it can't reach at all only gets a warning.
- **Step names** in reports: `build`, `check`, `copy`, `plan`, `pre-deploy`,
  `activate`, `reboot`, `post-deploy`, `serial group`. A failed post-reboot
  hook shows as `reboot`.
- **`--concurrency`** is one pool of slots for steps 1-4 and the deploy
  section. A host gives its slot back after planning and takes one again when
  its serial group lets it in. Waiting doesn't hold a slot.
- **Unchanged** means `/run/current-system` (what it runs) and
  `/nix/var/nix/profiles/system` (what it boots next) both already point at
  the built system. Those hosts are skipped, even with
  `rebootPolicy = "always"`. If snow can't read those paths, it deploys the
  host anyway, with a warning. A timeout fails the host.
- **`--redeploy`** pushes unchanged hosts too. Useful when an activation half
  failed: NixOS points `/run/current-system` at the new system before it
  starts services, so that host looks unchanged. It's a full deploy: hooks
  run, and `always` hosts reboot.
- **Planning is before any hook.** A host that planning blocks (say
  `--switch` when it needs a reboot) is never drained.
- **post-deploy** runs last, on the new system, rebooted or not. Only if
  everything before it worked.

## Serial groups

Hosts in the same serial group take turns through the deploy section, even
in the same phase. For example, two Kubernetes storage nodes never get
drained and rebooted at the same time.

```nix
snowglobe.push.serialGroup = "kubernetes";
```

- Steps 5-9 above take turns. The next host starts once the last one is done
  with all of it, post-deploy included, so a post-deploy health check
  ("storage volumes healthy") decides when the next one goes.
- Build, copy and plan stay parallel. Unchanged hosts never wait. Other groups
  and hosts without one aren't affected, beyond sharing `--concurrency`.
- Order: whoever finishes planning first. Not alphabetical, not
  configurable. A waiting host logs `waiting for serial group 'kubernetes'
  (held by praseodymium)`.
- `--test` and `--boot` take turns too. So does `--dry-run`, but it runs no
  hooks.

### When one fails

- The group stops before that host's on-failure hooks run, so nobody else
  starts meanwhile. Any step counts: build, copy, plan, activate, reboot, a
  hook.
- Members that hadn't started show as
  `⊘ <host> skipped, serial group 'kubernetes' halted after praseodymium failed`,
  and count as failed in the phase totals. They run no hooks, on-failure
  included. Nothing touched them.
- Hosts outside the group keep going. The phase fails, later phases don't
  run, `snow push` exits non-zero.
- An [optional host](#optional-hosts) failing doesn't stop the group.
- Only hosts in the same `snow push` run coordinate. Two pushes at once don't
  know about each other.

## Hooks

Commands that run before and after a host deploys, after a reboot, and on
failure. Everything about them is in [hooks.md](hooks.md).

## Reboots

### What needs a reboot

1. A new kernel.
2. A new initrd.
3. A service that isn't safe to restart would restart:
   `audit.service`, `kmod-static-nodes.service`,
   `network-local-commands.service`, `network-setup.service`,
   `syslog.socket`, `systemd-modules-load.service`, `systemd-oomd.socket`,
   `systemd-sysctl.service`, `systemd-udevd-control.socket`,
   `systemd-udevd-kernel.socket`.

### How snow checks

Only for hosts with `rebootPolicy = "auto"`, in auto mode or with
`--switch`. `never` and `always` skip it.

1. Compare `/run/booted-system/{kernel,initrd}` with the new system's. Fast,
   no activation, catches most cases.
2. If that found nothing, run `dry-activate` on the host (through
   `sudo snowglobe-activate` when it has the module) and look at the units it
   would stop or restart.

If it needs a reboot, snow sets the new system for next boot, runs
`sudo snowglobe-activate reboot` (or `sudo systemctl reboot` without the
module), and waits until the host answers with a new boot ID. It polls every 5
seconds, for up to 600 (`--reboot-timeout`). The report says
`✓ myhost (boot, rebooted)`.

`--skip-reboot` does the same but stops before the reboot. The report says
`✓ myhost (boot)` and the reboot is up to you.

### Reboot policies

```nix
snowglobe.push.rebootPolicy = "auto";  # or "never" or "always"
```

| Policy | What it does |
|---|---|
| `auto` (default) | Reboot only when needed. Otherwise switch. |
| `never` | Always boot mode. Never reboots. You reboot when it suits you. |
| `always` | Always boot mode and reboot. Unchanged hosts are still skipped. |

Policies apply in auto mode and to `--switch`. `--switch --force` overrides
them. `--boot`, `--test` and `--dry-run` ignore them.

| Policy | `snow push host --switch` |
|---|---|
| `auto` | Switches, or fails at plan if it needs a reboot |
| `never` | Boot mode, no reboot |
| `always` | Boot mode, then reboot |

## Modes

| Command | Reboot check | Activation | Reboots? |
|---|---|---|---|
| `snow push host` | yes | switch, or boot if it needs a reboot | if needed |
| `snow push host --skip-reboot` | yes | switch, or boot | no |
| `snow push host --switch` | under `auto` | see [policies](#reboot-policies) | only with `always` |
| `snow push host --switch --force` | yes, result ignored | switch | no |
| `snow push host --boot` | no | boot | no |
| `snow push host --test` | no | test | no |
| `snow push host --dry-run` | no | none | no |

Without `--force`, the reboot check only runs for hosts with
`rebootPolicy = "auto"`.

- **Default:** snow picks. Switch if it's safe, boot and reboot if not.
- **`--switch`:** switch now. Under `auto`, if the host needs a reboot it
  fails at plan with the reason, and nothing is activated. `--force` switches
  anyway, and the old kernel keeps running until the next reboot. That can
  leave services in a weird state if the kernel or a critical service
  changed.
- **`--boot`:** like `nixos-rebuild boot`. The host keeps running the old
  system and boots the new one next time. snow never reboots it, even with
  `rebootPolicy = "always"`.
- **`--test`:** activate, but don't make it the boot default. A reboot goes
  back to the old system.
- **`--dry-run`:** build, copy, then `dry-activate` to show what would
  change. Unchanged hosts stop before the copy. Nothing activated, no hooks,
  no reboot check.

`switch` and `boot` set `/nix/var/nix/profiles/system`. `test` and
`dry-run` don't.

## Reports and logs

Each phase ends with a report. One line per host: the mode and whether it
rebooted or was unchanged, or the step it failed at.

```
$ snow push hydrogen oxygen
✓ Push (2 hosts in 1:12)
  ✓ hydrogen (switch)
  ✓ oxygen (boot, rebooted)
```

oxygen needed a reboot (new kernel, say), so it got boot mode and a reboot.

Hosts without a phase are reported under `Push`. If they run after named
phases, the overview lists them as `(no phase)` and the report is titled
`Hosts without a phase`.

A failed host shows the step and the line of the error that matters, cut to
the terminal width. Under it, up to ten lines of context: the command that
failed, the stderr before the error, and where to find the rest. stdout is left out: for an activation it's mostly service
logs.

```
✗ Phase p1 (1/1 failed in 0:41)
  ✗ hydrogen at activate: Job for postgresql.service failed because the control process exited with error code.
    Command: ssh hydrogen sudo snowglobe-activate /nix/store/…-nixos-system-hydrogen switch
    activating the configuration...
    setting up /etc...
    reloading the following units: dbus.service
    restarting the following units: postgresql.service
    Job for postgresql.service failed because the control process exited with error code.
    … 7 more lines in /tmp/snow-1762255380.log
```

In a terminal, snow logs to `/tmp/snow-<timestamp>.log` and prints the path
when it starts. Every failure is in there in full: command, exit code,
stderr, stdout. If stderr isn't a terminal, the log goes to stderr instead.

More detail with `RUST_LOG`: `RUST_LOG=debug snow push hydrogen`,
`RUST_LOG=trace`, or one module, `RUST_LOG=snowlib::push=debug`.

## All flags

### Mode (pick one)

| Flag | |
|---|---|
| (none) | Auto: check, then switch or boot + reboot |
| `--switch` | Switch now. Fails if a reboot is needed, unless `--force` |
| `--boot` | Boot the new system next time. Ignores the reboot policy |
| `--test` | Activate, don't make it the boot default |
| `--dry-run` | Show what would change. No hooks |

### Reboots

| Flag | |
|---|---|
| `--skip-reboot` | Never reboot |
| `--reboot-timeout <SECONDS>` | How long to wait for a host to come back (default 600) |
| `--force` | With `--switch` only. Switch even if a reboot is needed or the policy says no |

### Hosts

| Flag | |
|---|---|
| `<HOSTS>...` | Hosts to push (default: all but phase `never`) |
| `-e`, `--filter <EXPR>` | Hosts whose config matches a Nix expression |
| `--phase <PHASE>` | One phase |
| `--phases <A,B>` | Any of these phases |
| `--tag <TAG>` | Hosts with this tag (`snowglobe.push.tags`) |
| `--tags <A,B>` | Any of these tags |

### Other

| Flag | |
|---|---|
| `-f`, `--flake-path <PATH>` | Flake directory (or `FLAKE_PATH`) |
| `--ignore-phases` | All selected hosts in one phase |
| `-q`, `--quiet` | Hide command output, show the summary |
| `--user <USER>` | SSH user, also for remote hooks |
| `--concurrency <N>` | Hosts at once (default 3) |
| `--redeploy` | Push hosts even if they already run the built system |

## See also

- [hooks.md](hooks.md): hooks
- [darwin.md](darwin.md): nix-darwin hosts
- [activation-helper.md](activation-helper.md): `snowglobe-activate` and what it does and doesn't protect
