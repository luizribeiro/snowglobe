{ pkgs, snow, snowglobeModule }:

let
  inherit (pkgs) lib;
  snowTest = import ./harness.nix { inherit pkgs snow snowglobeModule; };

  # In the order the demos page shows them
  names = [
    "tutorial"
    "switch"
    "reboot"
    "deploy-boot"
    "activation-failure"
    "hook-failure"
    "health-gate"
    "missing-hook"
    "unreachable"
    "hung-agent"
    "serial-group"
    "reboot-deadline"
    "rebuild-cancel"
    "rebuild-boot"
    "rebuild-reboot"
    "ssh-port"
    "deploy-user"
    "rebuild-sudo"
    "rebuild-sudo-keepalive"
    "never-phase"
    "optional-host"
    "preexisting-failure"
    "reboot-auto"
    "switch-refuses"
  ];

  files = lib.genAttrs names (name: import ./tests/${name}.nix);
  onDisk = map (lib.removeSuffix ".nix")
    (lib.filter (lib.hasSuffix ".nix") (builtins.attrNames (builtins.readDir ./tests)));
  sorted = lib.sort lib.lessThan;
  tests = lib.mapAttrs (_: snowTest) files;
in
assert lib.assertMsg (sorted names == sorted onDisk)
  "nix/e2e: the tests listed in default.nix differ from those in ./tests";
assert lib.all (name: lib.assertMsg (files.${name}.name == name)
  "nix/e2e/tests/${name}.nix: its name should match its file") names;
{
  inherit tests;
  recordings = (import ../demos).renderDemos {
    inherit pkgs;
    name = "snow-deploy-recordings";
    demos = lib.concatMap
      (name: map (demo: "${tests.${name}}/demos/${demo}") tests.${name}.demos)
      names;
  };
}
