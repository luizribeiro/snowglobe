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

  castToSvg = { pkgs, look ? terminalLook }: withLook {
    inherit pkgs look;
    name = "cast-to-svg";
    script = ./cast_to_svg.py;
  };
}
