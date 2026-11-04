let
  # snowglobe's maintainer's terminal: "Custom Monokai" with Hack, so the
  # demos look as they did when the commands were run
  terminalLook = builtins.fromJSON (builtins.readFile ./terminal-look.json);

  withLook = { pkgs, look, name, script, libraries ? [ ] }:
    pkgs.writeShellScriptBin name ''
      export TERMINAL_LOOK=${pkgs.writeText "terminal-look.json" (builtins.toJSON look)}
      exec ${pkgs.writers.writePython3 name { inherit libraries; } (builtins.readFile script)} "$@"
    '';
in
{
  inherit terminalLook;

  recorder = { pkgs, look ? terminalLook }: withLook {
    inherit pkgs look;
    name = "record";
    script = ./record.py;
    libraries = [ pkgs.python3Packages.pexpect ];
  };

  # scenarios.json carries what the casts can't, such as titles and their
  # order, to whatever publishes the demos
  renderDemos = { pkgs, casts, scenarios ? [ ], look ? terminalLook, name ? "demos" }:
    let
      castToSvg = withLook {
        inherit pkgs look;
        name = "cast-to-svg";
        script = ./cast_to_svg.py;
      };
    in
    pkgs.runCommand name { SVG_TERM = "${pkgs.svg-term}/bin/svg-term"; } ''
      mkdir -p $out
      cp ${casts}/* $out/
      cp ${pkgs.writeText "scenarios.json" (builtins.toJSON scenarios)} $out/scenarios.json
      for cast in $out/*.cast; do
        ${castToSvg}/bin/cast-to-svg "$cast" "''${cast%.cast}.svg"
      done
    '';
}
