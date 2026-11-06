{ pkgs }:

let
  fakeNixEnv = pkgs.writeShellScript "fake-nix-env" ''
    echo "$*" > "$TMPDIR/nix-env-called"
    [ ! -e "$TMPDIR/nix-env-fails" ]
  '';
  fakeSystem = pkgs.writeTextFile {
    name = "darwin-system-26.05";
    destination = "/activate";
    executable = true;
    text = ''
      #!${pkgs.runtimeShell}
      [ -e "$TMPDIR/nix-env-called" ] || { echo "activate ran before the profile was set" >&2; exit 1; }
      touch "$TMPDIR/activate-called"
    '';
  };
  systemWithoutActivate = pkgs.writeTextDir "share/empty" "";
  helper = import ./darwin-activate.nix {
    inherit pkgs;
    nixEnv = fakeNixEnv;
  };
in
pkgs.runCommand "darwin-activate-test" { } ''
  nix_env="$TMPDIR/nix-env-called"
  activated="$TMPDIR/activate-called"
  ${import ./activate-test-lib.nix {
    inherit helper;
    sideEffect = "$nix_env";
    sideEffectName = "touch the profile";
  }}

  "$activate" ${fakeSystem} switch
  [ "$(cat "$nix_env")" = "--profile /nix/var/nix/profiles/system --set ${fakeSystem}" ] \
    || fail "switch ran nix-env $(cat "$nix_env")"
  [ -e "$activated" ] || fail "switch did not run activate"
  rm "$nix_env" "$activated"

  touch "$TMPDIR/nix-env-fails"
  "$activate" ${fakeSystem} switch 2>/dev/null && fail "switch succeeded although nix-env failed"
  [ -e "$nix_env" ] || fail "nix-env did not run"
  [ -e "$activated" ] && fail "activate ran although nix-env failed"
  rm "$nix_env" "$TMPDIR/nix-env-fails"

  expect_error "Boot mode is not supported" ${fakeSystem} boot
  expect_error "Test mode is not supported" ${fakeSystem} test
  expect_error "Dry-activate mode is not supported" ${fakeSystem} dry-activate
  expect_error "Invalid mode" ${fakeSystem} poweroff
  expect_error "Usage" ${fakeSystem}
  expect_error "Invalid store path" /tmp/evil switch
  expect_error "Activation script not found" ${systemWithoutActivate} switch
  expect_error "Activation script not found" /nix/store/$hash-darwin-system-26.05 switch
  expect_traversal_rejected activate switch

  touch $out
''
