{ lib, ... }:

# Platform-agnostic push options, imported by both nixos.nix and darwin.nix

let
  # The characters snow's --phase and --tag filters keep
  filterName = lib.types.strMatching "[A-Za-z0-9_-]+";

  # "default" is the phase snow puts hosts without one in
  phaseName = lib.types.addCheck filterName (name: name != "default")
    // { description = ''phase name (letters, digits, "-" and "_"), other than "default"''; };

  tagName = filterName // { description = ''tag (letters, digits, "-" and "_")''; };
in
{
  options.snowglobe.push.phase = lib.mkOption {
    type = lib.types.nullOr phaseName;
    default = null;
    description = ''
      The push phase this host deploys in. Phases deploy one after another,
      in name order with numbers by value (p2 before p10), and hosts
      without a phase deploy last. If a host in a phase fails, later phases
      are skipped.

      Hosts in the phase `never` are pushed (or built) only when named, as
      in `snow push curium`, or selected with `--phase never`.
    '';
    example = "p1";
  };

  options.snowglobe.push.tags = lib.mkOption {
    type = lib.types.listOf tagName;
    default = [ ];
    description = ''
      Tags that select this host with `snow push --tag` or `--tags`.
    '';
    example = [ "web" "database" ];
  };

  options.snowglobe.push.rebootPolicy = lib.mkOption {
    type = lib.types.enum [ "auto" "never" "always" ];
    default = "auto";
    description = ''
      When `snow push` reboots this host: `auto` when the new system needs a
      reboot (a new kernel, initrd or critical service), `always` after
      every deploy, and `never` not at all, setting the new system for the
      next boot instead when it needs one. snow never reboots nix-darwin
      hosts, so it has no effect on them.
    '';
  };

  options.snowglobe.push.optional = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Whether `snow push` may go on without this host. An optional host that
      fails, for example because it is offline, is reported as failed, but
      its phase doesn't fail: later phases still run, and the push succeeds
      if only optional hosts failed.
    '';
  };

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
