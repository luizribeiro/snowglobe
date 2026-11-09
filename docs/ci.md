---
title: "Run from CI"
group: "Guides"
weight: 65
description: "Deploy NixOS and nix-darwin hosts from a CI job with a dedicated deploy user, a pinned snow and pinned host keys."
---

This guide sets up a CI job that runs `snow deploy` against your hosts, and an optional check that runs on pull requests. The examples use GitHub Actions, but every step translates directly to other CI systems: the job needs Nix, an SSH key, the hosts' public keys and network access to the hosts.

## How snow behaves in CI

snow decides whether it is interactive by checking whether standard error is a terminal. In a CI job it isn't, and snow adapts:

- **No live board.** The per-host progress rows are only drawn on a terminal. You still get the phase overview at the start and the report at the end of each phase, on standard output.
- **Logs go to standard error** instead of a log file. In a terminal, snow writes `/tmp/snow-<timestamp>.log` and prints its path; in CI there is no file, and the same log lines appear in the job output. Set `RUST_LOG=debug` for more detail.
- **Subprocess output is logged.** By default the output of `nix build`, `nix copy`, activation and hooks goes into that log, so it shows up in the job output. Pass `--quiet` (`-q`) to leave it out and keep only snow's own lines and the reports; the error for a failed step still includes the output of the command that failed.
- **Nothing prompts.** Every SSH connection runs with `BatchMode=yes`, so ssh fails instead of asking for a password, a key passphrase or confirmation of an unknown host key. That is why the key must be in an agent (or unencrypted on disk) and the host keys must be in `known_hosts` before the deployment starts.

snow never asks a question during `snow deploy`, so it needs no flags to run unattended. (`snow rebuild` is different: it may ask for a sudo password and, when a reboot is needed, shows a menu. It is meant for the machine you are sitting at, not for CI.)

## Exit codes

`snow deploy` exits `0` when every selected host deployed or was already up to date; hosts marked `snowglobe.deploy.optional` may have failed. It exits `1` when anything else failed: a host that isn't optional, evaluation, or a check before the deployment (such as an unresponsive ssh-agent, a missing local hook program or an unsupported mode for a nix-darwin host). An invalid command line exits `2`. The [Commands reference](/snowglobe/commands/#exit-codes) has the full table, including the codes for interruptions.

A non-zero exit fails the CI step, which is what you want: the report above it names each failed host and the step it failed at.

## Create a CI deploy user

Give CI its own user on each host rather than reusing a person's account. A dedicated user can be revoked on its own, shows up as itself in the hosts' logs, and its key never lives on a laptop.

```nix
{
  users.users.ci-deploy = {
    isNormalUser = true;
    openssh.authorizedKeys.keys = [
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyOnlyReplaceMe ci-deploy"
    ];
  };

  snowglobe.deploy.allowedUsers = [ "ci-deploy" ];

  # nix copy as a non-root user only imports unsigned paths from a trusted user
  nix.settings.trusted-users = [ "ci-deploy" ];
}
```

On a Mac, create the account and use the same `allowedUsers` and `trusted-users` settings; [Deploy nix-darwin hosts](/snowglobe/darwin/) covers the details, including Determinate Nix. [Set up a deploy user](/snowglobe/deploy-user/) explains each setting.

A user in `allowedUsers` can run any system closure as root through the activation helper, so it is root-equivalent on that host. Treat the CI secret that holds its key as a root credential: limit who can edit the workflow and who can run it, and consider restricting the key to your runners' addresses with an OpenSSH `from="..."` option in front of the key. See the [Security model](/snowglobe/security/).

Apply the configuration once with your existing access (`nixos-rebuild switch`, `darwin-rebuild switch`, or a `snow deploy` as yourself) before the first CI run, so the user and sudo rule exist.

## Generate the key and pin the host keys

Generate a key pair without a passphrase for CI, and store the private key as a CI secret (here, `DEPLOY_SSH_KEY`):

```bash
ssh-keygen -t ed25519 -N "" -C ci-deploy -f ci-deploy
```

Put `ci-deploy.pub` in the user's `authorizedKeys`, store the contents of `ci-deploy` in the secret, then delete the local copy.

Next, collect the hosts' public keys into a `known_hosts` file. Do not disable host key checking with `StrictHostKeyChecking=no`: that would let anyone who can intercept the runner's connection receive a closure and run it as root. Get the keys from a source you trust, such as a machine that has already verified them:

```bash
ssh-keygen -F web1.example.com >> ci_known_hosts
ssh-keygen -F db1.example.com >> ci_known_hosts
```

Or read each host's key directly on the host (`cat /etc/ssh/ssh_host_ed25519_key.pub`) and write lines of the form `web1.example.com ssh-ed25519 AAAA...`. The names must match the address snow connects to, which is the host's `networking.hostName`, followed by `.` and `networking.domain` when that is set. Commit the file to the repository (host public keys aren't secret) or store it as a CI variable.

## Install snow in the job

Run snow from its flake with `nix run`, pinned to a release tag or a commit so that a new snowglobe version never reaches your hosts unannounced. Replace `vX.Y.Z` with a tag from the [releases page](https://github.com/luizribeiro/snowglobe/releases):

```bash
nix run github:luizribeiro/snowglobe/vX.Y.Z -- deploy           # a release tag
nix run 'github:luizribeiro/snowglobe?rev=<commit>' -- deploy   # a commit
```

If your flake already has snowglobe as an input (it does if your hosts import its modules), you can instead run the exact revision locked in your `flake.lock`. That keeps the CLI and the modules on your hosts at the same version:

```bash
nix run --inputs-from . snowglobe -- deploy
```

snowglobe is pre-1.0, and minor releases may change the CLI and module options. Pin the version, read the [Changelog](/snowglobe/changelog/) before updating, and update it in a pull request like any other dependency.

## A GitHub Actions workflow

This workflow deploys every host when `main` changes, one run at a time:

```yaml
name: Deploy

on:
  push:
    branches: [main]
  workflow_dispatch:

concurrency:
  group: deploy
  cancel-in-progress: false

jobs:
  deploy:
    runs-on: ubuntu-latest
    environment: production
    env:
      SSH_AUTH_SOCK: /tmp/ssh_agent.sock
    steps:
      - uses: actions/checkout@v4

      - uses: cachix/install-nix-action@v31

      - name: Load the deploy key
        env:
          DEPLOY_SSH_KEY: ${{ secrets.DEPLOY_SSH_KEY }}
        run: |
          ssh-agent -a "$SSH_AUTH_SOCK" > /dev/null
          ssh-add - <<< "$DEPLOY_SSH_KEY"

      - name: Configure ssh
        run: |
          mkdir -p ~/.ssh
          cp ci_known_hosts ~/.ssh/known_hosts
          cat >> ~/.ssh/config <<'EOF'
          Host *.example.com
            User ci-deploy
          EOF

      - name: Deploy
        run: nix run --inputs-from . snowglobe -- deploy
```

Some notes on the choices made here:

- **`concurrency` with `cancel-in-progress: false`** queues a second run instead of cancelling the first. Cancelling a job stops snow wherever it is, which can be in the middle of a host's activation, and two deployments to the same hosts at once would race each other.
- **`environment: production`** lets you protect the deploy secret with GitHub's environment rules, such as required reviewers or branch restrictions. Store `DEPLOY_SSH_KEY` as an environment secret there. Passing it through the step's `env`, rather than writing `${{ secrets.DEPLOY_SSH_KEY }}` into the script, keeps the key out of the script text and out of every other step's environment.
- **The ssh-agent** started with `-a` keeps running for the rest of the job, and every later step finds it through `SSH_AUTH_SOCK`. Before connecting to any host, snow checks that the agent answers; an agent that never responds stops the deployment at once rather than hanging every host.
- **The SSH user** comes from `~/.ssh/config` here. You can instead pass `--user ci-deploy` to `snow deploy`; it applies to every SSH connection snow makes, including remote hooks. Per-host settings such as a different address or port also belong in `~/.ssh/config`; see [Connect to your hosts](/snowglobe/connect/).
- **The runner must reach the hosts.** A hosted runner usually needs a VPN step first, or you can use a self-hosted runner inside the network.

## Building on the runner

snow builds every host's closure on the machine it runs on and copies it to the host; it never builds on the target. The runner therefore has to be able to build each host's system:

- Hosts with the runner's architecture build directly.
- Hosts with another Linux architecture need emulation or a remote builder on the runner. See [Build for other architectures](/snowglobe/architectures/).
- nix-darwin hosts must be built on macOS. Run their deployment in a separate job on a macOS runner (for example `runs-on: macos-latest`, with a tag such as `snow deploy --tag macs`), or give the Linux runner a Darwin remote builder.

A binary cache that the runner can read, and that your earlier builds push to, saves most of the build time. Without one, every run rebuilds or downloads everything from scratch.

## Check pull requests

Two kinds of check are useful on pull requests, with very different requirements.

**`snow build`** evaluates and builds the selected hosts without connecting to any of them. It needs no secrets and no network access to your hosts, so it is safe to run on every pull request, including ones from forks:

```yaml
      - name: Build all hosts
        run: nix run --inputs-from . snowglobe -- build
```

It exits non-zero if any host fails to build, and its report lists each host with its store path or error. Like `snow deploy`, `snow build` with no host names selects every host outside the `never` phase, nix-darwin hosts included, and a Linux runner can't build those without a Darwin builder. Select the hosts the runner can build, for example `snow build --tag servers`, and build the Macs in a job on a macOS runner.

**`snow deploy --dry-run`** goes further: it builds, copies each closure to its host, and runs the new system's `dry-activate` there to show which services would restart. It doesn't change the running system or the boot entry, and runs no hooks. It does need the deploy key, network access, and root on each host through the activation helper, so only run it for pull requests from branches in your own repository, and only in a job that can read the deploy secret, such as one in a protected environment whose rules allow pull request branches. Don't pass `--quiet` to a dry run: what would change is in the activation output, which `--quiet` hides.

```yaml
      - name: Show what would change
        if: github.event.pull_request.head.repo.full_name == github.repository
        run: nix run --inputs-from . snowglobe -- deploy --dry-run --tag servers
```

nix-darwin hosts only support switching, and a dry run that selects one fails before building anything. Select only NixOS hosts for a dry run, here with a tag.

## Troubleshooting

- **`Permission denied (publickey)`**: the agent has no key, the key isn't in the user's `authorizedKeys`, or snow connected as the wrong user. Run `ssh-add -l` in the job to see the loaded keys.
- **`Host key verification failed`**: the host's key isn't in `~/.ssh/known_hosts` under the name snow connects to. Compare the name in the error with your file.
- **`sudo: a password is required`**: the configuration with the CI user in `allowedUsers` isn't active on that host yet. Apply it once by hand.
- **`the ssh-agent can't be reached, ssh will only try key files`**: a warning that `SSH_AUTH_SOCK` points to a socket with no agent behind it, usually because the agent step didn't run or used a different socket path. The deployment continues without the agent and then fails with `Permission denied (publickey)`.
- **`The ssh-agent did not answer within 10s`**: an agent is listening but doesn't respond, and snow stops before connecting to any host. Restart the agent in the job.

More in [Troubleshooting](/snowglobe/troubleshooting/).
