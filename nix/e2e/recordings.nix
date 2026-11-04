{ pkgs, pushTest }:

let
  scenarios = map
    (s: {
      inherit (s) name title;
      command = "snow ${s.command or "push"}${pkgs.lib.optionalString (s.args != "") " ${s.args}"}";
    })
    (import ./scenarios.nix);
  castToSvg = (import ../demos).castToSvg { inherit pkgs; };
in
pkgs.runCommand "snow-push-recordings" { SVG_TERM = "${pkgs.svg-term}/bin/svg-term"; } ''
  mkdir -p $out
  cp ${pushTest}/casts/* $out/
  cp ${pkgs.writeText "scenarios.json" (builtins.toJSON scenarios)} $out/scenarios.json
  for cast in $out/*.cast; do
    ${castToSvg}/bin/cast-to-svg "$cast" "''${cast%.cast}.svg"
  done
''
