{ pkgs, systemctl, nixEnv }:

pkgs.writeScriptBin "snowglobe-activate" ''
  #!${pkgs.bash}/bin/bash
  set -e

  ${import ./store-path-check.nix}

  usage() {
    echo "Usage: snowglobe-activate <store-path> <mode>" >&2
    echo "       snowglobe-activate reboot" >&2
    echo "Modes: switch, boot, test, dry-activate" >&2
    exit 1
  }

  # Rebooting lives here so deploy users only need sudo for this one helper
  if [ "$1" = "reboot" ]; then
    [ $# -eq 1 ] || usage
    exec ${systemctl} reboot
  fi

  if [ $# -lt 2 ]; then
    usage
  fi

  STORE_PATH="$1"
  MODE="$2"

  require_store_path "$STORE_PATH"

  # Validate mode is one of the allowed values
  case "$MODE" in
    switch|boot|test|dry-activate)
      ;;
    *)
      echo "Error: Invalid mode '$MODE'. Must be one of: switch, boot, test, dry-activate" >&2
      exit 1
      ;;
  esac

  # NixOS uses /bin/switch-to-configuration
  ACTIVATION_SCRIPT="$STORE_PATH/bin/switch-to-configuration"

  if [ ! -f "$ACTIVATION_SCRIPT" ]; then
    echo "Error: Activation script not found at $ACTIVATION_SCRIPT" >&2
    exit 1
  fi

  # Update system profile for persistent modes (switch/boot)
  # Ephemeral modes (test/dry-activate) skip this to avoid polluting generation history
  case "$MODE" in
    switch|boot)
      ${nixEnv} --profile /nix/var/nix/profiles/system --set "$STORE_PATH"
      ;;
  esac

  # Execute the activation script with the specified mode
  exec "$ACTIVATION_SCRIPT" "$MODE"
''
