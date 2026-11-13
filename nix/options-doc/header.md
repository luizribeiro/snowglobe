---
title: "Module options"
group: "Reference"
weight: 115
description: "Every snowglobe.* option of the NixOS and nix-darwin modules, with types, defaults and examples."
---

This page lists every option the snowglobe module adds under `snowglobe.*`. It is generated from the module source, [`modules/*.nix`](https://github.com/luizribeiro/snowglobe/tree/main/modules) on GitHub, so the descriptions are the ones the module itself declares.

You set these options in each host's configuration, next to the rest of its NixOS or nix-darwin settings. They exist only once the host imports the snowglobe module, `nixosModules.default` on NixOS and `darwinModules.default` on nix-darwin (see [Installation](/snowglobe/installation/)):

```nix
nixosConfigurations.web1 = nixpkgs.lib.nixosSystem {
  system = "x86_64-linux";
  modules = [
    snowglobe.nixosModules.default
    {
      snowglobe.deploy.phase = "p1";
      snowglobe.deploy.tags = [ "web" ];
    }
    ./hosts/web1.nix
  ];
};
```

Every option is optional: a host that sets none of them is still deployed by `snow deploy`, with no phase, no tags and the `auto` reboot policy. To check what `snow` reads for each host, run `snow list --json`.

Both modules declare the same options. Where the nix-darwin module behaves differently, the option says so. In particular:

- `snowglobe.deploy.allowedUsers` grants its sudo rule through `security.sudo.extraRules` on NixOS and through `security.sudo.extraConfig` on nix-darwin. The rule is the same on both: the listed users may run `/run/current-system/sw/bin/snowglobe-activate` as root without a password. On nix-darwin the helper accepts only the `switch` mode and cannot reboot the host.
- `snowglobe.deploy.rebootPolicy`, `snowglobe.deploy.ignoredFailedUnits` and `post-reboot` hooks have no effect on nix-darwin hosts, because `snow` never reboots them and nix-darwin has no systemd units.

For how these options fit together, see [How a deployment works](/snowglobe/how-deploy-works/), [Roll out in phases](/snowglobe/phases/), [Write deployment hooks](/snowglobe/hooks/) and [Control reboots and activation modes](/snowglobe/reboots/).
