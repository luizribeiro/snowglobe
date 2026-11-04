{ pkgs, snow, snowglobeModule }:

let
  inherit (pkgs) lib;

  targets = [ "boron" "cerium" "radon" ];

  # Systems to push, each a target with something added. They're in each
  # target's own closure already, so a push copies little and needs no network.
  variants = {
    v1.environment.etc.pushed.text = "v1";
    v2.environment.etc.pushed.text = "v2";
    v3.environment.etc.pushed.text = "v3";
    initrd = {
      environment.etc.pushed.text = "initrd";
      boot.initrd.availableKernelModules = [ "nbd" ];
    };
    broken = {
      environment.etc.pushed.text = "broken";
      systemd.services.storage-agent = {
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${pkgs.coreutils}/bin/false";
        };
      };
    };
  };

  scenarios = import ./scenarios.nix;
  scenariosByName = lib.listToAttrs (map (s: lib.nameValuePair s.name s) scenarios);

  # A flake per scenario with the metadata snow reads from a NixOS host, and
  # each host's system pointing at a prebuilt variant. Real NixOS configs
  # can't be evaluated in the client: it has no nixpkgs to evaluate them with.
  scenarioFlake = nodes: name: scenario:
    let
      hooksFile = host: pkgs.writeText "${name}-${host}-hooks.json"
        (builtins.toJSON (scenario.hosts.${host}.hooks or { }));
      hostConfig = host: settings: ''
        ${host}.config = {
          networking = { hostName = "${host}"; domain = ""; };
          core = {
            pushPhase = ${if settings ? phase then ''"${settings.phase}"'' else "null"};
            tags = [ ];
            rebootPolicy = "${settings.rebootPolicy or "auto"}";
          };
          snowglobe.push.serialGroup = ${if settings ? serialGroup then ''"${settings.serialGroup}"'' else "null"};
          snowglobe.hooksFile = copied "${hooksFile host}";
          system.build.toplevel = builtins.storePath
            "${nodes.${host}.specialisation.${settings.variant}.configuration.system.build.toplevel}";
        };
      '';
    in
    pkgs.writeTextDir "flake.nix" ''
      {
        outputs = _:
          let
            # A derivation, as the snowglobe module's hooks file is: snow
            # builds it to fetch what local hooks need
            copied = file: derivation {
              name = "snowglobe-hooks.json";
              system = "${pkgs.stdenv.hostPlatform.system}";
              builder = "''${builtins.storePath "${pkgs.bash}"}/bin/bash";
              args = [ "-c" "printf %s \"$(< ''${builtins.storePath file})\" > $out" ];
            };
          in
          {
            nixosConfigurations = {
              ${lib.concatStrings (lib.mapAttrsToList hostConfig scenario.hosts)}
            };
          };
      }
    '';

  # snow evaluates and builds the flake purely; the scenario flakes need
  # builtins.storePath to point at the prebuilt variants
  impureNix = pkgs.writeShellScriptBin "nix" ''
    case "$1" in
      eval|build) exec ${pkgs.nix}/bin/nix "$1" --impure "''${@:2}" ;;
      *) exec ${pkgs.nix}/bin/nix "$@" ;;
    esac
  '';

  recorder = (import ../demos).recorder { inherit pkgs; };

  snowRunner = {
    nix.settings.experimental-features = [ "nix-command" "flakes" ];
    environment.systemPackages = [ (lib.hiPrio impureNix) snow recorder ];
  };
in
pkgs.testers.runNixOSTest {
  name = "snow-push";

  defaults = {
    virtualisation.memorySize = 1024;
  };

  nodes = lib.genAttrs targets (name: { config, lib, ... }: {
    imports = [ snowglobeModule ] ++ lib.optional (name == "boron") snowRunner;
    services.openssh.enable = true;
    specialisation = lib.mapAttrs (_: configuration: { inherit configuration; }) variants;
    # Test VMs boot without a system profile, so snow could not read what
    # they boot into and would warn on every push. Record the booted system
    # as its first generation, as installing NixOS does.
    systemd.services.install-system-profile = {
      wantedBy = [ "multi-user.target" ];
      before = [ "sshd.service" ];
      unitConfig.ConditionPathExists = "!/nix/var/nix/profiles/system";
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = ''
        ${config.nix.package}/bin/nix-env --profile /nix/var/nix/profiles/system \
          --set "$(readlink -f /run/booted-system)"
      '';
    };
  }) // {
    # Holds every scenario's flake, even those run on boron: one in boron's own
    # /etc would point at a boron system that contains it
    client = { nodes, ... }: {
      imports = [ snowRunner ];
      environment.systemPackages = [ pkgs.socat ];
      environment.etc = lib.mapAttrs'
        (name: scenario: lib.nameValuePair "scenarios/${name}" {
          source = scenarioFlake nodes name scenario;
        })
        scenariosByName;
      programs.ssh.extraConfig = ''
        StrictHostKeyChecking no
        UserKnownHostsFile /dev/null
        LogLevel ERROR
      '';
    };
  };

  testScript = { nodes, ... }: ''
    variants = ${builtins.toJSON (lib.genAttrs targets (host:
      lib.mapAttrs (variant: _: "${nodes.${host}.specialisation.${variant}.configuration.system.build.toplevel}") variants))}
    scenarios = ${builtins.toJSON (map (s: removeAttrs s [ "hosts" ]) scenarios)}
  '' + builtins.readFile ./push.py;
}
