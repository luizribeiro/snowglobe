{ pkgs }:

let
  hash = "0123456789abcdfghijklmnpqrsvwxyz";
  check = pkgs.writeShellScript "check-store-path" ''
    ${import ./store-path-check.nix}
    require_store_path "$1"
  '';
in
pkgs.runCommand "store-path-check-test" { } ''
  fail() { echo "FAIL: $*" >&2; exit 1; }

  expect_valid() {
    ${check} "$1" || fail "'$1' should be a valid store path"
  }

  expect_rejected() {
    ! ${check} "$1" 2>"$TMPDIR/err" || fail "'$1' should have been rejected"
    grep -q "Invalid store path" "$TMPDIR/err" || fail "'$1' failed for another reason: $(cat "$TMPDIR/err")"
  }

  expect_valid /nix/store/${hash}-nixos-system-bravo-26.05
  expect_valid "/nix/store/${hash}-a+b_c?d=e.f"
  expect_valid ${check}

  expect_rejected ""
  expect_rejected /tmp/evil
  expect_rejected /nix/store/
  expect_rejected /nix/store/../../home/alice/evil
  expect_rejected /nix/store/./${hash}-system
  expect_rejected /nix/store/${hash}-system/
  expect_rejected /nix/store/${hash}-system/..
  expect_rejected /nix/store/${hash}-system/bin
  expect_rejected /nix/store/${hash}-foo/../../../tmp
  expect_rejected /nix/store/${hash}-.
  expect_rejected /nix/store/${hash}-..
  expect_rejected /nix/store/${hash}-.hidden
  expect_rejected /nix/store/${hash}-
  expect_rejected /nix/store/-system
  expect_rejected /nix/store/abc-system
  expect_rejected /nix/store/${hash}x-system
  expect_rejected /nix/store/${pkgs.lib.toUpper hash}-system
  expect_rejected /nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-system
  expect_rejected //nix/store/${hash}-system
  expect_rejected " /nix/store/${hash}-system"
  expect_rejected "/nix/store/${hash}-system "
  expect_rejected "/nix/store/${hash}-sys tem"
  expect_rejected "/nix/store/${hash}-sys"$'\t'"tem"
  expect_rejected "/nix/store/${hash}-sÿstem"
  expect_rejected "/nix/store/${hash}-system"$'\n'"/tmp/evil"
  expect_rejected "/tmp/evil"$'\n'"/nix/store/${hash}-system"
  expect_rejected "/nix/store/${hash}-system"$'\n'

  touch $out
''
