---
title: "Coming from deploy-rs or colmena"
group: "Guides"
weight: 70
description: "How deploy-rs and colmena settings map to snowglobe, and how to move hosts over one at a time."
---

This guide maps the settings and commands of [deploy-rs](https://github.com/serokell/deploy-rs) and [colmena](https://github.com/zhaofengli/colmena) to their snowglobe equivalents, and walks through moving hosts to snowglobe one at a time while the other tool keeps working for the rest.

All three tools build a NixOS system on your machine, copy it over SSH and activate it, so most concepts carry over. The main structural difference is where host settings live. deploy-rs and colmena keep deployment settings in a separate flake output (`deploy.nodes`, or the colmena hive). snowglobe reads your existing `nixosConfigurations` and `darwinConfigurations` and takes its per-host settings from NixOS options under `snowglobe.*`, so they are evaluated, type-checked and documented like the rest of the host's configuration. For a broader comparison, including features each tool has that the others don't, see [Comparison with other tools](/snowglobe/comparison/).

## Before you start

A few differences are worth knowing before you migrate, since they may decide whether snowglobe fits:

- **Flakes are required.** snowglobe only reads flake outputs.
- **There is no automatic rollback.** deploy-rs's magic rollback has no equivalent. If a deployment breaks SSH access to a host, snowglobe can't undo it. [Roll back a bad deployment](/snowglobe/rollback/) covers what you can do instead, including `--test` for risky changes.
- **Builds always happen on the deploying machine.** There is no remote build or build-on-target option. See [Build for other architectures](/snowglobe/architectures/) for building hosts of another architecture.
- **No secrets upload.** Use a secrets manager that works through the NixOS configuration, such as [sops-nix](https://github.com/Mic92/sops-nix) or [agenix](https://github.com/ryantm/agenix).
- **Only the system profile is deployed.** deploy-rs's per-user profiles (Home Manager and others) aren't supported.
- **No remote sudo password.** The deploy user runs the activation helper through a passwordless sudo rule that permits nothing else. deploy-rs's `interactiveSudo` has no equivalent.
- **snowglobe is pre-1.0.** The CLI and module options may change between minor releases; check the [Changelog](/snowglobe/changelog/) when you update.

## Setting mappings

### deploy-rs

| deploy-rs | snowglobe |
|---|---|
| `deploy.nodes.<name>` | the host's entry in `nixosConfigurations` or `darwinConfigurations`; no separate output |
| `hostname` | `networking.hostName`, plus `networking.domain` when set. To connect to a different address, add a `Host` entry with `HostName` in `~/.ssh/config` (see [Connect to your hosts](/snowglobe/connect/)) |
| `sshUser` | `User` in `~/.ssh/config`, or `snow deploy --user` for every host in a deployment. Without either, ssh uses your local user name |
| `sshOpts` | `~/.ssh/config`. For a non-default port there is also `snowglobe.deploy.sshPort` |
| `user` (the profile user) | always root, through the activation helper |
| `profiles.system` | the only profile; snowglobe always deploys the host's system |
| other `profiles.*` | no equivalent |
| `magicRollback`, `autoRollback`, `confirmTimeout` | no equivalent; see [Roll back a bad deployment](/snowglobe/rollback/) |
| `remoteBuild` | no equivalent; see [Build for other architectures](/snowglobe/architectures/) |
| `fastConnection` | snowglobe always copies the closure from the deploying machine, as with `fastConnection = true` |
| `interactiveSudo` | no equivalent; add the user to `snowglobe.deploy.allowedUsers` |
| `activate.darwin` | `snowglobe.darwinModules.default`; see [Deploy nix-darwin hosts](/snowglobe/darwin/) |
| `deploy .#web1` | `snow deploy web1` |
| `deploy .` | `snow deploy` |
| `deploy --dry-activate` | `snow deploy --dry-run` |
| `deploy-rs.lib.<system>.deployChecks` | No direct equivalent. snowglobe settings are ordinary NixOS options, so `nix flake check` already type-checks them, and `snow build` builds every host without deploying |

### colmena

| colmena | snowglobe |
|---|---|
| `colmenaHive` / `colmena` output | `nixosConfigurations`; no separate output |
| `deployment.targetHost` | `networking.hostName`, plus `networking.domain` when set; or a `Host` entry with `HostName` in `~/.ssh/config` |
| `deployment.targetUser` | `User` in `~/.ssh/config`, or `snow deploy --user`. colmena defaults to `root`; snowglobe leaves the choice to ssh, which uses your local user name unless configured |
| `deployment.targetPort` | `snowglobe.deploy.sshPort` |
| `deployment.tags` | `snowglobe.deploy.tags` |
| `deployment.keys` | no equivalent; use sops-nix or agenix |
| `deployment.buildOnTarget` | no equivalent; see [Build for other architectures](/snowglobe/architectures/) |
| `deployment.allowLocalDeployment` with `colmena apply-local` | `snow rebuild` on the machine itself; see [Rebuild the local machine](/snowglobe/rebuild/) |
| `deployment.privilegeEscalationCommand` | the activation helper and `snowglobe.deploy.allowedUsers` |
| `colmena apply` | `snow deploy` |
| `colmena apply --on @web` | `snow deploy --tag web` (or `--tags web,db` for several) |
| `colmena apply --on web1,web2` | `snow deploy web1 web2` |
| `colmena apply boot` / `test` / `dry-activate` | `snow deploy --boot` / `--test` / `--dry-run` |
| `colmena apply --reboot` | snowglobe reboots by default when the new system needs it; set `snowglobe.deploy.rebootPolicy = "always"` to reboot after every deployment |
| `colmena build` | `snow build` |
| `colmena apply --parallel` (default 10) | `snow deploy --concurrency` (default 3) |

snowglobe also has settings that neither tool has: phases that roll out groups of hosts in order and stop at the first failure ([Roll out in phases](/snowglobe/phases/)), serial groups for clusters ([Deploy a cluster one node at a time](/snowglobe/cluster/)), deployment hooks ([Write deployment hooks](/snowglobe/hooks/)), and reboot detection with per-host reboot policies ([Control reboots and activation modes](/snowglobe/reboots/)). The [Module options](/snowglobe/options/) page lists every option.

## Migrate one host

You can move hosts one at a time. snowglobe only reads `nixosConfigurations` and `darwinConfigurations`, and ignores `deploy.nodes` and colmena's outputs, so both tools can deploy from the same flake during the migration. Both also write the same system profile, `/nix/var/nix/profiles/system`, so generations created by either tool appear in the same list and either can roll back to them.

### 1. Make sure the host is in `nixosConfigurations`

With deploy-rs, it already is: `deploy.nodes.<name>.profiles.system.path` is normally built from `self.nixosConfigurations.<name>`.

colmena flakes often define hosts only inside the hive. In that case, define each host's modules once and use them for both outputs. colmena's `deployment.*` options only exist inside a colmena evaluation, so keep them out of the shared list:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    colmena.url = "github:zhaofengli/colmena";
    snowglobe.url = "github:luizribeiro/snowglobe";
  };

  outputs = { self, nixpkgs, colmena, snowglobe, ... }:
    let
      hosts = [ "web1" "web2" "db1" ];
      modulesFor = name: [
        snowglobe.nixosModules.default
        { networking.hostName = name; }
        ./hosts/${name}.nix
      ];
    in
    {
      nixosConfigurations = nixpkgs.lib.genAttrs hosts (name:
        nixpkgs.lib.nixosSystem { modules = modulesFor name; });

      colmenaHive = colmena.lib.makeHive ({
        meta.nixpkgs = import nixpkgs { system = "x86_64-linux"; };
      } // nixpkgs.lib.genAttrs hosts (name: {
        imports = modulesFor name;
        deployment.targetHost = "${name}.example.com";
      }));
    };
}
```

Each `./hosts/<name>.nix` sets `nixpkgs.hostPlatform`, so `nixosSystem` needs no `system` argument. Adapt the hive part to how your flake already passes `nodeNixpkgs` or `specialArgs`.

The shared list sets `networking.hostName` explicitly. colmena gives each node its name as a default host name, but a plain `nixosSystem` doesn't, and NixOS then falls back to `nixos`. snowglobe connects to `networking.hostName`, so without this line it would try to reach a host called `nixos`, and the two outputs would build different systems.

### 2. Import the snowglobe module

Add `snowglobe.nixosModules.default` to the host's modules, as above, if it isn't there yet. For nix-darwin hosts, use `snowglobe.darwinModules.default`; see [Deploy nix-darwin hosts](/snowglobe/darwin/).

### 3. Make the host's address resolvable

snowglobe connects to `networking.hostName`, followed by `.` and `networking.domain` when the domain is set. If that name doesn't resolve from your machine, or you used a different address in `hostname` or `targetHost`, add it to `~/.ssh/config`:

```text
Host web1.example.com
  HostName 192.0.2.10
  User deploy
```

This is also where `sshUser` / `targetUser` and other `sshOpts` go. The host key must already be in your `known_hosts`, since snow runs ssh in batch mode and never asks to confirm one.

### 4. Let the deploy user activate

```nix
{
  snowglobe.deploy.allowedUsers = [ "deploy" ];
  # nix copy as a non-root user only imports unsigned paths from a trusted user
  nix.settings.trusted-users = [ "deploy" ];
}
```

This is the user snow connects as. If the existing tool logged in as `root`, consider creating a dedicated user instead; [Set up a deploy user](/snowglobe/deploy-user/) explains why and how. If you do keep connecting as root, which that page argues against, `allowedUsers` and `trusted-users` aren't needed.

### 5. Deploy once with the existing tool

The sudoers rule and the activation helper only exist on the host once this configuration is active, so deploy it one last time with the tool you already use:

```bash
deploy .#web1          # deploy-rs
colmena apply --on web1  # colmena
```

### 6. Check and deploy with snowglobe

From the flake's directory:

```bash
snow list               # web1 should appear, with its phase and tags
snow build web1         # builds without deploying
snow deploy --dry-run web1
snow deploy web1
```

Because step 5 already activated this configuration, snow will most likely find the host running exactly the system it just built. Both the dry run and the deployment then stop there and report `✓ web1 (unchanged)`, which confirms that both tools build the same closure. If the dry run instead reports `✓ web1 (dry-run)`, the closures differ; the units that would change are listed in the log (`/tmp/snow-<timestamp>.log`, whose path snow prints when it starts). The next change you deploy reports `✓ web1 (switch)`. To activate an unchanged system anyway, pass `--redeploy`.

### 7. Move per-host settings over

Translate the host's tags, ordering and reboot habits into `snowglobe.deploy` options using the tables above. Then repeat these steps for the next host. Once every host is migrated, remove `deploy.nodes` or the hive, and the old tool's input, from the flake.

## Commands you used before

Most day-to-day commands have a direct counterpart:

```bash
snow deploy                    # every host, phase by phase
snow deploy web1 web2          # by name
snow deploy --tag web          # by tag
snow deploy -e 'config: config.services.nginx.enable or false'  # by a Nix expression
snow build                   # build every host, deploy nothing
snow rebuild                 # this machine, like nixos-rebuild switch
```

`snow deploy` with no host names deploys every host except those in the `never` phase. Run `snow deploy --help` for every flag, and see the [Commands reference](/snowglobe/commands/).
