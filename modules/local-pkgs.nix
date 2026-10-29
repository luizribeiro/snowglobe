{ config, lib, pkgs, ... }:

let
  hostSystem = pkgs.stdenv.hostPlatform.system;
  localSystem = config.snowglobe.localSystem;
in
{
  options.snowglobe.localSystem = lib.mkOption {
    type = lib.types.str;
    default = hostSystem;
    defaultText = lib.literalExpression "pkgs.stdenv.hostPlatform.system";
    example = "aarch64-darwin";
    description = ''
      Platform of the machine running `snow push`, which is where local
      hooks run. snow sets this when it reads hooks; you normally leave it
      alone. Hooks get a matching package set as the `localPkgs` module
      argument: the host's nixpkgs, instantiated for this platform with the
      host's `nixpkgs.config` and `nixpkgs.overlays`. Overlays baked into an
      externally supplied `nixpkgs.pkgs` aren't carried over.
    '';
  };

  config._module.args.localPkgs =
    if localSystem == hostSystem then
      pkgs
    else
      import pkgs.path {
        localSystem = { system = localSystem; };
        config = pkgs.config;
        overlays = config.nixpkgs.overlays or [ ];
      };
}
