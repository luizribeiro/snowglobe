{ pkgs }:

let
  helper = import ./nixos-activate.nix { inherit pkgs; };
in
pkgs.runCommand "nixos-activate-test" { } ''
  activate=${helper}/bin/snowglobe-activate

  expect_failure() {
    if "$activate" "$@" 2>/dev/null; then
      echo "FAIL: snowglobe-activate $* should have failed" >&2
      exit 1
    fi
  }

  expect_failure
  expect_failure /nix/store/abc-system
  expect_failure /tmp/evil switch
  expect_failure /nix/store/abc-system poweroff
  expect_failure /nix/store/abc-system reboot
  expect_failure /nix/store/abc-system test

  touch $out
''
