{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.snowglobe.deploy;

  activationHelper = import ./nixos-activate.nix {
    inherit pkgs;
    systemctl = "${config.systemd.package}/bin/systemctl";
    nixEnv = "${pkgs.nix}/bin/nix-env";
  };
in
{
  imports = [
    ./bootstrap.nix
    ./hooks.nix
    ./deploy.nix
  ];
  options.snowglobe.deploy = {
    allowedUsers = mkOption {
      type = types.listOf types.str;
      default = [];
      description = ''
        Users allowed to run snowglobe-activate without password.

        These users can activate any system from the Nix store, and reboot the
        host, without a password. This is meant for push deployments.

        Users in this list are effectively root on this host: anyone who can
        use the Nix daemon can add a store path with their own activation
        script and run it as root through snowglobe-activate. Use a dedicated
        deploy user, and guard its SSH key like a root key.
      '';
      example = [ "deploy" ];
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
