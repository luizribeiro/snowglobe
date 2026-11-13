# snowglobe

[![CI](https://github.com/luizribeiro/snowglobe/actions/workflows/ci.yml/badge.svg)](https://github.com/luizribeiro/snowglobe/actions/workflows/ci.yml) [![Release](https://img.shields.io/github/v/release/luizribeiro/snowglobe)](https://github.com/luizribeiro/snowglobe/releases) [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

Deploy NixOS and nix-darwin hosts, in phases.

snowglobe deploys the NixOS and nix-darwin hosts in a flake over SSH. Its
CLI, `snow`, builds each host, copies it over, activates it, and reboots it
when the new system needs a reboot. Hosts deploy in phases, and progress
shows as one row per host.

![switch](docs/demos/switch.svg)

> [!NOTE]
> snowglobe is pre-1.0. The CLI, module options and flake interface may change between minor releases; see the [changelog](CHANGELOG.md) before upgrading. It is used in production on one homelab of NixOS (x86_64, aarch64) and nix-darwin hosts.

## Features

- [Phases](https://l9o.dev/snowglobe/phases/) that roll out by risk and stop at the first failure.
- [Serial groups](https://l9o.dev/snowglobe/cluster/), so cluster nodes deploy one at a time.
- [Reboot detection](https://l9o.dev/snowglobe/reboots/) with per-host reboot policies.
- [Hooks](https://l9o.dev/snowglobe/hooks/) before and after a deploy, after a reboot, and on failure.
- NixOS and [nix-darwin](https://l9o.dev/snowglobe/darwin/) hosts in the same deployment.
- Reads the flake's existing `nixosConfigurations` and `darwinConfigurations`.
  No extra flake output is needed.

Other tools deploy many hosts too, and some roll out in order or gate on
health checks. snowglobe's reason to exist is the combination: parallel
builds and copies, phases and serial groups in one run, hooks that drain a
node and gate the next one, reboots only when the new system needs one,
and NixOS and nix-darwin hosts side by side. The
[comparison](https://l9o.dev/snowglobe/comparison/) says where another tool
fits better.

## What it doesn't do

- Build on the target. `snow` builds on the machine it runs on, or on remote
  builders Nix is configured to use. colmena, deploy-rs and Clan can build on the target.
- Upload secrets. Use agenix or sops-nix, or a tool that uploads them, such
  as colmena, morph or Clan.
- Roll back on its own. A host that fails stays on its new system until
  you [roll it back](https://l9o.dev/snowglobe/rollback/). deploy-rs and
  devenv Machines (NixOS targets) roll back automatically.
- Work without flakes.
- Pull. Every deployment is pushed over SSH; comin and other pull-based
  tools have each host fetch its own configuration.

## Requirements

- Nix with flakes on the deploying machine, Linux or macOS.
- NixOS or nix-darwin hosts.
- SSH key authentication to each host, with its host key already in
  `known_hosts`. `snow` never prompts for a password.

## Installation

```bash
nix run github:luizribeiro/snowglobe -- list      # run without installing
nix profile add github:luizribeiro/snowglobe      # install
```

To pin a release, use its tag: `github:luizribeiro/snowglobe/vX.Y.Z`.

Builds for x86_64-linux and aarch64-darwin come from the
[snowglobe.cachix.org](https://snowglobe.cachix.org) binary cache, which Nix
asks to use; see [Installation](https://l9o.dev/snowglobe/installation/) for
untrusted users. On aarch64-linux, `snow` compiles from source.

## Getting started

The short version is below; the [tutorial](https://l9o.dev/snowglobe/tutorial/)
walks through the same steps with a check after each one.

1. Import the module on each host:

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

2. Create a deploy user and let it run the activation helper:

   ```nix
   {
     users.users.deploy = {
       isNormalUser = true;
       openssh.authorizedKeys.keys = [ "ssh-ed25519 AAAA... you@laptop" ];
     };
     snowglobe.deploy.allowedUsers = [ "deploy" ];
     # nix copy as a non-root user needs a trusted user (or signed paths)
     nix.settings.trusted-users = [ "deploy" ];
   }
   ```

   Tell `ssh` to log in as that user, in `~/.ssh/config`:

   ```text
   Host web1
     User deploy
   ```

   `snow` connects to each host's `networking.hostName` (plus
   `networking.domain`, if set), not to the flake attribute name, so that
   is the name the `Host` line must match. If that name does not resolve,
   add a `HostName` line; see
   [Connect to your hosts](https://l9o.dev/snowglobe/connect/).

3. Apply this configuration once with `nixos-rebuild switch`, as an account
   that can already use `sudo`. Then, from the flake's directory:

   ```bash
   snow list                 # the hosts snow sees
   snow deploy --dry-run web1  # build, copy and show what would change
   snow deploy web1            # deploy one host
   snow deploy                 # deploy every host
   ```

4. Optionally, give hosts a phase, tags or a reboot policy:

   ```nix
   {
     snowglobe.deploy = {
       phase = "p2";            # deploys after the p1 hosts
       tags = [ "web" ];        # snow deploy --tag web
       rebootPolicy = "auto";   # or "never" or "always"
     };
   }
   ```

Macs use `darwinModules.default`; see [Deploy nix-darwin hosts](https://l9o.dev/snowglobe/darwin/).
`snow rebuild` works like `nixos-rebuild` or `darwin-rebuild` on the local
machine, with the same reboot check. Everything else is in `snow --help` and
the module's option descriptions.

## How it works

Phases are for rolling out by risk. Low-risk hosts go first, and a
post-deploy hook can check each one. A failure stops the deployment before any
host in a later phase is touched. Spreading a cluster's nodes across phases
migrates it gradually instead of all at once.

Hosts reboot only when they need to. Switching to a new kernel, or
restarting services like udev or sysctl, leaves a host half old and half new,
so `snow` checks for that before activating. If a reboot is needed, it sets
the new system for the next boot, reboots and waits for the host. Per-host
policies cover the rest: `never` for hosts rebooted by hand, `auto` for hosts
that should reboot only when they must. A policy wins over `--switch`.

![reboot](docs/demos/reboot.svg)

Planning runs before any hook, so a host that can't deploy is never drained.
Hosts that already run the built system are skipped.

## Comparison with other tools

Compared with the three tools most users are choosing between. The [full
comparison](https://l9o.dev/snowglobe/comparison/) also covers morph, Clan,
devenv Machines and pull-based tools.

| | `nixos-rebuild --target-host` | deploy-rs | colmena | snowglobe |
|---|---|---|---|---|
| Many hosts per run | — | ✓ | ✓ | ✓ |
| Ordered phases that stop on failure | — | — | — | ✓ |
| Serial groups | — | — | — | ✓ |
| Reboot detection | — | — | `apply --reboot` (unconditional) | ✓ |
| Pre/post-deploy hooks | — | — | — | ✓ |
| nix-darwin targets | — | ✓ | — | ✓ |
| Automatic rollback | — | ✓ | — | — |
| Build on target | `--build-host` | `remoteBuild` | `buildOnTarget` | — |
| Secrets upload | — | — | `deployment.keys` | — |
| Sudo password prompt | `--ask-sudo-password` | `interactiveSudo` | — | not for remote hosts |
| Works without flakes | ✓ | partial | `hive.nix` | — |
| Extra flake output needed | no | `deploy.nodes` | `colmenaHive` | no |

deploy-rs also deploys non-system profiles. [Corrections
welcome](https://github.com/luizribeiro/snowglobe/issues).

## Security

- Push-only over SSH, with no agent or daemon on the hosts.
- One helper, `snowglobe-activate`, gets passwordless sudo, not
  `NOPASSWD: ALL`.
- The deploy user is still root-equivalent on the host. Use a dedicated
  deploy user and guard its SSH key like a root key.
- Local hooks run code from the flake on the deploying machine.

Details in the [security model](https://l9o.dev/snowglobe/security/).

## Documentation

The documentation is at [l9o.dev/snowglobe](https://l9o.dev/snowglobe/),
starting with the [tutorial](https://l9o.dev/snowglobe/tutorial/). Its
source is in [docs/](docs/).

- [docs/demos.md](docs/demos.md): every clip is a NixOS VM test deploying to real VMs
- [CHANGELOG.md](CHANGELOG.md)
- [Contributing](https://l9o.dev/snowglobe/contributing/)

## Roadmap

Not built yet:

- Signed deployments: hosts verify a signature before activating, so a
  deploy user is no longer root-equivalent. The goal is to not require a
  hardware token.
- `snow init`, an onboarding flow for new flakes and hosts.
- Pull mode, where each host fetches and applies its own configuration.

## Contributing

Issues and pull requests are welcome; see the
[contributing guide](https://l9o.dev/snowglobe/contributing/).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in snowglobe by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
