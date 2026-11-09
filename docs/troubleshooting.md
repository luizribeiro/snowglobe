---
title: "Troubleshooting"
group: "Guides"
weight: 80
description: "How to read a failed deployment, and what to do about the errors you are most likely to see."
---

This guide helps you find out why a deployment failed and what to change. It starts with how to read the report and the log, then goes through common problems by the message or symptom you see.

## Read the report and the log

Each phase of a deployment ends with a report, one line per host. A host that failed shows the step it failed at and the line of the error that matters most, followed by the lines leading up to it:

```text
✗ Phase p1 (1/2 failed in 0:41)
  ✓ web1 (switch)
  ✗ web2 at activate: activated, but storage-agent.service is failing
```

The step names are `build`, `check` (reading which system the host runs), `copy`, `plan` (deciding whether the host needs a reboot), `pre-deploy`, `activate`, `reboot` (which includes `post-reboot` hooks) and `post-deploy`. A host marked `⊘ … skipped, serial group '…' halted after … failed` was never started because another host in its [serial group](/snowglobe/cluster/) failed first. The step tells you where to look: a failure at `build` happened on your machine, while one at `activate` happened on the host.

After the last phase, `snow` prints a total such as `✗ 2/3 deployed, 1 failed`, and then the error that ended the command, naming each failed host and its step:

```text
✗ 1 of 3 hosts failed: web2 (activate)
  help: See the per-host errors above.
```

The report keeps each error to a few lines. When there is more, the last line says where to find it, for example `… 7 more lines in /tmp/snow-1762255380.log`. When you run `snow` in a terminal, it writes the full log to `/tmp/snow-<timestamp>.log` and prints the path when it starts (`Logs: /tmp/snow-1762255380.log`). When standard error is not a terminal, as in CI, the log goes to standard error instead.

The log records every step of every host with its timing, so it is the place to look when the report is not enough. Each failure has a line of the form `<host>: failed at <step>: <error>`:

```bash
grep 'failed at' /tmp/snow-1762255380.log
```

For more detail in the log, set `RUST_LOG` (the terminal still shows only warnings and the report):

```bash
RUST_LOG=debug snow deploy web2
```

To see the output of the commands `snow` runs (the Nix build, `nix copy`, the activation) as they happen, leave out `--quiet`. Retrying a single host is usually the quickest way to iterate: `snow deploy web2`.

## "sudo: a password is required"

`snow` never types a password on a remote host. It runs the activation helper with `sudo` over a non-interactive SSH session, so sudo must let the deploy user run the helper without one. When it does not, the host fails with `sudo: a password is required` as its cause, usually at `plan` with a longer message:

```text
Could not tell whether the new system needs a reboot: `sudo snowglobe-activate '/nix/store/…' dry-activate` failed with exit status: 1. If sudo refused it, import the snowglobe module on the host and add the SSH user to snowglobe.deploy.allowedUsers.
```

It can also fail at `activate` or `reboot`. In each case, check the following, in order:

1. The host imports `snowglobe.nixosModules.default` (or `darwinModules.default` on a Mac).
2. The user `snow` connects as is listed in [`snowglobe.deploy.allowedUsers`](/snowglobe/options/#snowglobedeployallowedusers). If you deploy with `--user deploy`, that is `deploy`; otherwise it is whatever your SSH config or local user name gives.
3. The configuration with that setting has been activated on the host. The sudo rule only exists once the host runs it, so the first activation has to be done by hand, for example with `sudo nixos-rebuild switch`.
4. The rule applies. This command prints the helper's path if it does, and fails without prompting if it does not:

   ```bash
   ssh deploy@web1 sudo -n -l /run/current-system/sw/bin/snowglobe-activate
   ```

5. The host uses the regular `sudo`. The module adds its rule through `security.sudo`, so a host that uses `sudo-rs` gets no rule; see [Hosts that use sudo-rs](/snowglobe/deploy-user/#hosts-that-use-sudo-rs) for the rule to add yourself.

[Set up a deploy user](/snowglobe/deploy-user/) walks through the whole setup, and [Security model](/snowglobe/security/) explains why the helper exists.

## "snowglobe-activate not found … falling back"

This warning means the host does not have the activation helper yet:

```text
! snowglobe-activate not found on web1, falling back to direct activation
```

On NixOS, `snow` then activates by running `sudo nix-env --profile /nix/var/nix/profiles/system --set …` and `sudo …/bin/switch-to-configuration` directly. That only works if you connect as root or as a user who may run those commands with sudo without a password, which is what the helper is designed to avoid. Import the snowglobe module on the host, add the deploy user to `snowglobe.deploy.allowedUsers`, and activate the configuration by hand once; later deployments pick up the helper on their own.

nix-darwin hosts have no fallback: without the module, activation fails. See [Deploy nix-darwin hosts](/snowglobe/darwin/).

## A host fails with "activated, but … is failing"

A NixOS switch fails when any systemd unit is failing afterwards, even a unit the deployment did not change: the switch starts its targets again, which reruns units that were already failing. By then the new system is active, and the report says so:

```text
  ✗ web2 at activate: activated, but storage-agent.service is failing
```

If the unit was failing before the deployment too, the message adds `(it was already failing before this deployment)`. When several units fail, the line says `activated, but 2 units are failing` and lists them underneath.

The host still counts as failed, so later phases do not run unless the host is [optional](/snowglobe/options/#snowglobedeployoptional). Look at the unit on the host before you deploy again:

```bash
ssh web2 systemctl status storage-agent.service
ssh web2 journalctl -u storage-agent.service -n 50
```

If the unit is broken for reasons unrelated to the deployment and you want deployments to go ahead anyway, list it in the host's configuration:

```nix
{
  snowglobe.deploy.ignoredFailedUnits = [ "storage-agent.service" ];
}
```

The host is then deployed when the listed units are the only failing ones and the switch failed for no other reason. Its report line names them, as in `✓ web2 (switch, ignored failing storage-agent.service)`. A listed unit is ignored even when this deployment is what broke it, so keep the list short. The option only affects `switch` and `test` activations on NixOS; `snow rebuild` gives the same message but does not read the list.

## A host fails to activate

[![One host fails to activate while the others succeed](/snowglobe/activation-failure.svg)](https://asciinema.org/a/NRfbtSt4AyH53Xsy)

When activation fails, only that host fails. Other hosts in the same phase carry on, and later phases are skipped. The report shows the command `snow` ran and the end of its output, which normally contains the reason, such as a unit that failed to start or an activation script that exited with an error. The host's on-failure hooks run.

Keep in mind that a failed switch may already have changed the host. NixOS records the new system as current before it starts services, so a host that failed at `activate` may run the new system with some services not started.

If the cause is in the configuration, fix it and deploy again as usual. If the configuration was fine and the failure was temporary, for example a service that depends on something that was briefly unavailable, deploy the same configuration again with `--redeploy`. Without it, `snow` may skip the host as already up to date, because the host now runs the system you built:

```bash
snow deploy --redeploy web2
```

To go back to the previous system instead, see [Roll back a bad deployment](/snowglobe/rollback/).

## A host is unreachable

[![A host is off the network: ssh times out and only that host fails](/snowglobe/unreachable.svg)](https://asciinema.org/a/cX25pebSCFgr3uDE)

When `snow` cannot reach a host, that host fails with ssh's own error, such as `ssh: connect to host web2 port 22: Connection timed out`, and the other hosts in the phase are deployed as usual. `snow` gives up on a connection after 10 seconds, so an offline host does not hold up the deployment for long. Before the failure you may see the warning `could not read installed systems, deploying anyway`: `snow` could not check whether the host was up to date, and the copy that follows is what fails.

Check that you can reach the host the way `snow` does. `snow` connects to the host's `networking.hostName`, followed by `.` and `networking.domain` when that is set, through your SSH config, in batch mode so that ssh never prompts:

```bash
ssh -o BatchMode=yes web2.example.com true
```

Common causes:

- The name does not resolve, or resolves to the wrong address. Add a `Host` entry to `~/.ssh/config`, or set `networking.domain`.
- `Permission denied (publickey)`: the key is not in your agent or SSH config, or the user is wrong. Use `--user` or a `User` line in your SSH config.
- `Host key verification failed.`: the host key is not in `known_hosts` yet, or it changed. Connect once by hand to accept it.
- SSH listens on another port. Set `snowglobe.deploy.sshPort`, or a `Port` line in your SSH config.

[Connect to your hosts](/snowglobe/connect/) covers addresses, users and ports in detail. For a host that is often offline, such as a laptop, set `snowglobe.deploy.optional = true`: it still fails, but it no longer stops later phases or fails the deployment.

## The ssh-agent does not answer

[![The ssh-agent never answers, so the deployment stops at once](/snowglobe/hung-agent.svg)](https://asciinema.org/a/eOyxRZmJRZ1fQB7f)

If `SSH_AUTH_SOCK` points at an agent that accepts connections but never replies, every ssh call would hang. `snow` checks the agent with `ssh-add -l` before it touches any host, and stops the deployment if the agent does not answer within 10 seconds:

```text
✗ The ssh-agent did not answer within 10s
  help: Every ssh connection would hang on it. Check it with `ssh-add -l`, then restart the agent or unset `SSH_AUTH_SOCK`.
```

Run `ssh-add -l` yourself. If it hangs too, restart the agent, or unlock the hardware key or password manager that backs it. To deploy with key files only, unset the variable for one command:

```bash
env -u SSH_AUTH_SOCK snow deploy web1
```

An agent that cannot be reached at all is not fatal. In that case `snow` warns `the ssh-agent can't be reached, ssh will only try key files` and carries on.

## A hook fails

[![A pre-deploy hook fails, so its host is never activated](/snowglobe/hook-failure.svg)](https://asciinema.org/a/SjhKcts8A6EwwRoG)

A hook fails when its command exits with a non-zero status or runs past its `timeout` (`Hook timed out after 300 seconds`), after any `retries`. The host then fails at the hook's stage, and the report shows the last line the hook wrote to standard error:

```text
  ✗ db1 at pre-deploy: storage on db1 is degraded
```

A failed `pre-deploy` hook means the host was never activated. The log names the hook and stage, as in `db1: hook 'wait-storage-healthy' (pre-deploy) failed with exit code 1`, and the full error includes the command. To debug it, run the command by hand where it runs: on your machine for `location = "local"`, or on the host, as the deploy user, for `location = "remote"`. Hooks receive `SNOW_HOSTNAME`, `SNOW_PHASE` and `SNOW_STORE_PATH`, so set those when you try the command:

```bash
SNOW_HOSTNAME=db1 SNOW_PHASE=p2 /nix/store/…-wait-storage-healthy/bin/wait-storage-healthy
```

To list a host's hooks of one stage, evaluate them:

```bash
nix eval .#nixosConfigurations.db1.config.snowglobe.hooks.pre-deploy --json
```

For a hook that may fail without stopping the host, such as a notification, set `continueOnFailure = true`. [Write deployment hooks](/snowglobe/hooks/) explains stages, retries and timeouts.

If one of a hook's `environmentFiles` cannot be read, the host fails at that hook's stage with `Failed to read hook environment file '<path>'`, unless the hook has `continueOnFailure = true`. These files are read on the machine running `snow`, for remote hooks too, so they must exist there.

### A local hook's program is missing

[![A local hook's program is missing, so the deployment stops before building](/snowglobe/missing-hook.svg)](https://asciinema.org/a/uH01eTmwST7t8kNE)

Before building anything, `snow` checks that every store path a local hook's command refers to exists on your machine. If one is missing, the deployment stops before any host is touched:

```text
✗ Hook 'wait-storage-healthy' of host 'db1' references '/nix/store/…-wait-storage-healthy', which is not present on this machine
  help: Local hooks run on the machine running snow. Use `localPkgs` (not `pkgs`) for packages in local hook commands so snow builds them for this machine. The path may also have been garbage-collected.
```

The usual cause is a local hook that interpolates a package from `pkgs`, which is built for the host rather than for your machine. Use `localPkgs` instead:

```nix
{ config, localPkgs, ... }:
{
  snowglobe.hooks.pre-deploy.drain = {
    location = "local";
    command = "${localPkgs.kubectl}/bin/kubectl drain ${config.networking.hostName}";
  };
}
```

## A rebooted host never comes back

[![A rebooted host never comes back before the deadline](/snowglobe/reboot-deadline.svg)](https://asciinema.org/a/eFy1uI5b9rnfI0jt)

After it asks a host to reboot, `snow` waits for the host to answer over SSH with a new boot ID. If that does not happen within `--reboot-timeout` (600 seconds by default), the host fails at `reboot` with a message such as `host did not come back online within 600s`, followed by the last connection error. If the host answers but still has the same boot ID, the message is `boot ID is unchanged, so the host never rebooted` instead.

At that point the new system is already set as the host's next boot, so the host starts the new system whenever it does come back. Its `post-reboot` and `post-deploy` hooks did not run, but its on-failure hooks did, and later phases were skipped.

What to do depends on the host:

- If it is still booting, for example because of a slow firmware or a disk check, wait for it, and raise the deadline for future deployments: `snow deploy --reboot-timeout 1200`.
- If it does not boot, use its console. Select the previous generation in the boot menu to get back to a working system, then see [Roll back a bad deployment](/snowglobe/rollback/).
- If the boot ID never changed, the reboot was not carried out. Check that the deploy user can run `sudo snowglobe-activate reboot` on the host.

Once the host is back, deploy it again. If it came back on the new system, the deployment reports it as unchanged and runs none of its hooks, so run any health checks by hand.

## A host is skipped as already up to date

A host shows `✓ web1 (unchanged)`, and the log says `web1: already running the built system, skipping`. This happens when the host already runs the system that was just built and would also boot it. `snow` does not copy anything, run hooks or reboot such a host, so a deployment that changes nothing does not drain or restart your machines.

If you expected a change, the change did not reach the built system. Check that you edited the configuration of the right host and that the files are part of the flake: in a Git repository, Nix only sees files that Git tracks, so a new file has to be added with `git add` first. You can compare what would be built with what the host runs:

```bash
nix build .#nixosConfigurations.web1.config.system.build.toplevel --print-out-paths
ssh web1 readlink -f /run/current-system
```

To activate a host again although nothing changed, for example after an activation that partly failed, use `--redeploy`:

```bash
snow deploy --redeploy web1
```

## Builds fail for another architecture

`snow` builds every host on the machine you run it on. If a host has a different architecture, Nix stops with an error saying that a system such as `aarch64-linux` is required to build a derivation and that the current machine is a different one. Nothing is copied to the host.

The usual fixes are to give Nix a remote builder of that architecture, or to let your machine build for it through emulation; cross-compiling is a third option. Building on the target host itself is not supported. [Build for other architectures](/snowglobe/architectures/) explains each setup. To check a fix without deploying, build that host with `snow build` first:

```bash
snow build nas1
```

A related error is `nix-darwin can only switch, so your-mac can't boot` (or `test`, or `dry-activate` for `--dry-run`). nix-darwin has no other activation mode, so `snow` refuses before it builds anything. Deploy nix-darwin hosts without `--boot`, `--test` or `--dry-run`.

## Still stuck

If none of this helps, open an issue on [GitHub](https://github.com/luizribeiro/snowglobe/issues) with the command you ran, the report, and the relevant part of the log. Remove host names, addresses and secrets you do not want to share.
