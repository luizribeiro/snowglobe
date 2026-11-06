# Bash function shared by the activation helpers. It accepts only a single
# top-level store path, so "/nix/store/../home/alice/evil" can't run as root.
# It is not a security boundary: any nix daemon user can add store paths.
''
  require_store_path() {
    local hash='[0123456789abcdfghijklmnpqrsvwxyz]{32}'
    local name='[A-Za-z0-9+_?=-][A-Za-z0-9+._?=-]*'
    local LC_ALL=C
    if [[ ! "$1" =~ ^/nix/store/$hash-$name$ ]]; then
      echo "Error: Invalid store path '$1' (must be /nix/store/<hash>-<name>)" >&2
      exit 1
    fi
  }
''
