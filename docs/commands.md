---
title: "Commands"
group: "Reference"
weight: 110
description: "What each snow command does, the options every command shares, where logs go, and the exit codes."
---

`snow` has four commands: `list`, `build`, `deploy` and `rebuild`. This page describes what each one does and shows typical invocations. For the complete list of a command's flags, run `snow <command> --help`; `snow --help` lists the commands themselves, and `snow --version` prints the version you have.

snowglobe is pre-1.0, so flags may still change between minor releases. The [changelog](/snowglobe/changelog/) records each change.

## Finding the flake

Every command works on one flake. Without any option, `snow` looks for a `flake.nix` in the current directory, then in each parent directory in turn, and uses the first one it finds. This means you can run `snow` from anywhere inside the repository that holds your flake.

To point at a flake elsewhere, pass `-f` (`--flake-path`) or set the `SNOW_FLAKE` environment variable. The value is a local path, either a directory containing `flake.nix` or the `flake.nix` file itself; `~` and environment variables in it are expanded. The flag takes precedence over the variable, and both take precedence over the search.

```bash
snow -f ~/src/infra list
SNOW_FLAKE=~/src/infra snow deploy web1
```

The flag can go before or after the command name. A flake reference such as `github:owner/repo` is not accepted: `snow` needs a checkout on disk.

If no flake is found, the command stops with `Could not find flake.nix in '<directory>' or any parent directory`.

## Selecting hosts

`list`, `build` and `deploy` share the same host selection. Name hosts as arguments, or select them with one of these filters:

| Flag | Selects |
|---|---|
| `--phase p1` / `--phases p1,p2` | hosts whose `snowglobe.deploy.phase` is one of these |
| `--tag web` / `--tags web,db` | hosts with any of these `snowglobe.deploy.tags` |
| `-e` / `--filter 'config: …'` | hosts for which a Nix function of the host's `config` returns true |

Only one filter can be used at a time. When you combine host names with a filter, `snow` acts on the named hosts that also match it. A host name that does not exist in the flake is an error, and the error lists the hosts that do.

`build` and `deploy` leave out hosts in the phase `never` unless you name them or select `--phase never`. `list` with no selection at all shows every host, `never` included; with a filter it selects the same hosts `deploy` would. See [Roll out in phases](/snowglobe/phases/) for how phases work.

## snow list

`snow list` shows the hosts in the flake's `nixosConfigurations` and `darwinConfigurations`, with each host's phase and tags. Use it to check what `snow` sees before you deploy, or to test a selection: `snow list --tag web` shows exactly the hosts `snow deploy --tag web` would deploy.

```bash
snow list
snow list --tag web
snow list -e 'config: config.services.postgresql.enable or false'
```

The output is a table with one row per host. A dash means the host has no phase or no tags:

```text
HOST    PHASE  TAGS
db1     p2     db
web1    p1     web
web2    p1     web
```

With `--json`, `snow list` prints a JSON array instead, one object per host with its `name`, `system`, `phase`, `tags`, `rebootPolicy`, `optional` and `serialGroup`. When a host sets no phase, serial group or `system`, that field is `null`; `tags` is then an empty list, `rebootPolicy` is `"auto"` and `optional` is `false`. This is the output to use from scripts:

```bash
snow list --json | jq -r '.[] | select(.phase == "p1") | .name'
```

Run `snow list --help` for all options.

## snow build

`snow build` builds the system of each selected host on the local machine, without copying or activating anything. It is useful to check that a change evaluates and builds for every host before you deploy, for example in CI. Hosts build in parallel, three at a time by default (`--concurrency`).

```bash
snow build
snow build web1 web2
snow build --phase p1 --quiet
```

After the builds, `snow` prints one line per host, with the store path of the built system or the reason the build failed, and a total such as `✓ All 3 hosts built`. If any build fails, the command exits with status 1. `--quiet` hides the output of the Nix builds themselves.

Builds run on the machine running `snow`. To build for another architecture, see [Build for other architectures](/snowglobe/architectures/).

Run `snow build --help` for all options.

## snow deploy

`snow deploy` deploys the selected hosts over SSH. For each host it builds the system locally, skips the host if it already runs that system, copies the closure, plans whether the host needs a reboot, runs the host's hooks, activates the new system and reboots the host when needed. Hosts deploy phase by phase, and a failure stops later phases. [How a deployment works](/snowglobe/how-deploy-works/) describes each step.

```bash
snow deploy                    # every host except those in the phase "never"
snow deploy web1 web2          # these two hosts
snow deploy --phase p1         # one phase
snow deploy --dry-run          # build, copy and show what would change
snow deploy --boot db1         # set the new system for the next boot only
```

Before anything starts, `snow` prints the hosts each phase will deploy. While it runs, it shows one row per host with its current step, and each phase ends with a report of one line per host. The final line sums up the deployment, for example `✓ 3 hosts deployed`.

By default `snow deploy` chooses how to activate each host: it switches when that is safe, and otherwise sets the new system for the next boot and reboots, following the host's reboot policy. The flags `--dry-run`, `--switch`, `--boot` and `--test` force a mode instead; at most one of them may be given. [Control reboots and activation modes](/snowglobe/reboots/) explains each mode and when to use it.

Other flags you are likely to need:

- `--user deploy` connects as this SSH user. Without it, `ssh` picks the user from your SSH config, or uses your local user name.
- `--redeploy` deploys hosts even if they already run the built system, for example to retry an activation that partly failed.
- `--concurrency N` limits how many hosts are worked on at once (default 3).
- `--ignore-phases` deploys all selected hosts together instead of phase by phase.
- `--reboot-timeout SECONDS` sets how long to wait for a rebooted host to come back (default 600).
- `--quiet` hides the output of the commands `snow` runs.

Run `snow deploy --help` for all options.

## snow rebuild

`snow rebuild` builds and activates the machine you run it on, like `nixos-rebuild` or `darwin-rebuild`, but with the same reboot check as `snow deploy`. It works on NixOS and nix-darwin, and needs the machine's configuration to be in the flake.

```bash
snow rebuild                 # build and switch this machine
snow rebuild --boot          # use the new system from the next boot
snow rebuild --host web1     # rebuild using the configuration named web1
```

The host defaults to this machine's hostname: `/etc/hostname` on NixOS, `hostname -s` on macOS. `snow rebuild` asks for your sudo password once, before the build starts, so a long build does not end at a password prompt. When a switch would need a reboot, it shows why and lets you choose to reboot now, use the new system from the next boot, switch anyway, or cancel. `--skip-reboot` and `--force` answer that question in advance, which is what you want in scripts. nix-darwin machines can only switch.

Unlike `snow deploy`, `snow rebuild` does not run hooks and does not read `snowglobe.deploy.ignoredFailedUnits`. See [Rebuild the local machine](/snowglobe/rebuild/) for details.

Run `snow rebuild --help` for all options.

## Logs

When standard error is a terminal, every command writes a full log to `/tmp/snow-<timestamp>.log` and prints its path first, as `Logs: /tmp/snow-1762255380.log`. The terminal shows warnings and the end-of-deployment report; the log keeps every step of every host, with timings and complete error output. When standard error is not a terminal, as in CI, the log goes to standard error instead and no file is written.

The log level defaults to `info`. Set `RUST_LOG` to change what the log records (in a terminal, the screen still shows only warnings and the report), using the [`tracing` filter syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html):

```bash
RUST_LOG=debug snow deploy web1
RUST_LOG=snow=debug,snowlib=debug snow deploy web1   # debug output from snow's own code, not its libraries
```

## Exit codes

| Code | Meaning |
|---|---|
| `0` | The command succeeded. For `snow deploy`, every host was deployed or was already up to date, or the only hosts that failed are [optional](/snowglobe/phases/). A `snow rebuild` you cancel at its menu also exits 0. |
| `1` | The command failed. For `snow build` and `snow deploy`, at least one (non-optional) host failed; the error printed last names them, with the step each one failed at. Any other error, such as a flake that does not evaluate or an unknown host, also exits 1. |
| `2` | The command line was invalid, for example an unknown flag or two activation modes at once. |
| `130` | `snow` was interrupted with Ctrl-C. |
| `143` | `snow` was stopped with `SIGTERM`. |

There is no separate exit code for a partial failure: a deployment in which one host of ten failed exits 1, like one in which all of them failed. To find out which hosts failed, read the last error line or the log.
