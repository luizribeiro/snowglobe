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
  snowglobe.push.allowedUsers = [ "deploy" ];
}
```

The flake side is in the [README quickstart](../README.md#quickstart) for
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

## How snow uses it

Before the reboot check and before activating, snow runs
`command -v snowglobe-activate` on the host over SSH:

- exit 0: the helper is there, snow uses it.
- exit 1: it isn't. On NixOS, snow runs things directly with `sudo`. For
  activation it also warns
  (`snowglobe-activate not found on <host>, falling back to direct activation…`).
  This needs passwordless sudo for `nix-env` (switch and boot) and
  `<system>/bin/switch-to-configuration` (activation and the dry-activate
  check). On Darwin it fails, see [darwin.md](darwin.md#without-the-module).
- anything else (SSH failed or timed out): the host fails.

To reboot, snow runs one command on the host that uses
`sudo snowglobe-activate reboot` when the helper is there, and
`sudo systemctl reboot` when it isn't. Without the helper, that needs
passwordless sudo for `systemctl reboot` too.

With the helper, snow doesn't set the profile itself.

## Running it by hand

```bash
P=/nix/store/qqz7czg0f46pg7n3f0xqmiv14bampzr7-nixos-system-25.05
sudo snowglobe-activate $P switch
sudo snowglobe-activate $P boot          # next boot only
sudo snowglobe-activate $P test          # until the next reboot
sudo snowglobe-activate $P dry-activate  # show what would change
sudo snowglobe-activate reboot           # NixOS only
```

On Darwin only `switch` works.

## What it checks

1. The path is one top-level store path, `/nix/store/<hash>-<name>`. Not
   `/run/current-system`, not a path inside a store path, no `..`.
2. The mode is `switch`, `boot`, `test` or `dry-activate` (only `switch` on
   Darwin).
3. The system has an activation script: `bin/switch-to-configuration` on
   NixOS, `activate` on Darwin.

Then:

- NixOS: sets the system profile for `switch` and `boot`, and passes the
  mode to `switch-to-configuration`. `reboot` runs `systemctl reboot`.
- Darwin: always sets the profile (nix-darwin's `activate` doesn't record a
  generation), then runs `<system>/activate` with no argument. No `reboot`.

## Security

### Allowed users are root-equivalent

Any user who can push without a password is effectively root. Activation
runs the system's activation script as root, and whoever calls the helper
picks the store path. Writing to `/nix/store` needs root, but adding a path
to it doesn't: anyone who can use the Nix daemon can build a path whose
`bin/switch-to-configuration` does whatever they want.

Every push tool needs a user that can become root on the host, since
activation runs as root. deploy-rs (`interactiveSudo`) and
`nixos-rebuild --target-host` (`--ask-sudo-password`) can ask for that user's
sudo password. snow can't yet, so its deploy user becomes root without one.

The checks above catch mistakes, like a wrong mode or a path that isn't a
system. They are not a security boundary against an allowed user.

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

**`snowglobe-activate not found on <host>, falling back to direct activation`**
The module isn't active on the host yet. Import it, add the SSH user to
`allowedUsers`, rebuild by hand once.

**`Invalid store path '…' (must be /nix/store/<hash>-<name>)`**
Pass the resolved path, e.g. `"$(readlink -f /run/current-system)"`.

**`Activation script not found`**
The path isn't a system. Look for `<path>/bin/switch-to-configuration`
(NixOS) or `<path>/activate` (Darwin).

**The helper check fails**
That's SSH. Try `ssh -o BatchMode=yes host echo test`.

**sudo asks for a password** (`sudo: a password is required`)

1. Is the module imported, and the SSH user in `allowedUsers`?
2. Did you rebuild by hand after adding them?
3. `ssh host sudo -n -l /run/current-system/sw/bin/snowglobe-activate`
   prints the path if the rule applies, and fails without prompting if not.
4. Look for the rule in `/etc/sudoers` (NixOS) or
   `/etc/sudoers.d/10-nix-darwin-extra-config` (Darwin).
5. sudo-rs? It gets no rule.

## Moving off `NOPASSWD: ALL`

1. Add the module. Keep your old sudoers rule for now.
2. Rebuild.
3. Try the helper: `ssh host 'sudo snowglobe-activate "$(readlink -f /run/current-system)" switch'`
4. `snow push host`. It picks the helper up on its own.
5. Remove the old rule and rebuild again.

See also [push.md](push.md) and [darwin.md](darwin.md).
