---
title: "Build for other architectures"
group: "Guides"
weight: 58
description: "Deploy hosts whose platform differs from the machine running snow, using a remote builder or emulation."
---

This guide shows how to deploy hosts whose CPU architecture or operating system differs from the machine running `snow`: aarch64 boards from an x86_64 laptop, Linux servers from a Mac, and so on. snow builds everything on the machine that runs it, so the work is in teaching that machine's Nix to build for the other platform.

## Where snow builds

For each host, `snow deploy` (and `snow build`) runs `nix build` on the host's system in your flake's directory, adding only flags that say where to put the result:

```bash
nix build .#nixosConfigurations.web1.config.system.build.toplevel
```

It then copies the result to the host with `nix copy --to ssh://...`. Nothing is built on the target host, and the host does not fetch anything from a binary cache by itself: every path it needs comes from the machine running snow. snow has no equivalent of colmena's `buildOnTarget` or deploy-rs's `remoteBuild`.

snow passes no build settings of its own. The `nix build` runs with your normal Nix configuration: `/etc/nix/nix.conf` (on NixOS, `nix.settings` and `nix.buildMachines`), `~/.config/nix/nix.conf`, and the `NIX_CONFIG` environment variable, which snow passes through unchanged. Whatever makes that `nix build` command work on your machine also makes `snow deploy` work, and you can test your setup with `snow build` before deploying anything.

If your machine cannot build for a host's platform, the build step fails with an error from Nix along the lines of:

```text
error: Cannot build '/nix/store/...-nixos-system-arm1.drv'.
       Reason: required system or feature not available
       Required system: 'aarch64-linux' with features {}
       Current system: 'x86_64-linux' with features {benchmark, big-parallel, kvm, nixos-test}
```

To see which platform each host is, list them as JSON; the `system` field comes from `nixpkgs.hostPlatform`:

```bash
snow list --json
```

## Options

There are three ways to build for another platform. They can be combined.

| Approach | Speed | Needs | Good for |
|---|---|---|---|
| Remote builder | native speed | a machine of the target platform, reachable over SSH | regular deploys, large closures, anything compiled locally |
| binfmt emulation | slow (QEMU user-mode emulation) | a NixOS deployer, nothing else | a few small hosts whose packages mostly come from the binary cache |
| Cross-compilation | native speed | changes to the host's configuration | special cases; most of the closure is not in the binary cache |

Most aarch64-linux and x86_64-linux packages are in the official binary cache, so even an emulated build mostly downloads. What gets built locally is your own configuration (the system's `/etc`, scripts, unit files) plus any package you patched or overrode. The fewer packages you override, the less the choice matters.

## Use a remote builder

A remote builder is another machine that Nix sends builds to over SSH. Any Linux machine of the target platform that runs Nix will do: a cloud ARM instance, a spare board, or one of the hosts you are deploying. The local Nix daemon decides which builds to send where, based on each builder's `system`.

On a NixOS machine running snow, declare the builder:

```nix
{
  nix.distributedBuilds = true;
  nix.buildMachines = [{
    hostName = "arm-builder.example.com";
    sshUser = "nixbuilder";
    sshKey = "/root/.ssh/nixbuilder";
    protocol = "ssh-ng";
    system = "aarch64-linux";
    maxJobs = 4;
    supportedFeatures = [ "big-parallel" ];
  }];
  # Let the builder fetch dependencies from the binary cache itself,
  # instead of receiving them from this machine
  nix.settings.builders-use-substitutes = true;
}
```

On the builder, allow that user to submit builds:

```nix
{
  users.users.nixbuilder = {
    isNormalUser = true;
    openssh.authorizedKeys.keys = [ "ssh-ed25519 AAAA... nixbuilder" ];
  };
  nix.settings.trusted-users = [ "nixbuilder" ];
}
```

The Nix daemon, which runs as root, makes the SSH connection, so the key and the builder's host key must be usable by root, not just by your user. Check the connection as root:

```bash
sudo ssh -i /root/.ssh/nixbuilder nixbuilder@arm-builder.example.com true
sudo nix store info --store 'ssh-ng://nixbuilder@arm-builder.example.com?ssh-key=/root/.ssh/nixbuilder'
```

On a machine that is not NixOS, put the same builder into the system-wide `/etc/nix/nix.conf` and restart the Nix daemon. A `builders` setting in your own `~/.config/nix/nix.conf` is ignored unless your user is in `trusted-users`.

```text
builders = ssh-ng://nixbuilder@arm-builder.example.com aarch64-linux /root/.ssh/nixbuilder 4 1 big-parallel
builders-use-substitutes = true
```

Then build one host to confirm that the builds go to the builder:

```bash
snow build arm1
```

Outputs built remotely are copied back to the machine running snow, and from there to the host during the deployment. On a slow uplink, keep the remote builder close to the machine running snow, or run snow from a machine on the same network as your hosts (see [Run from CI](/snowglobe/ci/)).

## Use binfmt emulation

On a NixOS machine running snow, you can let the kernel run binaries for other architectures through QEMU, and let Nix build for those platforms directly:

```nix
{
  boot.binfmt.emulatedSystems = [ "aarch64-linux" ];
}
```

After a `nixos-rebuild switch` on that machine, Nix accepts `aarch64-linux` builds and runs them locally under emulation; no other setting is needed. Emulated builds are many times slower than native ones, which is fine when nearly everything comes from the binary cache and painful when you compile a kernel or a large package. If a deployment spends most of its time compiling, switch to a remote builder.

## Deploy Linux hosts from a Mac

A Mac running snow cannot build Linux systems by itself. nix-darwin can run a small NixOS virtual machine as a local Linux builder:

```nix
# the Mac's nix-darwin configuration
{
  nix.linux-builder.enable = true;
}
```

This needs nix-darwin to manage the Nix installation on the Mac. The builder runs Linux on the Mac's own architecture, so an Apple silicon Mac gets an `aarch64-linux` builder. For hosts of another architecture, add a remote builder of that platform as described above, in the Mac's `nix.buildMachines` or `nix.conf`.

The closure is copied from the Mac to each host as usual. Hooks are not affected: local hooks run on the Mac with packages from `localPkgs`, which snow builds for the Mac, while remote hooks use the host's own `pkgs`. See [Packages in hook commands](/snowglobe/hooks/#packages-in-hook-commands).

## Cross-compile

A host's configuration can ask nixpkgs to cross-compile it, so that the machine running snow builds it natively for the host's platform:

```nix
{
  nixpkgs.buildPlatform = "x86_64-linux";
  nixpkgs.hostPlatform = "aarch64-linux";
}
```

The trade-off is that cross-compiled packages are a different build from the native ones, so the binary cache rarely has them and most of the system is compiled locally. Some packages also don't cross-compile. Treat this as a last resort for platforms where neither a builder nor emulation is practical. Note that `nixpkgs.buildPlatform` is part of the host's configuration, so it also changes how the host builds itself if you ever run `nixos-rebuild` on it.

## Related pages

- [How a deployment works](/snowglobe/how-deploy-works/): the build, copy and activate steps in order.
- [Deploy nix-darwin hosts](/snowglobe/darwin/): Macs as deployment targets.
- [Troubleshooting](/snowglobe/troubleshooting/): build failures and other errors.
