# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Before 1.0, minor releases may break the CLI, the library API and the module
options.

## [Unreleased]

## [0.1.0] - YYYY-MM-DD

First release. snowglobe is pre-1.0: the CLI, module options and flake
interface may change between minor releases. It is used in production on one
homelab of NixOS (x86_64, aarch64) and nix-darwin hosts.

Release binaries: static Linux (x86_64 and aarch64, musl) and macOS (x86_64
and aarch64), with `SHA256SUMS`.

### Added

#### Commands

- `snow list`: hosts in the flake, with phase and tags. `--json` for
  scripts.
- `snow build`: build, don't deploy.
- `snow push`: deploy over SSH. See [Push](#push).
- `snow rebuild`: `nixos-rebuild` for the machine you're on. Asks for sudo
  once at the start. If a switch needs a reboot, you get a menu: reboot
  now, next boot, switch anyway, or cancel.
- Hosts by name, `--phase(s)`, `--tag(s)` or `-e <nix expression>`.
- `-f` or `FLAKE_PATH` if `flake.nix` isn't in the current directory or
  above it.
- Logs go to `/tmp/snow-<timestamp>.log` in a terminal, stderr otherwise.
- `snow --version`.

#### Unstable commands

Not in release builds. May change or break any time. Try them with
`nix run github:luizribeiro/snowglobe#snow-unstable -- <command>` or
`cargo build --features unstable`.

- `snow plan`: what a build would compile from source vs. fetch from a
  substituter, with download sizes, without building. `--fail-on-build`
  exits 2 if anything non-trivial would build. The last plan is saved under
  `$XDG_STATE_HOME/snowglobe/`.
- `snow status`: hosts as a table or `--json`. Doesn't query hosts yet, so
  every status is `unknown`.
- `snow bootstrap`: new host setup. Generates its ed25519 SSH host key,
  optionally adds its age key to `.sops.yaml` (`--update-sops`), builds its
  image with the host's provisioner (`manual`, `sd-image`, `raw` or
  `proxmox-vma`). `--target` writes it to a block device or a Proxmox host.

#### Push

Details in [docs/push.md](docs/push.md) and [docs/hooks.md](docs/hooks.md).

- Phases (`snowglobe.deploy.phase`), in order. A failed phase stops the rest.
  `--ignore-phases` to push all at once.
- `never` phase, for hosts you only push by name.
- Optional hosts (`snowglobe.deploy.optional`), e.g. a laptop that's often off.
- Serial groups (`snowglobe.deploy.serialGroup`).
- Hooks (`snowglobe.hooks`): `pre-deploy`, `post-deploy`, `post-reboot`,
  `on-failure`, local or remote.
- Reboot detection (new kernel or initrd, or a critical service would
  restart), plus `auto`/`never`/`always` reboot policies per host. Policies
  win over `--switch`. `--boot`, `--test` and `--dry-run` ignore them.
- Modes to switch, boot next time, test or dry-run.
- Unchanged hosts are skipped. `--redeploy` to push them anyway.
- A host whose units are failing after its switch fails, with a message
  saying the new system is active and naming the failing units.
  `snowglobe.deploy.ignoredFailedUnits` lists units that don't fail it.
- Hosts don't wait on each other.
- `snowglobe.deploy.sshPort` for hosts on another port. ssh never forwards
  your agent.
- Live board with a row per host, and a report at the end of each phase.

#### Modules

- `nixosModules.default` and `darwinModules.default`.
- `snowglobe-activate` on each host. Users in `snowglobe.deploy.allowedUsers`
  can sudo it without a password, nothing else. Takes only store paths that
  are a system.
- NixOS hosts without the module still work, activated directly, with a
  warning. Macs need it.

#### Testing

- NixOS VM tests that run `snow push` and `snow rebuild` against real VMs:
  reboots, failures, serial groups. The same tests record the terminal demos.
  The recorder is exposed as the flake's `lib`.
- Unit tests for the Nix modules, as flake checks.

### Migrating from pre-release versions

- Phase, tags and reboot policy are read only from `snowglobe.deploy.phase`,
  `.tags` and `.rebootPolicy`, not from `core.pushPhase`, `core.tags` and
  `core.rebootPolicy`.
- The hook stages `pre-activate` and `post-activate` are now `pre-deploy` and
  `post-deploy`. The old names fail evaluation and point to the new ones.

[Unreleased]: https://github.com/luizribeiro/snowglobe/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/luizribeiro/snowglobe/releases/tag/v0.1.0
