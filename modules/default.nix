{ pkgs, ... }:

# Platform-aware module that imports the appropriate implementation
# for NixOS or nix-darwin based on the current system

if pkgs.stdenv.isDarwin then
  import ./darwin.nix
else
  import ./nixos.nix
