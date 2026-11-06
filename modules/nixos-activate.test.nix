{ pkgs }:

let
  hash = "0123456789abcdfghijklmnpqrsvwxyz";
  fakeSystemctl = pkgs.writeShellScript "fake-systemctl" ''
    echo "$*" > "$TMPDIR/systemctl-called"
  '';
  helper = import ./nixos-activate.nix {
    inherit pkgs;
    systemctl = fakeSystemctl;
  };
in
pkgs.runCommand "nixos-activate-test" { } ''
  activate=${helper}/bin/snowglobe-activate
  marker="$TMPDIR/systemctl-called"

  fail() { echo "FAIL: $*" >&2; exit 1; }

  expect_error() {
    local message="$1" output
    shift
    output=$("$activate" "$@" 2>&1) && fail "snowglobe-activate $* should have failed"
    [[ "$output" == *"$message"* ]] || fail "snowglobe-activate $* should fail with '$message', got: $output"
    [ ! -e "$marker" ] || fail "snowglobe-activate $* must not call systemctl"
  }

  expect_failure() { expect_error "" "$@"; }

  "$activate" reboot
  [ "$(cat "$marker")" = "reboot" ] || fail "reboot ran systemctl $(cat "$marker")"
  rm "$marker"

  expect_failure reboot now
  expect_failure reboot ""
  expect_failure reboot /nix/store/abc-system
  expect_failure
  expect_failure /nix/store/abc-system
  expect_error "Invalid store path" /tmp/evil switch
  expect_error "Invalid mode" /nix/store/${hash}-nixos-system-boron poweroff
  expect_error "Invalid mode" /nix/store/${hash}-nixos-system-boron reboot
  expect_error "Activation script not found" /nix/store/${hash}-nixos-system-boron test

  evil="$TMPDIR/evil/bin/switch-to-configuration"
  mkdir -p "$(dirname "$evil")"
  printf '#!/bin/sh\ntouch "$TMPDIR/pwned"\n' > "$evil"
  chmod +x "$evil"
  traversal="/nix/store/../..$TMPDIR/evil"
  [ -x "$traversal/bin/switch-to-configuration" ] || fail "$traversal does not reach $evil"
  expect_error "Invalid store path" "$traversal" test
  [ ! -e "$TMPDIR/pwned" ] || fail "ran a script outside the store"
  expect_error "Invalid store path" /nix/store/${hash}-system/../../../tmp/evil switch

  touch $out
''
