{ pkgs }:

let
  fakePrivileged = name: pkgs.writeShellScript "fake-${name}" ''
    echo "${name} $*" >> "$TMPDIR/privileged-called"
  '';
  helper = import ./nixos-activate.nix {
    inherit pkgs;
    systemctl = fakePrivileged "systemctl";
    nixEnv = fakePrivileged "nix-env";
  };
  packageNotSystem = pkgs.writeTextDir "bin/hello" "";
in
pkgs.runCommand "nixos-activate-test" { } ''
  marker="$TMPDIR/privileged-called"
  ${import ./activate-test-lib.nix {
    inherit helper;
    sideEffect = "$marker";
    sideEffectName = "call systemctl or nix-env";
  }}

  expect_failure() { expect_error "" "$@"; }

  "$activate" reboot
  [ "$(cat "$marker")" = "systemctl reboot" ] || fail "reboot ran $(cat "$marker")"
  rm "$marker"

  expect_failure reboot now
  expect_failure reboot ""
  expect_failure reboot /nix/store/abc-system
  expect_failure
  expect_failure /nix/store/abc-system
  expect_error "Invalid store path" /tmp/evil switch
  expect_error "Invalid mode" /nix/store/$hash-nixos-system-boron poweroff
  expect_error "Invalid mode" /nix/store/$hash-nixos-system-boron reboot
  expect_error "Activation script not found" /nix/store/$hash-nixos-system-boron test
  expect_error "Activation script not found" ${packageNotSystem} switch
  expect_error "Activation script not found" ${packageNotSystem} boot
  expect_traversal_rejected bin/switch-to-configuration test

  touch $out
''
