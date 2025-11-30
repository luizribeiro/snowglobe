{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.snowglobe.push;

  activationHelper = pkgs.writeScriptBin "snowglobe-activate" ''
    #!${pkgs.bash}/bin/bash
    set -e

    if [ $# -lt 2 ]; then
      echo "Usage: snowglobe-activate <store-path> <mode>" >&2
      echo "Modes: switch, boot, test, dry-activate" >&2
      exit 1
    fi

    STORE_PATH="$1"
    MODE="$2"

    # Validate store path starts with /nix/store/
    if [[ ! "$STORE_PATH" =~ ^/nix/store/ ]]; then
      echo "Error: Invalid store path (must start with /nix/store/)" >&2
      exit 1
    fi

    # Validate mode is one of the allowed values
    case "$MODE" in
      switch|boot|test|dry-activate)
        ;;
      *)
        echo "Error: Invalid mode '$MODE'. Must be one of: switch, boot, test, dry-activate" >&2
        exit 1
        ;;
    esac

    # Update system profile for persistent modes (switch/boot)
    # Ephemeral modes (test/dry-activate) skip this to avoid polluting generation history
    case "$MODE" in
      switch|boot)
        ${pkgs.nix}/bin/nix-env --profile /nix/var/nix/profiles/system --set "$STORE_PATH"
        ;;
    esac

    # NixOS uses /bin/switch-to-configuration
    ACTIVATION_SCRIPT="$STORE_PATH/bin/switch-to-configuration"

    if [ ! -f "$ACTIVATION_SCRIPT" ]; then
      echo "Error: Activation script not found at $ACTIVATION_SCRIPT" >&2
      exit 1
    fi

    # Execute the activation script with the specified mode
    exec "$ACTIVATION_SCRIPT" "$MODE"
  '';
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
