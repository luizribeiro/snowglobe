{ pkgs, snow, snowglobeModule }:

let
  inherit (pkgs) lib;
  snowTest = import ./harness.nix { inherit pkgs snow snowglobeModule; };

  # In the order the demos page shows them
  names = [
    "switch"
    "rebuild-cancel"
  ];

  files = lib.genAttrs names (name: import ./tests/${name}.nix);
  onDisk = map (lib.removeSuffix ".nix")
    (lib.filter (lib.hasSuffix ".nix") (builtins.attrNames (builtins.readDir ./tests)));
  sorted = lib.sort lib.lessThan;
in
assert lib.assertMsg (sorted names == sorted onDisk)
  "nix/e2e: the tests listed in default.nix differ from those in ./tests";
assert lib.all (name: lib.assertMsg (files.${name}.name == name)
  "nix/e2e/tests/${name}.nix: its name should match its file") names;
lib.mapAttrs (_: snowTest) files
