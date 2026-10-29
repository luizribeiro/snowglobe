{ pkgs }:

name: tests:
let
  failures = pkgs.lib.debug.runTests tests;
  testCount = builtins.length (builtins.filter (pkgs.lib.hasPrefix "test") (builtins.attrNames tests));
in
assert pkgs.lib.assertMsg (testCount > 0) "${name}: no attributes start with \"test\", so nothing would run";
pkgs.runCommand "${name}-tests" { } (
  if failures == [ ] then
    ''
      echo "${name}: ${toString testCount} tests passed"
      touch $out
    ''
  else
    ''
      echo ${pkgs.lib.escapeShellArg (pkgs.lib.generators.toPretty { } failures)} >&2
      exit 1
    ''
)
