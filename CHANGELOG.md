# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Before 1.0, minor releases may break the CLI, the library API and the module
options.

## [Unreleased]

## [0.1.0] - YYYY-MM-DD

The first release: push-based deployment of NixOS and nix-darwin hosts from a
flake, with phases, hooks and reboot handling.

### Added

#### Commands

- `snow list` lists the hosts in a flake's `nixosConfigurations` and
  `darwinConfigurations` with their phase and tags. `--json` prints each
  host's Nix platform, phase, tags, reboot policy, serial group and optional
  flag, for scripts.
- `snow build` builds hosts' systems in parallel without deploying them.
- `snow push` deploys hosts over SSH (see below).
- `snow rebuild` rebuilds the host it runs on, with the same reboot detection
  as `snow push`: it lists why a reboot is needed and asks whether to reboot,
  set the new system for the next boot, or switch anyway.
- Hosts are selected by name, or by `--phase`/`--phases`, `--tag`/`--tags`
  or a Nix expression over a host's config (`--filter`).
- The flake is found from the current directory upwards, or given with `-f`
  or `FLAKE_PATH`.
- `snow --version`.

#### Unstable commands

Left out of release builds; they may change or break at any time. Run them
with `nix run github:luizribeiro/snowglobe#snow-unstable -- <command>` or
build with `cargo build --features unstable`.

- `snow plan` shows, without building, what building the selected hosts would
  compile from source and what it would fetch from a substituter, with
  download sizes. `--fail-on-build` exits 2 if anything non-trivial would be
  built. The last plan is saved under `$XDG_STATE_HOME/snowglobe/`.
- `snow status` lists hosts as a table or `--json`; it doesn't query them
  yet, so every status reads `unknown`.
- `snow bootstrap` provisions a new host: it generates its SSH host keys,
  optionally adds its age key to `.sops.yaml` (`--update-sops`), and builds an
  image with the host's provisioner (`manual`, `sd-image`, `raw` or
  `proxmox-vma`), which `--target` writes to a block device or deploys to a
  Proxmox host.

#### Push

- **Breaking for pre-release users:** snow reads a host's phase, tags and
  reboot policy only from the module's `snowglobe.push.phase`, `.tags` and
  `.rebootPolicy`, no longer from `core.pushPhase`, `core.tags` and
  `core.rebootPolicy`, which your own module had to declare. Move those
  settings to the new options.
- Phases (`snowglobe.push.phase`) deploy one after another, in name order
  with numbers by value (`p2` before `p10`), and hosts without a phase deploy
  last; a failed phase stops the later ones. `--ignore-phases` deploys every
  selected host at once. Phase names may contain only letters, digits, `-`
  and `_`, and `default` is reserved.
- Hosts in the phase `never` are pushed, built or planned only when named or
  selected with `--phase never`.
- Optional hosts (`snowglobe.push.optional`) may fail without failing their
  phase or the push; the report marks their failures `(optional)`.
- Each host runs build, copy, plan, activate and reboot on its own, in
  parallel with the others, up to `--concurrency`.
- Hosts already running and booting the built system are skipped;
  `--redeploy` deploys them anyway.
- Serial groups (`snowglobe.push.serialGroup`) deploy their hosts one at a
  time within a phase, and stop the group when one of them fails.
- Lifecycle hooks (`snowglobe.hooks`): `pre-deploy`, `post-deploy`,
  `post-reboot` and `on-failure`, run on the deploying machine or on the
  host, with timeouts, retries, priorities, environment files and
  `continueOnFailure`. Local hooks can use packages built for the deploying
  machine through `localPkgs`.
- Reboot detection compares the booted kernel, initrd and init with the new
  system, then checks with `dry-activate` whether critical services would
  restart. Hosts that need a reboot are set to boot the new system and
  rebooted, and snow waits (`--reboot-timeout`) until they come back with a
  new boot ID.
- Per-host reboot policies (`snowglobe.push.rebootPolicy`): `auto`, `never` or
  `always`. Policies take precedence over `--switch`; `--switch --force`
  overrides them and the reboot detection.
- Activation modes: `--switch`, `--boot`, `--test` and `--dry-run`, and
  `--skip-reboot` to only report that a reboot is needed.
- A live terminal view with one row per host, its current step and a
  progress bar per phase, followed by a report per phase that says why each
  failed host failed. `--quiet` hides subprocess output.
- Every run is logged in full to `/tmp/snow-<timestamp>.log`.
- snow checks that the ssh-agent answers before connecting to any host.

#### Modules

- A NixOS module (`nixosModules.default`) and a nix-darwin module
  (`darwinModules.default`) with the push, hooks and bootstrap options.
- The `snowglobe-activate` helper lets the users in
  `snowglobe.push.allowedUsers` activate a system (and, on NixOS, reboot)
  through passwordless sudo for that one command. It accepts only
  `/nix/store/<hash>-<name>` paths that contain a system to activate.
  Hosts without the module are activated directly, with a warning.

#### Testing

- End-to-end NixOS VM tests that run `snow push` and `snow rebuild` against
  VMs, covering reboots, failures and serial groups. They also record the
  terminal demos; the recorder is exposed as the flake's `lib`.
- Unit tests for the Nix modules, run as flake checks.

[Unreleased]: https://github.com/luizribeiro/snowglobe/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/luizribeiro/snowglobe/releases/tag/v0.1.0
