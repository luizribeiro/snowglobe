# Platform-agnostic bootstrap configuration module
# This module is imported by both nixos.nix and darwin.nix
#
# Example configuration for a Raspberry Pi host:
#   snowglobe.bootstrap = {
#     provisioner = "sd-image";
#     imageAttr = "config.system.build.sdImage";
#   };
{ config, lib, ... }:

with lib;

let
  cfg = config.snowglobe.bootstrap;

  unstableNote = "\n" + ''
    Only `snow bootstrap` reads this option. The command is unstable and
    left out of release builds, so the option may change or go away.
  '';
in
{
  options.snowglobe.bootstrap = {
    provisioner = mkOption {
      type = types.nullOr (types.enum [ "manual" "sd-image" "raw" "proxmox-vma" ]);
      default = null;
      description = ''
        Provisioner to use for key injection during bootstrap.

        Available provisioners:
        - `manual`: Output instructions for manual key injection
        - `sd-image`: Inject keys into SD card images (Raspberry Pi, etc.)
        - `raw`: Inject keys into raw disk images
        - `proxmox-vma`: Inject keys into Proxmox VMA backup images

        When null, defaults to "manual".
      '' + unstableNote;
      example = "sd-image";
    };

    imageAttr = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = ''
        Nix attribute path for building the bootable image.

        This should be a path relative to the host's configuration that
        builds a bootable image (e.g., SD card image, raw disk image).

        When set, `snow bootstrap` generates the host's SSH key, updates
        .sops.yaml with `--update-sops`, then builds this image and injects
        the key into it. The key comes first so the image's secrets can be
        encrypted for the new host; if the build fails, the key is left
        behind.
      '' + unstableNote;
      example = "config.system.build.sdImage";
    };

    sshKeyPath = mkOption {
      type = types.str;
      default = "/etc/ssh";
      description = ''
        Path where SSH host keys should be injected in the image.

        This is the directory inside the image where the generated
        ed25519 host key pair will be placed.
      '' + unstableNote;
      example = "/run/secrets/ssh";
    };
  };

  config.assertions = [
    {
      assertion =
        cfg.provisioner == null ||
        cfg.provisioner == "manual" ||
        cfg.imageAttr != null;
      message = ''
        When using provisioner "${toString cfg.provisioner}", you must set
        snowglobe.bootstrap.imageAttr to specify which image to build.

        Example:
          snowglobe.bootstrap.imageAttr = "config.system.build.sdImage";
      '';
    }
  ];
}
