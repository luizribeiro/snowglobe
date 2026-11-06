{ pkgs, nixEnv }:

pkgs.writeScriptBin "snowglobe-activate" ''
  #!${pkgs.bash}/bin/bash
  set -e

  ${import ./store-path-check.nix}

  if [ $# -lt 2 ]; then
    echo "Usage: snowglobe-activate <store-path> <mode>" >&2
    echo "Modes: switch (only mode supported on Darwin)" >&2
    exit 1
  fi

  STORE_PATH="$1"
  MODE="$2"

  require_store_path "$STORE_PATH"

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

  # Darwin's activate doesn't record a generation; without it, rollback and
  # boot-time activation still see the previous system
  ${nixEnv} --profile /nix/var/nix/profiles/system --set "$STORE_PATH"

  # Darwin's activate doesn't take mode arguments
  exec "$ACTIVATION_SCRIPT"
''
