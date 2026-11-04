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

  # Takes directories as `record` writes them. demos.json lists their
  # demo.json files in the given order, for whatever publishes the demos.
  renderDemos = { pkgs, demos, look ? terminalLook, name ? "demos" }:
    let
      castToSvg = withLook {
        inherit pkgs look;
        name = "cast-to-svg";
        script = ./cast_to_svg.py;
      };
    in
    pkgs.runCommand name {
      SVG_TERM = "${pkgs.svg-term}/bin/svg-term";
      nativeBuildInputs = [ pkgs.jq castToSvg ];
    } ''
      mkdir -p $out
      for demo in ${pkgs.lib.escapeShellArgs demos}; do
        demo_name=$(jq -r .name "$demo/demo.json")
        if [ -e "$out/$demo_name.cast" ]; then
          echo "renderDemos: two demos are named $demo_name" >&2
          exit 1
        fi
        cp "$demo/demo.cast" "$out/$demo_name.cast"
        cast-to-svg "$out/$demo_name.cast" "$out/$demo_name.svg"
        cat "$demo/demo.json"
      done > demos
      jq -s . demos > $out/demos.json
    '';
}
