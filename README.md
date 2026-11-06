# snowglobe

[![CI](https://github.com/luizribeiro/snowglobe/actions/workflows/ci.yml/badge.svg)](https://github.com/luizribeiro/snowglobe/actions/workflows/ci.yml)

snowglobe deploys NixOS and nix-darwin hosts from a flake over SSH. Its CLI,
`snow`, builds each host's system, copies it to the host, activates it, and
reboots the host when the new system needs it. Hosts deploy in ordered phases,
in parallel within a phase, with optional serial groups and lifecycle hooks.
Progress shows as a live board with one row per host.

[![snow push: three hosts in two phases](docs/demos/switch.svg)](https://asciinema.org/a/FG9AARHrB4eEO1jq)

snowglobe is young. Expect breaking changes to the CLI and module options
before 1.0.

## What works today

| Command | What it does |
| --- | --- |
| `snow list` | List hosts in `nixosConfigurations` and `darwinConfigurations` |
| `snow status` | List hosts (live host state is not implemented yet: every host shows `unknown`) |
| `snow build` | Build hosts' systems locally, in parallel |
| `snow plan` | Show what a build would compile from source versus fetch, without building |
| `snow push` | Deploy hosts over SSH |
| `snow rebuild` | Rebuild and activate the machine you run it on |
| `snow bootstrap` | Generate a new host's SSH key, build its image and inject the key |

`build`, `plan` and `push` take host names, or select hosts with one of
`-e/--filter` (a Nix expression over the host's config), `--phase`/`--phases`
or `--tag`/`--tags`. With no hosts, they act on every host. `list` and
`status` take `-e`. snow finds `flake.nix`
in the current directory or a parent; `-f` or `FLAKE_PATH` points it elsewhere.

### `snow push`

For each host, snow builds the system locally, skips the host if it already
runs that system, copies the closure with `nix copy`, plans the activation,
runs `pre-deploy` hooks, activates, reboots if needed, then runs
`post-reboot` and `post-deploy` hooks. A failure stops that host and runs its
`on-failure` hooks. Other hosts keep going.

- **Phases.** Each host can have a phase. Phases run one after another, in name
  order; hosts without one go in the phase `default`. If any host in a phase
  fails, later phases are skipped. `--ignore-phases` pushes everything at once.
- **Concurrency.** `--concurrency N` (default 3) caps how many hosts are active
  at once. Each host moves through its steps on its own, so fast hosts don't
  wait for slow ones.
- **Serial groups.** Within a phase, hosts with the same
  `snowglobe.push.serialGroup` take turns
  through hooks, activation and reboot. Build, copy and planning stay parallel.
  If one fails, the group's remaining hosts are skipped.
- **Hooks.** `snowglobe.hooks.{pre-deploy,post-deploy,post-reboot,on-failure}`
  run commands on the deploying machine or on the host, with timeouts, retries
  and priorities. See [docs/hooks.md](docs/hooks.md).
- **Reboot detection.** snow compares the new kernel and initrd with the
  booted ones. If they match, it dry-activates and checks whether a service
  that can't be safely restarted (udev, audit, sysctl and a few others) would
  restart. If a reboot is needed, snow sets the new system for the next boot,
  reboots the host, and waits for it to come back (`--reboot-timeout`, default
  600 seconds).
- **Reboot policies.** A host's policy is `auto` (the default: reboot only when
  needed), `never` (set the new system for next boot and leave the reboot to
  you) or `always`.
- **Modes.** `--switch`, `--boot`, `--test` and `--dry-run` pick the activation
  mode. `--boot` sets the new system for the next boot and leaves the reboot to
  you. `--skip-reboot` deploys but never reboots. `--switch --force` switches
  even when a reboot is needed or the policy says otherwise. `--redeploy`
  deploys hosts that already run the built system.

On nix-darwin, push only switches. `--boot`, `--test` and `--dry-run` fail,
and there is no reboot detection or reboot. See [docs/push.md](docs/push.md) for the full reference and
[docs/darwin.md](docs/darwin.md) for Darwin setup.

### `snow rebuild`

Like `nixos-rebuild`, for the machine you're on, with the same reboot
detection. Reboot policies don't apply. When a switch needs a reboot, it asks whether to switch anyway, set
the new system for the next boot, reboot now, or cancel.

### `snow bootstrap`

For a new NixOS host: snow generates its SSH host key and prints the matching
age public key. With `--update-sops`, it adds that key to `.sops.yaml` and
re-encrypts the secrets, so the host can decrypt them on first boot. It then
builds the image named by `snowglobe.bootstrap.imageAttr` and injects the key.
Set `snowglobe.bootstrap.provisioner` to `sd-image`, `raw` or `proxmox-vma`;
the default, `manual`, stops with an error after the build. `--target` writes
the image to a block device or a Proxmox host.

### Modules

`nixosModules.default` and `darwinModules.default` install
`snowglobe-activate`, the helper snow activates and reboots through, and give
the users in `snowglobe.push.allowedUsers` passwordless sudo for it alone. They
also define the hook, serial group and bootstrap options.

## Install

Run it without installing:

```bash
nix run github:luizribeiro/snowglobe -- list
```

Or install it into your profile:

```bash
nix profile install github:luizribeiro/snowglobe
```

## Quickstart

Add snowglobe to your flake and import the module on each host:

```nix
{
  inputs.snowglobe.url = "github:luizribeiro/snowglobe";

  outputs = { nixpkgs, snowglobe, ... }: {
    nixosConfigurations.web1 = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        snowglobe.nixosModules.default
        ./hosts/web1.nix
      ];
    };
  };
}
```

On nix-darwin, import `snowglobe.darwinModules.default` instead.

In the host's config, let your deploy user run the helper:

```nix
{
  snowglobe.push.allowedUsers = [ "deploy" ];
  # nix copy as a non-root user needs a trusted user (or signed paths)
  nix.settings.trusted-users = [ "deploy" ];
}
```

snow connects to `networking.hostName`, plus `networking.domain` when set,
using your SSH config. Deploy the module once with `nixos-rebuild`, then from
the flake's directory:

```bash
snow list                 # the hosts snow sees
snow push --dry-run web1  # build, copy and show what would change (NixOS only)
snow push web1            # deploy it
snow push                 # deploy every host
```

`--user` picks the SSH user when your SSH config doesn't.

Phases, tags and reboot policies are read from `core.pushPhase`, `core.tags`
and `core.rebootPolicy` in each host's config. snowglobe's modules don't
declare these options yet, so declare them in your own module:

```nix
{ lib, ... }:
{
  options.core = {
    pushPhase = lib.mkOption { type = lib.types.nullOr lib.types.str; default = null; };
    tags = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ ]; };
    rebootPolicy = lib.mkOption {
      type = lib.types.enum [ "auto" "never" "always" ];
      default = "auto";
    };
  };
}
```

## Demos

Recorded from real pushes between VMs in the NixOS tests.

A host that must reboot is set to boot the new system, then rebooted:

[![reboot](docs/demos/reboot.svg)](https://asciinema.org/a/C3DbCPc7hX4bzUiW)

Two hosts in a serial group take turns:

[![serial-group](docs/demos/serial-group.svg)](https://asciinema.org/a/6mFMCpWYPMuoNSfW)

One host fails to activate while the others succeed:

[![activation-failure](docs/demos/activation-failure.svg)](https://asciinema.org/a/Dr9UFyXxR6QaisGD)

`snow rebuild` reboots into the new system:

[![rebuild-reboot](docs/demos/rebuild-reboot.svg)](https://asciinema.org/a/7Tlyl2BUHoinFGZK)

More on [the demos page](docs/demos.md).

## How it compares

- **`nixos-rebuild --target-host`** deploys one NixOS host per run and ships
  with NixOS. snowglobe deploys many hosts per run and adds phases, hooks,
  reboot detection and nix-darwin hosts.
- **deploy-rs** deploys many flake hosts per run and has magic rollback: if the
  host can't confirm a connection after activation, it rolls back. It can also
  deploy profiles other than the system. snowglobe has neither.
- **colmena** deploys many hosts in parallel, selects them by tag, can build
  on the target, and uploads secrets outside the Nix store. snowglobe has tags
  but none of the others.

What snowglobe adds:

- ordered phases that stop on failure, and serial groups
- lifecycle hooks
- automatic reboot detection with per-host policies
- a live board with one row per host
- NixOS and nix-darwin hosts in one push

What it lacks: rollback, secrets upload, building on the target, and setups
without flakes. It is also much newer than the others.

## Security

The activation helper puts one auditable command in sudoers instead of
`NOPASSWD: ALL`. That doesn't make the deploy user less powerful: anyone in
`allowedUsers` can activate a system they built, so they are root-equivalent
on the host, as with any push tool. Use a dedicated deploy user and guard its
SSH key like a root key. See [docs/activation-helper.md](docs/activation-helper.md).

## Roadmap

None of this exists yet:

- **Signed deployments.** The host verifies a signed statement before it
  activates a system, so the deploy user is no longer root-equivalent.
- **`snow init`.** Onboarding that sets up a flake and hosts for snowglobe.
- **Pull-based agents.** An agent on each host fetches and applies its own
  configuration.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in snowglobe by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
