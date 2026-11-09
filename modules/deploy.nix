{ lib, ... }:

# Platform-agnostic push options, imported by both nixos.nix and darwin.nix

let
  # The characters snow's --phase and --tag filters keep
  filterName = lib.types.strMatching "[A-Za-z0-9_-]+";

  # "default" is the phase snow puts hosts without one in
  phaseName = lib.types.addCheck filterName (name: name != "default")
    // { description = ''phase name (letters, digits, "-" and "_"), other than "default"''; };

  tagName = filterName // { description = ''tag (letters, digits, "-" and "_")''; };

  unitName = lib.types.strMatching
    "[^/[:space:]]+[.](service|socket|target|device|mount|automount|swap|path|timer|slice|scope)"
    // { description = ''systemd unit name with its type, as in "unifi.service"''; };
in
{
  options.snowglobe.deploy.phase = lib.mkOption {
    type = lib.types.nullOr phaseName;
    default = null;
    description = ''
      The push phase this host deploys in. Phases deploy one after another,
      in name order with numbers by value (p2 before p10), and hosts
      without a phase deploy last. If a host in a phase fails, later phases
      are skipped.

      Hosts in the phase `never` are pushed (or built) only when named, as
      in `snow deploy web1`, or selected with `--phase never`.
    '';
    example = "p1";
  };

  options.snowglobe.deploy.tags = lib.mkOption {
    type = lib.types.listOf tagName;
    default = [ ];
    description = ''
      Tags that select this host with `snow deploy --tag` or `--tags`.
    '';
    example = [ "web" "database" ];
  };

  options.snowglobe.deploy.rebootPolicy = lib.mkOption {
    type = lib.types.enum [ "auto" "never" "always" ];
    default = "auto";
    description = ''
      When `snow deploy` reboots this host: `auto` when the new system needs a
      reboot (a new kernel, initrd or critical service), `always` after
      every deploy, and `never` not at all, setting the new system for the
      next boot instead when it needs one. snow never reboots nix-darwin
      hosts, so it has no effect on them.
    '';
  };

  options.snowglobe.deploy.optional = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Whether `snow deploy` may go on without this host. An optional host that
      fails, for example because it is offline, is reported as failed, but
      its phase doesn't fail: later phases still run, and the push succeeds
      if only optional hosts failed.
    '';
  };

  options.snowglobe.deploy.serialGroup = lib.mkOption {
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

      Only hosts pushed by the same `snow deploy` invocation coordinate.
    '';
    example = "kubernetes";
  };

  options.snowglobe.deploy.ignoredFailedUnits = lib.mkOption {
    type = lib.types.listOf unitName;
    default = [ ];
    description = ''
      Units that may be failing after `snow deploy` activates this host
      without failing it.

      A NixOS switch fails when any unit is failing afterwards, even one
      the push didn't touch: the switch starts its targets again, which
      reruns units that were already failing, such as a crash-looping
      service. snow then fails the host at activate, saying that the new
      system is active. If the only failing units are listed here, the
      host is deployed instead, and the report names the units.

      The host still fails if any other unit is failing, or if the switch
      fails for another reason, such as a unit that failed to reload.
      Listed units are ignored even if this push broke them. nix-darwin
      hosts aren't affected.
    '';
    example = [ "unifi.service" ];
  };

  options.snowglobe.deploy.sshPort = lib.mkOption {
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
