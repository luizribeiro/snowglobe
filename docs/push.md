# snow push

`snow push` builds each host's system, copies it over SSH, activates it, and
reboots the host if it needs it. This page explains how a push behaves and
why. For every flag, see `snow push --help`.

```bash
snow push hydrogen oxygen        # these two
snow push                        # every host (except phase "never")
snow push --phase p1             # one phase
snow push --tags web,database    # any of these tags
snow push -e 'config: config.services.nginx.enable or false'
```

## SSH

snow reaches `networking.hostName`, plus `networking.domain` if set, through
your SSH config, so your `Host` entries apply. Another port:
`snowglobe.push.sshPort`. `--user` picks the user.

Every ssh call runs in batch mode, so there are no password prompts: you need
key auth (an agent or a key file) and the host key already in `known_hosts`.
snow never forwards your agent, so a compromised host can't use it to get
into the others. If your agent hangs, the push fails before touching any
host.

## Phases

```nix
snowglobe.push.phase = "p1";
```

Phases run one after another, in name order, with numbers sorted by value
(`p2` before `p10`). Hosts without a phase go last. If a host fails, later
phases don't run, unless it's [optional](#hosts-that-may-be-off).

The point is controlling how far a bad push gets. Whatever you put in the
early phases goes first, and if something breaks there, the hosts in later
phases are never touched. `--ignore-phases` pushes everything at once.

### Hosts you only push on purpose

Hosts in the phase `never` only go when you name them or ask for
`--phase never`. A bare `snow push`, a tag or a filter skips them. For hosts
you'd rather deploy by hand.

### Hosts that may be off

```nix
snowglobe.push.optional = true;
```

For a laptop, say. If an optional host fails, its on-failure hooks still
run and the report marks it `(optional)`, but it doesn't stop its serial
group or later phases, and `snow push` still exits zero.

## What happens to each host

Inside a phase, hosts don't wait for each other. Each one goes through:

```
1. build
2. unchanged? ── yes ──▶ done
3. copy
4. plan
   ┌─ one host at a time per serial group ─┐
5. │ pre-deploy hooks                      │
6. │ activate                              │
7. │ reboot (if needed)                    │
8. │ post-reboot hooks (after a reboot)    │
9. │ post-deploy hooks                     │
   └───────────────────────────────────────┘

A failure at any step stops the host and runs its on-failure hooks.
```

Up to `--concurrency` hosts (default 3) are busy at once.

A few choices here are on purpose:

- **Unchanged hosts are skipped.** A host that already runs and boots the
  built system has nothing to do, so it doesn't get drained or rebooted.
  `--redeploy` pushes it anyway, for example to retry a half-failed
  activation.
- **Planning comes before any hook.** If a host can't deploy (say `--switch`
  when it needs a reboot), you find out before it's drained, not after.
- **Everything is checked before anything starts.** snow builds every
  host's hooks first, and checks that the packages local hooks use exist on
  your machine. If one is broken, nothing gets deployed.

## Serial groups

```nix
snowglobe.push.serialGroup = "kubernetes";
```

Hosts in the same group take turns through steps 5 to 9, so your storage or
Kubernetes nodes get drained and rebooted one at a time. The next host starts
only after the previous one finished its post-deploy hooks, so a health check
there decides when the next one goes. Build, copy and plan still run in
parallel.

If a host in the group fails, the hosts that haven't started are skipped and
untouched. The rest of the phase keeps going, but the phase fails. A failed
optional host doesn't stop the group; the next one waits for its on-failure
hooks, then goes.

Groups only coordinate within one `snow push`. Two pushes at the same time
don't know about each other.

## Reboots

Some changes can't be switched to safely. A new kernel or initrd only takes
effect on boot, and restarting services like udev or sysctl on a live system
can leave it half old, half new. So before activating, snow compares the
host's booted kernel and initrd with the new ones, then runs `dry-activate`
to see whether one of those services would restart.

If a reboot is needed, snow sets the new system for the next boot, reboots
the host, and waits for it to come back with a new boot ID (up to
`--reboot-timeout`, 600 seconds by default). Otherwise it just switches.

### Policies

```nix
snowglobe.push.rebootPolicy = "auto";  # or "never" or "always"
```

| Policy | |
|---|---|
| `auto` (default) | reboot only when needed |
| `never` | boot the new system next time, and leave the reboot to you |
| `always` | always reboot after deploying (unchanged hosts are still skipped) |

A host's policy wins over `--switch`. A host that should never be switched
live isn't, even if you forget. `--switch --force` is the explicit override.

## Modes

Most of the time plain `snow push` is what you want. The others:

| | |
|---|---|
| `--dry-run` | build, copy, and show what would change. Runs no hooks |
| `--switch` | switch now. Fails at plan if the host needs a reboot |
| `--switch --force` | switch now even if it needs a reboot. The old kernel keeps running |
| `--boot` | boot the new system next time. Never reboots, whatever the policy |
| `--test` | activate, but go back to the old system on reboot |
| `--skip-reboot` | like the default, but stop before rebooting |

`--boot`, `--test` and `--dry-run` ignore reboot policies. nix-darwin hosts
only switch; see [darwin.md](darwin.md).

## Reports and logs

Each phase ends with a report, one line per host:

```
$ snow push hydrogen oxygen
✓ Push (2 hosts in 1:12)
  ✓ hydrogen (switch)
  ✓ oxygen (boot, rebooted)
```

A failed host shows the step it failed at and the line of the error that
matters, with the command and the stderr leading up to it:

```
✗ Phase p1 (1/1 failed in 0:41)
  ✗ hydrogen at activate: Job for postgresql.service failed because the control process exited with error code.
    Command: ssh hydrogen sudo snowglobe-activate /nix/store/…-nixos-system-hydrogen switch
    activating the configuration...
    restarting the following units: postgresql.service
    Job for postgresql.service failed because the control process exited with error code.
    … 7 more lines in /tmp/snow-1762255380.log
```

In a terminal, snow writes a full log to `/tmp/snow-<timestamp>.log` and
prints the path when it starts. Otherwise the log goes to stderr. Set
`RUST_LOG=debug` for more.

## See also

- [hooks.md](hooks.md)
- [darwin.md](darwin.md)
- [activation-helper.md](activation-helper.md)
