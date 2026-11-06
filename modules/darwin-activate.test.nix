{ pkgs }:

let
  hash = "0123456789abcdfghijklmnpqrsvwxyz";
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
  activate=${helper}/bin/snowglobe-activate
  nix_env="$TMPDIR/nix-env-called"
  activated="$TMPDIR/activate-called"

  fail() { echo "FAIL: $*" >&2; exit 1; }

  expect_error() {
    local message="$1" output
    shift
    output=$("$activate" "$@" 2>&1) && fail "snowglobe-activate $* should have failed"
    [[ "$output" == *"$message"* ]] || fail "snowglobe-activate $* should fail with '$message', got: $output"
    [ ! -e "$nix_env" ] || fail "snowglobe-activate $* must not touch the profile"
  }

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
  expect_error "Activation script not found" /nix/store/${hash}-darwin-system-26.05 switch

  evil="$TMPDIR/evil/activate"
  mkdir -p "$(dirname "$evil")"
  printf '#!/bin/sh\ntouch "$TMPDIR/pwned"\n' > "$evil"
  chmod +x "$evil"
  traversal="/nix/store/../..$TMPDIR/evil"
  [ -x "$traversal/activate" ] || fail "$traversal does not reach $evil"
  expect_error "Invalid store path" "$traversal" switch
  [ ! -e "$TMPDIR/pwned" ] || fail "ran a script outside the store"
  expect_error "Invalid store path" /nix/store/${hash}-system/../../../tmp/evil switch

  touch $out
''
