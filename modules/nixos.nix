{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.snowglobe.push;

  activationHelper = import ./nixos-activate.nix { inherit pkgs; };
in
{
  imports = [
    ./bootstrap.nix
    ./hooks.nix
  ];
  options.snowglobe.push = {
    allowedUsers = mkOption {
      type = types.listOf types.str;
      default = [];
      description = ''
        Users allowed to run snowglobe-activate without password.

        These users will be able to activate any NixOS configuration from the
        Nix store without requiring a password. This is designed for automated
        deployment scenarios (CI/CD, remote push deployments).

        Security note: Users in this list can activate any system configuration
        from /nix/store/, but cannot modify the store itself (which requires root).
      '';
      example = [ "alice" "pusher" ];
    };
  };

  config = {
    # Always install the activation helper system-wide
    environment.systemPackages = [ activationHelper ];

    # Configure passwordless sudo for specified users
    security.sudo.extraRules = mkIf (cfg.allowedUsers != []) [
      {
        users = cfg.allowedUsers;
        commands = [
          {
            command = "/run/current-system/sw/bin/snowglobe-activate";
            options = [ "NOPASSWD" ];
          }
        ];
      }
    ];
  };
}
