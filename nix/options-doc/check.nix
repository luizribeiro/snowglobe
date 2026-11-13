{ pkgs, page }:

pkgs.runCommand "options-doc-check" { } ''
  if ! diff -u ${../../docs/options.md} ${page}; then
    echo "docs/options.md is out of date with the modules. Regenerate it:" >&2
    echo "  nix build .#options-md && cat result > docs/options.md" >&2
    exit 1
  fi
  touch $out
''
