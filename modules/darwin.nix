{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.snowglobe.deploy;

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
    ./deploy.nix
  ];

  options.snowglobe.deploy = {
    allowedUsers = mkOption {
      type = types.listOf types.str;
      default = [];
      description = ''
        Users allowed to run snowglobe-activate without password.

        These users can activate any system from the Nix store without a
        password. This is meant for push deployments. On Darwin, only the
        'switch' activation mode is supported.

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
    # Note: nix-darwin uses extraConfig (string), not extraRules (list)
    security.sudo.extraConfig = mkIf (cfg.allowedUsers != []) (
      concatMapStringsSep "\n" (user:
        "${user} ALL=(root) NOPASSWD: /run/current-system/sw/bin/snowglobe-activate"
      ) cfg.allowedUsers
    );
  };
}
