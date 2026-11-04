{ pkgs, pushTest }:

let
  scenarios = map
    (s: {
      inherit (s) name title;
      command = "snow ${s.command or "push"}${pkgs.lib.optionalString (s.args != "") " ${s.args}"}";
    })
    (import ./scenarios.nix);
in
pkgs.runCommand "snow-push-recordings" { } ''
  mkdir -p $out
  cp ${pushTest}/casts/* $out/
  cp ${pkgs.writeText "scenarios.json" (builtins.toJSON scenarios)} $out/scenarios.json
''
