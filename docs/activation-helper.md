# snowglobe-activate

`snowglobe-activate` is a tiny script the module installs on every host.
snow goes through it to activate, to check if a reboot is needed, and on
NixOS to reboot. So your deploy user gets passwordless sudo for one command,
not for everything.

## Why

Activating a system means running something like:

```bash
sudo /nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-nixos-system-25.05/bin/switch-to-configuration switch
```

The hash changes with every build, so that path can't go in sudoers, and
people end up with `NOPASSWD: ALL`. The helper sits at a fixed path,
`/run/current-system/sw/bin/snowglobe-activate`, takes the store path and
mode as arguments, checks them, sets the system profile and runs the
activation script. sudoers gets one rule for that one path.

The rule points at the running system's helper. That helper activates the
new system, and then `/run/current-system` points at the new one, new
helper included. So updating the helper itself is fine.

## Setup

Import the module and list your deploy users:

```nix
{
  snowglobe.deploy.allowedUsers = [ "deploy" ];
}
```

The flake side is in the [README setup](../README.md#setup) for
NixOS and in [darwin.md](darwin.md#setup) for Macs. Apply it once by hand
(`sudo nixos-rebuild switch`, or `sudo darwin-rebuild switch --flake .#your-mac`).

The helper is always installed. The sudo rule is only added when
`allowedUsers` isn't empty:

```
# NixOS, through security.sudo.extraRules
deploy ALL=(ALL:ALL) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate

# nix-darwin, through security.sudo.extraConfig, in /etc/sudoers.d/10-nix-darwin-extra-config
deploy ALL=(root) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate
```

Hosts on sudo-rs get no rule.

## Without the helper

If a NixOS host doesn't have it yet, snow warns and runs the activation
directly with `sudo`, which then needs to work without a password. On a Mac
there's no fallback: see [darwin.md](darwin.md#without-the-module).

## By hand

```bash
sudo snowglobe-activate /nix/store/…-nixos-system-… switch   # or boot, test, dry-activate
sudo snowglobe-activate reboot                               # NixOS only
```

It only accepts a top-level store path that contains a system, like
`"$(readlink -f /run/current-system)"`. Those checks catch typos. They're not
a security boundary, see below.

## Security

### Allowed users are root-equivalent

Any user who can push without a password is effectively root. Activation
runs the system's activation script as root, and whoever calls the helper
picks the store path. Adding a path to `/nix/store` doesn't need root:
anyone who can use the Nix daemon can build a system that does whatever they
want, then activate it.

Every push tool needs a user that can become root on the host, since
activation runs as root. deploy-rs and `nixos-rebuild --target-host` can ask
for that user's sudo password. `snow push` can't yet, so its deploy user
becomes root without one.

What the helper does give you: one command in sudoers, which you can read,
instead of `NOPASSWD: ALL`, at a path that doesn't change when the system
does.

Signed deployments are on the roadmap: the host would only activate systems
signed with a deploy key, and the deploy user would no longer be
root-equivalent. Not built yet.

### Advice

1. A dedicated deploy user, and only that one. A personal account in
   `allowedUsers` is root without a password.
2. Treat its SSH key like a root key. Key-only login, and the key only on the
   machine or CI runner that deploys.
3. Keep an eye on activations in the system logs.

## Troubleshooting

**`snowglobe-activate not found on <host>, falling back to direct activation`**:
the module isn't active on the host yet. Import it, add the SSH user to
`allowedUsers`, rebuild by hand once.

**sudo asks for a password** (`sudo: a password is required`):

1. Is the module imported, and the SSH user in `allowedUsers`?
2. Did you rebuild by hand after adding them?
3. `ssh host sudo -n -l /run/current-system/sw/bin/snowglobe-activate`
   prints the path if the rule applies, and fails without prompting if not.
4. sudo-rs? It gets no rule.

## Moving off `NOPASSWD: ALL`

1. Add the module. Keep your old sudoers rule for now.
2. Rebuild.
3. Try the helper: `ssh host 'sudo snowglobe-activate "$(readlink -f /run/current-system)" switch'`
4. `snow push host`. It picks the helper up on its own.
5. Remove the old rule and rebuild again.

See also [push.md](push.md) and [darwin.md](darwin.md).
