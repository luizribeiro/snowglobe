{ pkgs, pushTest }:

let
  scenarios = map
    (s: {
      inherit (s) name title;
      command = "snow ${s.command or "push"}${pkgs.lib.optionalString (s.args != "") " ${s.args}"}";
    })
    (import ./scenarios.nix);
  castToSvg = pkgs.writers.writePython3 "cast-to-svg" { } (builtins.readFile ./cast_to_svg.py);
in
pkgs.runCommand "snow-push-recordings" { SVG_TERM = "${pkgs.svg-term}/bin/svg-term"; } ''
  mkdir -p $out
  cp ${pushTest}/casts/* $out/
  cp ${pkgs.writeText "scenarios.json" (builtins.toJSON scenarios)} $out/scenarios.json
  for cast in $out/*.cast; do
    ${castToSvg} "$cast" "''${cast%.cast}.svg"
  done
''
