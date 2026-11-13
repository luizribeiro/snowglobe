---
title: "Security model"
group: "Concepts"
weight: 100
description: "What snow needs on your hosts, what the activation helper does and doesn't protect, and how to limit the risk of a deploy key."
---

This page explains what access `snow` needs on your hosts and on the machine that runs it, what the `snowglobe-activate` helper protects against and what it does not, and how to limit the damage if a deploy key leaks. The short version: a user that can deploy to a host is root on that host, so treat its SSH key like a root key.

## What runs where

snowglobe is push-only. There is no agent, daemon or open port on the hosts beyond SSH, and nothing on a host contacts the machine running `snow`. Every deployment is a sequence of commands that `snow` runs over SSH as the deploy user:

- `nix copy` sends the new system's closure into the host's Nix store;
- read-only commands read which system the host runs and boots, and its boot ID;
- `sudo snowglobe-activate` runs `dry-activate` for reboot detection, activates the new system and reboots the host;
- remote hooks run as the deploy user, without `sudo`.

On the machine running `snow`, the flake is evaluated and built, and local hooks run as you.

## The activation helper

Activating a NixOS system means running its `switch-to-configuration` script as root:

```bash
sudo /nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-nixos-system-web1-25.05/bin/switch-to-configuration switch
```

The store path changes with every build, so it cannot be named in a sudoers rule. The usual workaround is `NOPASSWD: ALL` for the deploy user, which lets it run anything as root.

The snowglobe module instead installs a small script, `snowglobe-activate`, at a fixed path: `/run/current-system/sw/bin/snowglobe-activate`. It takes a store path and a mode, sets the system profile for `switch` and `boot`, and runs the activation script. On NixOS it can also reboot the host. Setting `snowglobe.deploy.allowedUsers` adds one sudoers rule for that one path:

```nix
{
  snowglobe.deploy.allowedUsers = [ "deploy" ];
}
```

On NixOS this becomes, through `security.sudo.extraRules`:

```text
deploy ALL=(ALL:ALL) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate
```

On nix-darwin, through `security.sudo.extraConfig`:

```text
deploy ALL=(root) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate
```

The helper is always installed; the rule is only added when `allowedUsers` is not empty. The rule is written for `sudo`, so a host that uses sudo-rs instead gets no rule. The rule points at the running system's helper, which activates the new system; after that, `/run/current-system` points at the new system and its helper, so a deployment can update the helper itself.

If a NixOS host does not have the helper yet, `snow` warns and runs `nix-env` and `switch-to-configuration` directly with `sudo`, which then has to work without a password. nix-darwin hosts have no such fallback and need the module. [Set up a deploy user](/snowglobe/deploy-user/) walks through the setup.

### What the helper checks, and why that isn't a boundary

The helper accepts only a single top-level store path of the form `/nix/store/<hash>-<name>`, containing a system's activation script, and only the modes `switch`, `boot`, `test` and `dry-activate` (only `switch` on nix-darwin). Those checks catch typos and stop a path such as `/nix/store/../home/alice/evil` from running as root.

They do not make the deploy user less than root. Any user who can talk to the Nix daemon can add a store path, and a system built by the deploy user, with an activation script that does whatever that user wants, is a perfectly valid top-level store path. The helper would activate it, as root. The deploy user is usually also a trusted user of the Nix daemon (`nix.settings.trusted-users`), since `nix copy` from a non-root user otherwise only accepts signed paths, and a trusted user can already do a great deal to the store.

What the helper gives you is a sudoers configuration you can read: one command at a stable path, instead of `NOPASSWD: ALL`. It does not give you a deploy user that is safe to hand out.

### Allowed users are root-equivalent

Every SSH-based deployment tool needs a user that can become root on the host, because activation runs as root. deploy-rs, morph, Clan and `nixos-rebuild --target-host` can ask for that user's sudo password during a deployment; `snow deploy` cannot, so its deploy user becomes root without one. Anyone who can log in as a user in `allowedUsers` can take over the host.

## SSH

Every `ssh` call `snow deploy` makes, including `nix copy` and remote hooks, uses `BatchMode=yes`, so `ssh` never prompts for a password, a key passphrase or an unknown host key. You need key authentication through an agent or an unencrypted key file, and each host's key already in `known_hosts` (unless your SSH configuration sets `StrictHostKeyChecking=accept-new`). A changed host key fails the deployment instead of being accepted.

`snow` passes `ForwardAgent=no` on every connection, even if your SSH configuration enables forwarding for that host. A compromised host could otherwise use your forwarded agent to log in to every other host as you. The one way around it is `NIX_SSHOPTS`: for `nix copy`, options you set there come first, and `ssh` keeps the first value it sees.

Connections otherwise follow your SSH configuration, so `Host` entries, `User`, `IdentityFile` and `ProxyJump` apply as usual. `--user` overrides the user, and `snowglobe.deploy.sshPort` sets the port per host. See [Connect to your hosts](/snowglobe/connect/).

`snow` also checks that your SSH agent answers before it touches any host, because a hung agent would make every `ssh` call hang.

## Local hooks run flake code on your machine

A `local` hook is a command from the host's configuration that runs on the machine running `snow`, as you, with your environment: your `KUBECONFIG`, your cloud credentials, your SSH agent. Anyone who can change the flake, or one of its inputs, can therefore run code on that machine on the next deployment. Building the flake already runs code from it in the Nix sandbox; local hooks run it outside the sandbox.

Review changes to `snowglobe.hooks` like changes to a script you are about to run, and pin and review flake inputs that contribute hooks. Before any host is touched, `snow` builds every host's hooks and checks that the packages local hooks use exist on your machine, but it does not judge what a hook does.

## Secrets in hooks

Hook commands and their `environment` values are part of the host's configuration, so they end up in the Nix store, which is world-readable. Keep secrets out of both.

- Put secrets in `environmentFiles`. Those files are read at deploy time on the machine running `snow`, and only their paths are in the store. Write the paths as strings: a path literal like `./webhook.env` is copied into the store.
- Secrets decrypted on the host by agenix or sops-nix only work in `environmentFiles` if the same file also exists on the machine running `snow`.
- For a remote hook, values from environment files are passed as arguments to `ssh`, so other users on either machine can see them in `ps` while the hook runs.

[Write deployment hooks](/snowglobe/hooks/) has examples.

## Advice

1. **Use a dedicated deploy user, and only that one, in `allowedUsers`.** A personal account in that list is root without a password.
2. **Treat the deploy user's SSH key like a root key.** Allow key-only login, and keep the key only on the machines or CI runners that deploy.
3. **Keep the deploying machine trustworthy.** It holds the key, builds every system and runs local hooks. A CI runner that deploys should only run trusted branches.
4. **Review changes to hooks and flake inputs** before deploying them.
5. **Watch activations.** The helper runs through `sudo`, so activations appear in the host's system log.
6. **Move off `NOPASSWD: ALL`** if you had it: add the module, deploy once, check that the helper works, then remove the old rule.

## Not built yet: signed deployments

The roadmap includes signed deployments: a host would only activate systems signed with a deploy key, so a user who can deploy but does not hold that key could no longer run arbitrary code as root. The aim is to do this without requiring a hardware token. This is not implemented; today the model above applies. snowglobe is pre-1.0, and this part of the design may change.
