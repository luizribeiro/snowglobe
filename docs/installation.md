---
title: "Installation"
group: "Get started"
weight: 10
description: "Install the snow CLI and add the snowglobe modules to your flake."
---

This page shows how to get the `snow` command onto the machine you deploy
from, and how to add snowglobe to your flake so your hosts can use its
NixOS and nix-darwin modules.

snowglobe has two parts, and you need both:

- **The `snow` CLI**, which runs on the deploying machine (your laptop, a
  workstation or a CI runner).
- **The snowglobe module**, which you import into each host's configuration.
  It installs the activation helper, `snowglobe-activate`, and defines the
  `snowglobe.*` options that `snow` reads.

## Requirements

On the deploying machine:

- Linux or macOS.
- Nix with the `nix-command` and `flakes` experimental features enabled.
  `snow` runs the `nix` command for every evaluation, build and copy, and
  does not turn these features on by itself. If they are not enabled yet,
  add this line to `~/.config/nix/nix.conf` (or `/etc/nix/nix.conf`):

  ```ini
  experimental-features = nix-command flakes
  ```

- An OpenSSH client (`ssh`) on your `PATH`.
- The ability to build every host's system. `snow` always builds locally, so
  a host of another architecture needs a remote builder or emulation; see
  [Build for other architectures](/snowglobe/architectures/).

On each host:

- NixOS, or macOS with nix-darwin.
- A running SSH server you can reach from the deploying machine.
- A user to deploy as, which you log in to with an SSH key. `snow` runs
  `ssh` in batch mode and never prompts for a password or a host key; see
  [Set up a deploy user](/snowglobe/deploy-user/) and
  [Connect to your hosts](/snowglobe/connect/).

Your hosts must be defined in a flake, as `nixosConfigurations` or
`darwinConfigurations`. snowglobe does not support configurations outside a
flake.

## Try it without installing

`nix run` builds `snow` from the repository and runs it once. The first run
compiles `snow` from source, which takes a few minutes; later runs reuse the
build.

```bash
nix run github:luizribeiro/snowglobe -- --version
```

You should see the version, for example:

```text
snow 0.1.0
```

Anything after `--` goes to `snow`, so `nix run github:luizribeiro/snowglobe -- list`
lists the hosts of the flake in the current directory.

## Install with nix profile

To keep `snow` on your `PATH`, add it to your Nix profile:

```bash
nix profile add github:luizribeiro/snowglobe
```

Older Nix releases call this command `nix profile install`. To move
to the latest commit later, find the entry's name with `nix profile list`
and run `nix profile upgrade <name>`.

## Add snowglobe to your flake

Your hosts need the snowglobe module, so add snowglobe as an input of the
flake that defines them. A minimal flake with one NixOS host and one Mac
looks like this:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-darwin.url = "github:nix-darwin/nix-darwin";
    nix-darwin.inputs.nixpkgs.follows = "nixpkgs";
    snowglobe.url = "github:luizribeiro/snowglobe";
  };

  outputs = { nixpkgs, nix-darwin, snowglobe, ... }: {
    nixosConfigurations.web1 = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        snowglobe.nixosModules.default
        ./hosts/web1/configuration.nix
      ];
    };

    darwinConfigurations.your-mac = nix-darwin.lib.darwinSystem {
      system = "aarch64-darwin";
      modules = [
        snowglobe.darwinModules.default
        ./hosts/your-mac/configuration.nix
      ];
    };
  };
}
```

The flake exposes:

| Output | What it is |
|---|---|
| `nixosModules.default` | The module for NixOS hosts |
| `darwinModules.default` | The module for nix-darwin hosts |
| `packages.<system>.snow` | The `snow` CLI (also `packages.<system>.default`) |

Importing the module changes nothing on a host by itself, apart from
installing `snowglobe-activate`. The deploy user gets permission to run it
only once you list that user in `snowglobe.deploy.allowedUsers`; the
[tutorial](/snowglobe/tutorial/) walks through that step.

### Use the CLI from the same input

Because snowglobe is pre-1.0, the CLI and the module can change together:
a newer `snow` may read an option an older module does not define, or the
other way round. The simplest way to keep them in step is to run `snow` from
the same flake input your hosts import, for example from a development
shell:

```nix
{
  outputs = { nixpkgs, snowglobe, ... }: {
    # ... nixosConfigurations as above ...

    devShells.x86_64-linux.default =
      nixpkgs.legacyPackages.x86_64-linux.mkShell {
        packages = [ snowglobe.packages.x86_64-linux.snow ];
      };
  };
}
```

Then `nix develop` puts the pinned `snow` on your `PATH`. Use the system of
your deploying machine in place of `x86_64-linux`, for example
`aarch64-darwin` on an Apple silicon Mac.

### Pin and update

`flake.lock` pins snowglobe to the commit it resolved when you added the
input, so every machine that uses the flake gets the same version. To move
to the latest commit on `main`:

```bash
nix flake update snowglobe
```

To follow a release rather than `main`, put a release tag in the URL. Tags
are listed on the [releases
page](https://github.com/luizribeiro/snowglobe/releases); replace `vX.Y.Z`
with one of them:

```nix
{
  inputs.snowglobe.url = "github:luizribeiro/snowglobe/vX.Y.Z";
}
```

snowglobe has its own `nixpkgs` input, which it uses only to build the
`snow` package; the modules use the `pkgs` of the host that imports them. If
you would rather not download a second nixpkgs, make it follow yours:

```nix
{
  inputs.snowglobe.inputs.nixpkgs.follows = "nixpkgs";
}
```

`snow` is then built with your nixpkgs' Rust toolchain. If that build fails
because your nixpkgs is too old, remove the `follows` line.

Before upgrading, read the [changelog](/snowglobe/changelog/): minor
releases before 1.0 may rename commands, flags or options.

## Check the installation

Run:

```bash
snow --version
snow --help
```

`snow --version` prints `snow` followed by the version number. `snow --help`
lists the commands: `list`, `build`, `deploy` and `rebuild`. The
[Commands reference](/snowglobe/commands/) describes each of them.

To check that `snow` can read your flake, run `snow list` in the flake's
directory (or any directory below it). It prints a table of your hosts:

```text
HOST      PHASE  TAGS
web1      -      -
your-mac  -      -
```

If your flake is elsewhere, pass its path with `-f`, or set `FLAKE_PATH`:

```bash
snow -f ~/src/infra list
```

## Next steps

- Follow the [tutorial](/snowglobe/tutorial/) to deploy your first host.
- Set up the account `snow` connects as: [Set up a deploy
  user](/snowglobe/deploy-user/).
