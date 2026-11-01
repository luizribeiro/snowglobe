{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.snowglobe.push;

  # With nix.enable = false (e.g. Determinate Nix) nix-darwin doesn't manage
  # nix, so use the host's own install; sudo's secure_path may not include it.
  nixEnv =
    if config.nix.enable or true
    then "${config.nix.package}/bin/nix-env"
    else "/nix/var/nix/profiles/default/bin/nix-env";

  activationHelper = import ./darwin-activate.nix {
    inherit pkgs nixEnv;
  };
in
{
  imports = [
    ./bootstrap.nix
    ./hooks.nix
    ./push.nix
  ];

  options.snowglobe.push = {
    allowedUsers = mkOption {
      type = types.listOf types.str;
      default = [];
      description = ''
        Users allowed to run snowglobe-activate without password.

        These users will be able to activate any nix-darwin configuration from
        the Nix store without requiring a password. This is designed for
        automated deployment scenarios (CI/CD, remote push deployments).

        Security note: Users in this list can activate any system configuration
        from /nix/store/, but cannot modify the store itself (which requires root).
        On Darwin, only the 'switch' activation mode is supported.
      '';
      example = [ "alice" "pusher" ];
    };
  };

  config = {
    # Always install the activation helper system-wide
    environment.systemPackages = [ activationHelper ];

    # Configure passwordless sudo for specified users
    # Note: nix-darwin uses extraConfig (string), not extraRules (list)
    security.sudo.extraConfig = mkIf (cfg.allowedUsers != []) (
      concatMapStringsSep "\n" (user:
        "${user} ALL=(root) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate"
      ) cfg.allowedUsers
    );
  };
}
