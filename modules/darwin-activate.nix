{ pkgs, nixEnv }:

pkgs.writeScriptBin "snowglobe-activate" ''
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
''
