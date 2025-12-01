{ pkgs, fetchFromGitHub, fetchurl, ... }:

(pkgs.qemu_kvm.override {
  alsaSupport = false;
  pulseSupport = false;
  sdlSupport = false;
  jackSupport = false;
  gtkSupport = false;
  vncSupport = false;
  smartcardSupport = false;
  spiceSupport = false;
  ncursesSupport = false;
  libiscsiSupport = false;
  tpmSupport = false;
  numaSupport = false;
  seccompSupport = false;
  guestAgentSupport = false;
}).overrideAttrs (super: rec {
  pname = "vma";
  version = "9.0.0";

  src = fetchurl {
    url = "https://download.qemu.org/qemu-${version}.tar.xz";
    hash = "sha256-MnCKxmww2MiSYz6paMdxwcdtWX1w3erSGg0izPOG2mk=";
  };

  patches = [
    "${fetchFromGitHub {
      owner = "proxmox";
      repo = "pve-qemu";
      rev = "14afbdd55f04d250bd679ca1ad55d3f47cd9d4c8";
      hash = "sha256-lSJQA5SHIHfxJvMLIID2drv2H43crTPMNIlIT37w9Nc=";
    }}/debian/patches/pve/0027-PVE-Backup-add-vma-backup-format-code.patch"
  ];

  buildInputs = super.buildInputs ++ [ pkgs.libuuid ];
  nativeBuildInputs = super.nativeBuildInputs ++ [ pkgs.perl ];

  meta = super.meta // {
    description = "Proxmox VMA backup format tool (patched QEMU)";
    platforms = [ "x86_64-linux" ];
  };
})
