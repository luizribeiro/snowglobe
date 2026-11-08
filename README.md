# snowglobe

[![CI](https://github.com/luizribeiro/snowglobe/actions/workflows/ci.yml/badge.svg)](https://github.com/luizribeiro/snowglobe/actions/workflows/ci.yml)

snowglobe deploys NixOS and nix-darwin hosts from a flake, over SSH. The CLI
is `snow`. `snow push` builds every host, copies it over, activates it, and
reboots the ones that need a reboot. Hosts go in phases, and you watch it
happen on a board with one row per host.

[![switch](docs/demos/switch.svg)](https://asciinema.org/a/6J8p7HDK0Df1tPSx)

I use it for my homelab: x86 and aarch64 NixOS hosts (some are Raspberry
Pis) and a few Macs on nix-darwin. The machine running `snow` can be a Mac or
Linux. CI pushes it phase by phase with `snow push --phase pN`.

snowglobe is young. We are its only users so far, and the CLI and module
options will change before 1.0.

## Install

```bash
nix run github:luizribeiro/snowglobe -- list     # try it
nix profile install github:luizribeiro/snowglobe # or install it
```

There are also static binaries for Linux and macOS on the
[releases page](https://github.com/luizribeiro/snowglobe/releases).

## Setup

Import the module on each host:

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

And give your deploy user the activation helper:

```nix
{
  snowglobe.push.allowedUsers = [ "deploy" ];
  # nix copy as a non-root user needs a trusted user (or signed paths)
  nix.settings.trusted-users = [ "deploy" ];
}
```

Macs work the same way with `darwinModules.default`, see
[docs/darwin.md](docs/darwin.md).

`nixos-rebuild` that once by hand, then from the flake's directory:

```bash
snow list                 # the hosts snow sees
snow push --dry-run web1  # build, copy, show what would change
snow push web1
snow push                 # every host
```

snow uses your SSH config to reach `networking.hostName`. It never asks for
a password, so you need key auth and the host key in `known_hosts`.

Then give hosts a phase, tags or a reboot policy:

```nix
{
  snowglobe.push = {
    phase = "p2";            # after the p1 hosts
    tags = [ "web" ];        # snow push --tag web
    rebootPolicy = "auto";   # or "never" or "always"
  };
}
```

On the machine you're sitting at, `snow rebuild` is `nixos-rebuild switch`
(or `darwin-rebuild`) with the same reboot check: if the new system needs a
reboot, it asks what to do.

`snow --help` and the module's option descriptions have the rest.

## How it works

- **Phases.** Phases go one after another, and a failure stops the push
  there. Whatever you put early goes first, and the hosts in later phases
  aren't touched if it breaks.
- **Serial groups.** Hosts in a group take turns, so your storage or
  Kubernetes nodes drain one at a time.
- **Reboots.** Switching a host to a new kernel, or restarting udev or
  sysctl, leaves it half old, half new. snow checks for that, and if a reboot
  is needed, it sets the new system for the next boot, reboots and waits for
  the host. Per-host policies win over `--switch`, so you can't switch a
  `never` host by accident.
- **Nothing is drained for nothing.** Planning runs before any hook, so a
  host that can't deploy is never drained. Unchanged hosts are skipped.
- **Hooks.** Drain before, uncordon after, alert on failure, on your machine
  or on the host. [docs/hooks.md](docs/hooks.md)

Details in [docs/push.md](docs/push.md).

## Demos

Every demo is a NixOS VM test pushing to real VMs. The test checks the
result, and the same run records the clip.

A host that needs a reboot gets the new system for next boot, then reboots:

[![reboot](docs/demos/reboot.svg)](https://asciinema.org/a/CRvXGdOvXPBkkiDV)

Two hosts in a serial group take turns:

[![serial-group](docs/demos/serial-group.svg)](https://asciinema.org/a/DfWYNZ7K7KCfiFvg)

One host fails to activate, the others keep going:

[![activation-failure](docs/demos/activation-failure.svg)](https://asciinema.org/a/NRfbtSt4AyH53Xsy)

`snow rebuild` reboots into the new system:

[![rebuild-reboot](docs/demos/rebuild-reboot.svg)](https://asciinema.org/a/HrREagz2DZPhuXDC)

All of them on [the demos page](docs/demos.md).

## Compared to

- `nixos-rebuild --target-host`: one host per run. snowglobe does many, with
  phases, hooks, reboots, and Macs in the same push.
- deploy-rs: many hosts, nix-darwin too, magic rollback (it rolls back if it
  can't reach the host after activating), building on the target, profiles
  other than the system, no flake required.
- colmena: parallel, tags, building on the target, secrets uploaded outside
  the store, no flake required. `apply --reboot` always reboots.

What snowglobe has that they don't: phases that stop on failure, serial
groups, hooks, and knowing when a host needs a reboot.

What it doesn't have: rollback, secrets, building on the target, non-flake
setups, and a sudo password prompt for remote hosts. It's also a lot newer than all of them.

## Security

- Push only, over SSH. No agent or daemon on the hosts.
- One helper on each host, `snowglobe-activate`. Deploy users get
  passwordless sudo for that, and nothing else.
- Every push tool needs a user that can become root on the host. deploy-rs
  and `nixos-rebuild --target-host` can ask for that user's sudo password.
  `snow push` can't yet, so its deploy user is root without one. Use a dedicated
  deploy user and treat its SSH key like a root key.
- Local hooks run on your machine, as you. Pushing someone else's flake runs
  their code.

Details: [docs/activation-helper.md](docs/activation-helper.md#security).

## Roadmap

Not built yet:

- Signed deployments. The host checks a signature before it activates, so a
  deploy user is no longer root. Ideally without having to buy hardware like
  a YubiKey, and easy to set up.
- `snow init`, an onboarding flow that sets up your flake and hosts.
- Pull mode: each host fetches and applies its own config.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in snowglobe by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
