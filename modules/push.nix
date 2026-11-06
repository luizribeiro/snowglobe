{ lib, ... }:

# Platform-agnostic push options, imported by both nixos.nix and darwin.nix

{
  options.snowglobe.push.serialGroup = lib.mkOption {
    type = lib.types.nullOr (lib.types.strMatching ".+");
    default = null;
    description = ''
      Hosts that share a serial group deploy one at a time within a push
      phase. Only the disruptive part is serialized: pre-deploy hooks,
      activation, any reboot, post-reboot hooks and post-deploy hooks.
      Build, copy and planning stay parallel, and hosts with nothing to
      deploy never wait.

      If a host in the group fails, its on-failure hooks run before the
      next host may start, and group members that haven't started are
      skipped.

      Only hosts pushed by the same `snow push` invocation coordinate.
    '';
    example = "kubernetes";
  };

  options.snowglobe.push.sshPort = lib.mkOption {
    type = lib.types.nullOr lib.types.port;
    default = null;
    description = ''
      The port snow connects to over ssh to push to this host: for
      `nix copy`, activation, reboots and remote hooks. When null, ssh
      picks the port, from the deployer's ssh config or the default 22.
    '';
    example = 2222;
  };
}
