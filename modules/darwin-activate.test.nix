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
  activate=${helper}/bin/snowglobe-activate
  nix_env="$TMPDIR/nix-env-called"
  activated="$TMPDIR/activate-called"

  fail() { echo "FAIL: $*" >&2; exit 1; }

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

  for args in "${fakeSystem} boot" "${fakeSystem} test" "${fakeSystem} dry-activate" "/tmp/evil switch" "${fakeSystem}" "${systemWithoutActivate} switch"; do
    "$activate" $args 2>/dev/null && fail "snowglobe-activate $args should have failed"
    [ -e "$nix_env" ] && fail "snowglobe-activate $args must not touch the profile"
  done

  touch $out
''
