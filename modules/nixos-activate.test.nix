{ pkgs }:

let
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

  expect_failure() {
    if "$activate" "$@" 2>/dev/null; then
      echo "FAIL: snowglobe-activate $* should have failed" >&2
      exit 1
    fi
    if [ -e "$marker" ]; then
      echo "FAIL: snowglobe-activate $* must not call systemctl" >&2
      exit 1
    fi
  }

  "$activate" reboot
  [ "$(cat "$marker")" = "reboot" ] || { echo "FAIL: reboot ran systemctl $(cat "$marker")" >&2; exit 1; }
  rm "$marker"

  expect_failure reboot now
  expect_failure reboot ""
  expect_failure reboot /nix/store/abc-system
  expect_failure
  expect_failure /nix/store/abc-system
  expect_failure /tmp/evil switch
  expect_failure /nix/store/abc-system poweroff
  expect_failure /nix/store/abc-system reboot
  expect_failure /nix/store/abc-system test

  touch $out
''
