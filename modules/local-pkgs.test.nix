{ pkgs }:

let
  evalLocal = modules: pkgs.lib.evalModules {
    modules = [
      ./local-pkgs.nix
      { _module.args.pkgs = pkgs; }
    ] ++ modules;
  };

  hostSystem = pkgs.stdenv.hostPlatform.system;

  # nixpkgs.overlays is normally declared by NixOS/nix-darwin
  markerOverlay = {
    options.nixpkgs.overlays = pkgs.lib.mkOption { type = pkgs.lib.types.listOf pkgs.lib.types.unspecified; default = [ ]; };
    config.nixpkgs.overlays = [ (final: prev: { snowglobeMarker = "from-overlay"; }) ];
  };
  otherSystem = if hostSystem == "aarch64-linux" then "x86_64-linux" else "aarch64-linux";
in
{
  testLocalSystemDefaultsToHostPlatform = {
    expr = (evalLocal [ ]).config.snowglobe.localSystem;
    expected = hostSystem;
  };

  testLocalPkgsReusesHostPkgsOnSamePlatform = {
    # The overlay isn't in `pkgs`, so it only shows up if nixpkgs was re-imported
    expr = (evalLocal [ markerOverlay ])._module.args.localPkgs ? snowglobeMarker;
    expected = false;
  };

  testLocalPkgsFollowsLocalSystem = {
    expr = (evalLocal [ { snowglobe.localSystem = otherSystem; } ])._module.args.localPkgs.stdenv.hostPlatform.system;
    expected = otherSystem;
  };

  testLocalPkgsForOtherPlatformKeepsHostOverlays = {
    expr = (evalLocal [ markerOverlay { snowglobe.localSystem = otherSystem; } ])._module.args.localPkgs.snowglobeMarker;
    expected = "from-overlay";
  };
}
