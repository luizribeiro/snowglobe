{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.snowglobe.push;

  activationHelper = pkgs.writeScriptBin "snowglobe-activate" ''
    #!${pkgs.bash}/bin/bash
    set -e

    if [ $# -lt 2 ]; then
      echo "Usage: snowglobe-activate <store-path> <mode>" >&2
      echo "Modes: switch (only mode supported on Darwin)" >&2
      exit 1
    fi

    STORE_PATH="$1"
    MODE="$2"

    # Validate store path starts with /nix/store/
    if [[ ! "$STORE_PATH" =~ ^/nix/store/ ]]; then
      echo "Error: Invalid store path (must start with /nix/store/)" >&2
      exit 1
    fi

    # Darwin only supports switch mode - error on unsupported modes
    case "$MODE" in
      switch)
        ;;
      boot)
        echo "Error: Boot mode is not supported on Darwin" >&2
        echo "Darwin's activate script does not support delayed activation" >&2
        exit 1
        ;;
      test)
        echo "Error: Test mode is not supported on Darwin" >&2
        echo "Darwin's activate script does not support temporary activation" >&2
        exit 1
        ;;
      dry-activate)
        echo "Error: Dry-activate mode is not supported on Darwin" >&2
        echo "Darwin's activate script does not support dry-run mode" >&2
        exit 1
        ;;
      *)
        echo "Error: Invalid mode '$MODE'. Darwin only supports: switch" >&2
        exit 1
        ;;
    esac

    # Darwin uses /activate at the root of the system closure
    ACTIVATION_SCRIPT="$STORE_PATH/activate"

    if [ ! -f "$ACTIVATION_SCRIPT" ]; then
      echo "Error: Activation script not found at $ACTIVATION_SCRIPT" >&2
      exit 1
    fi

    # Execute the activation script (Darwin's activate doesn't take mode arguments)
    exec "$ACTIVATION_SCRIPT"
  '';
in
{
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
