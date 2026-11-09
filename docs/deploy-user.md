---
title: "Set up a deploy user"
group: "Guides"
weight: 30
description: "Create a dedicated account that snow logs in as, and give it passwordless sudo for the activation helper only."
---

This guide sets up a dedicated `deploy` account on each host, lets it copy systems into the Nix store, and gives it passwordless sudo for exactly one command: the activation helper, `snowglobe-activate`. When you finish, `snow deploy` can deploy to the host without root SSH logins and without `NOPASSWD: ALL`.

## Before you start

You need:

- A host that already imports the snowglobe module (`snowglobe.nixosModules.default`, or `snowglobe.darwinModules.default` on a Mac). The [tutorial](/snowglobe/tutorial/) shows how to add it to a flake.
- A way to apply the host's configuration once by hand, such as a console or an existing SSH login with sudo. The sudo rule only exists after the first rebuild, so `snow deploy` cannot set it up for you.
- An SSH key pair on the machine you will run `snow` from. This guide assumes `~/.ssh/deploy_ed25519`; create one with `ssh-keygen -t ed25519 -f ~/.ssh/deploy_ed25519` if you need to.

## Why a dedicated user

Activating a system runs its activation script as root, so whatever account `snow` logs in as has to become root on the host. snow cannot answer a password prompt on the host: every connection runs in batch mode (see [Connect to your hosts](/snowglobe/connect/#no-prompts-batch-mode)), and remote commands run without a terminal, so sudo has nowhere to ask. That account therefore gets passwordless root, through one command.

A user who can run `snowglobe-activate` without a password is effectively root. Anyone who can talk to the Nix daemon can put a system with an activation script of their choosing into `/nix/store`, and the helper will activate it. The helper narrows *which command* is in sudoers, not *what that command can do*. The [Security model](/snowglobe/security/) page covers this in detail.

That is the reason for a separate account. If you add your personal login to the allowed users, every process running as you, and anyone who steals your session, has passwordless root on the host. A `deploy` user with a single purpose keeps that power in one place: one SSH key, used only by the machine or CI runner that deploys.

## Create the user on NixOS

Add the following to the host's configuration, or to a module that every host imports:

```nix
{
  users.users.deploy = {
    isNormalUser = true;
    openssh.authorizedKeys.keys = [
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA... deploy@your-laptop"
    ];
  };

  snowglobe.deploy.allowedUsers = [ "deploy" ];

  nix.settings.trusted-users = [ "deploy" ];
}
```

Each line has a job:

`users.users.deploy`
: A normal account with a login shell and your public key. It does not need to be in `wheel`, and should not be: membership in `wheel` usually comes with general sudo rights, which is what this setup avoids.

`snowglobe.deploy.allowedUsers`
: The users that may run the activation helper with sudo and no password. The module turns the list into a sudo rule, shown in the next section. With the default empty list, no rule is added.

`nix.settings.trusted-users`
: `snow deploy` copies the built system with `nix copy --to ssh://web1`, logged in as `deploy`. That copy is an import into the host's Nix store, performed by the host's Nix daemon on behalf of `deploy`. The daemon only accepts store paths that are signed by a key in its `trusted-public-keys`, unless the importing user is trusted. Systems you build locally are not signed, so without this line the copy fails with an error that the paths lack a signature from a trusted key. Being a trusted Nix user is itself root-equivalent, but this account already is.

If every path you deploy is signed by a key the host trusts (for example because CI builds and signs them), you can leave out `trusted-users`. snow does not sign paths itself.

## Create the user on nix-darwin

The snowglobe options are the same on a Mac:

```nix
{
  snowglobe.deploy.allowedUsers = [ "deploy" ];
  nix.settings.trusted-users = [ "deploy" ];
}
```

Creating the account works differently: nix-darwin only creates and manages users that you list in `users.knownUsers`, each with an explicit `uid`. You can also create the account in System Settings. Either way, put your public key in the account's `~/.ssh/authorized_keys`, and make sure Remote Login allows the user to sign in.

With Determinate Nix (`nix.enable = false`), `nix.settings` is not available; add `deploy` to `trusted-users` in `/etc/nix/nix.custom.conf` instead. The [nix-darwin guide](/snowglobe/darwin/) has a complete Mac configuration.

## The sudo rule the module writes

When `allowedUsers` is not empty, the module grants each listed user one command. On NixOS, through `security.sudo.extraRules`, the rule in `/etc/sudoers` reads:

```text
deploy ALL=(ALL:ALL) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate
```

On nix-darwin, through `security.sudo.extraConfig`, it lands in `/etc/sudoers.d/10-nix-darwin-extra-config`:

```text
deploy ALL=(root) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate
```

The rule names a fixed path rather than a store path, because a system's store path changes with every build. The helper takes the new store path and a mode as arguments, sets the system profile for `switch` and `boot`, and runs the activation script. [Security model](/snowglobe/security/#the-activation-helper) explains what the helper checks and why that does not limit what the deploy user can do.

snow calls the helper for everything that needs root: activation, the `dry-activate` that [reboot detection](/snowglobe/reboot-detection/) relies on, and, on NixOS, `snowglobe-activate reboot`. Nothing else in a deployment uses sudo. Remote [hooks](/snowglobe/hooks/) run as `deploy` without sudo.

## Apply the configuration once

Apply the configuration once the way you already deploy, so that the user, its key and the sudo rule exist. From your machine, use an account that can already run `sudo` on the host (`--ask-sudo-password` prompts for its password):

```bash
nixos-rebuild switch --flake .#web1 --target-host admin@web1 --sudo --ask-sudo-password
```

Running `sudo nixos-rebuild switch --flake .#web1` on the host itself works too. On a Mac, run `sudo darwin-rebuild switch --flake .#your-mac` on the Mac. After this, `snow deploy` keeps the configuration up to date like any other.

## Check the setup

Point your SSH client at the new user with a `User deploy` entry in `~/.ssh/config`; [Connect to your hosts](/snowglobe/connect/#ssh-config-entries) explains which names the entry must match and the other options. Then check three things. First, that key login works without a prompt:

```bash
ssh -o BatchMode=yes web1 id -un
```

This should print `deploy`. A `Permission denied (publickey)` error means the key is not in the account's authorized keys, or the client is offering a different key.

Second, that the sudo rule applies:

```bash
ssh deploy@web1 sudo -n -l /run/current-system/sw/bin/snowglobe-activate
```

If the rule applies, sudo prints the path back and exits zero. If it does not, `-n` makes sudo fail with `sudo: a password is required` instead of prompting.

Third, that the deploy user has no other sudo rights, which is the point of the exercise:

```bash
ssh deploy@web1 sudo -n true
```

This should fail with `sudo: a password is required`.

Finally, try a dry run. It builds, copies and plans, then runs a dry activation through the helper, which exercises the SSH login, the copy and the sudo rule without changing anything:

```bash
snow deploy --dry-run --redeploy web1
```

`--redeploy` is needed here because you have just applied this configuration by hand: without it, snow sees that the host already runs the system it built, reports it as unchanged, and stops before the copy. The report should end with a line such as `✓ 1 host deployed`, with the host marked `(dry-run)`. A failing copy at this stage usually means `trusted-users` is missing or the host has not been rebuilt yet.

## Hosts that use sudo-rs

If a NixOS host enables `security.sudo-rs.enable`, the classic `security.sudo` module is disabled, and the rule snowglobe writes to `security.sudo.extraRules` never reaches the host. The deployment then fails at the first sudo call with a password error.

Until the module handles sudo-rs itself, add the same rule to `security.sudo-rs.extraRules`, which uses the same format:

```nix
{ config, ... }:
{
  security.sudo-rs.enable = true;
  security.sudo-rs.extraRules = [
    {
      users = config.snowglobe.deploy.allowedUsers;
      commands = [
        {
          command = "/run/current-system/sw/bin/snowglobe-activate";
          options = [ "NOPASSWD" ];
        }
      ];
    }
  ];
}
```

To check it, run the helper without arguments. It only prints its usage, and it can only do that if sudo let it run:

```bash
ssh deploy@web1 sudo -n /run/current-system/sw/bin/snowglobe-activate
```

`Usage: snowglobe-activate <store-path> <mode>` means the rule works; a password error means it does not.

## Keep the user manager running

On NixOS, each SSH login starts a systemd user manager for `deploy`, which stops a few seconds after the last session ends. If a switch happens while that manager is stopping, the user part of the activation fails and the switch exits with status 4, which snow reports as a failed activation. The project's own VM tests hit this after logging in as the deploy user just before a deployment. Enabling lingering keeps the manager running and avoids the race:

```nix
{
  users.users.deploy.linger = true;
}
```

## Move off `NOPASSWD: ALL`

If your hosts already let a deploy account run anything without a password (a hand-written `NOPASSWD: ALL` rule, or `security.sudo.wheelNeedsPassword = false` with the account in `wheel`), you can switch to the helper without locking yourself out:

1. Add the snowglobe module and `allowedUsers`, and keep the old rule for now.
2. Rebuild the host by hand.
3. Try the helper directly. On NixOS, a dry activation of the running system changes nothing:

   ```bash
   ssh deploy@web1 'sudo -n snowglobe-activate "$(readlink -f /run/current-system)" dry-activate'
   ```

4. Run `snow deploy --redeploy web1`. snow finds the helper on its own and uses it. `--redeploy` makes snow activate the system again even though the host already runs it, so the helper is actually exercised.
5. Remove the old rule (or take the account out of `wheel`, or set `wheelNeedsPassword` back to `true`), deploy again, and repeat the checks above. `sudo -n true` should now fail.

On a NixOS host without the helper, snow logs a warning (`snowglobe-activate not found on web1, falling back to direct activation`) and runs `switch-to-configuration` with plain sudo, which only works under a broad rule like the one you are removing. nix-darwin hosts have no such fallback: they need the module before the first deployment.

## Troubleshooting

**A deployment fails with `sudo: a password is required` or `sudo: a terminal is required to read the password`.** Check, in order: the host imports the snowglobe module; the user `snow` logs in as is in `allowedUsers` (with `--user` or `User` in your SSH config, that is the name to list); the host was rebuilt after you added it; the host does not use [sudo-rs](#hosts-that-use-sudo-rs). The `sudo -n -l` check above tells you whether the rule is in place.

**The copy fails with a signature error.** The deploy user is not a trusted Nix user on the host, or the host was not rebuilt after you added it.

**Login works as you but not as `deploy`.** The SSH user comes from `--user`, from `User` in your SSH config, or defaults to your local user name. See [Choose the SSH user](/snowglobe/connect/#choose-the-ssh-user).

More symptoms are collected in [Troubleshooting](/snowglobe/troubleshooting/). The source for the helper and the sudo rules is in [`modules/nixos.nix`](https://github.com/luizribeiro/snowglobe/blob/main/modules/nixos.nix), [`modules/nixos-activate.nix`](https://github.com/luizribeiro/snowglobe/blob/main/modules/nixos-activate.nix) and [`modules/darwin.nix`](https://github.com/luizribeiro/snowglobe/blob/main/modules/darwin.nix).
