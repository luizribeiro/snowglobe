# snowTest: a NixOS VM test of snow pushing to hosts, from a test file in
# ./tests. The hosts boot a base system shared by every test; the test's
# `nodes` are what snow pushes to them, from the deployer `sodium`. A test's
# optional `common` module is booted by every VM, the deployer's included,
# and its optional `base.<host>` module goes on that host's base system;
# both are booted and pushed alike.
{ pkgs, snow, snowglobeModule }:

let
  inherit (pkgs) lib;

  deployer = "sodium";
  demos = import ../demos;
  sshKeys = import "${pkgs.path}/nixos/tests/ssh-keys.nix" pkgs;

  # snow evaluates and builds the flake purely; the ops flake needs
  # builtins.storePath to point at the prebuilt systems. Being prebuilt,
  # they build at once, so a demo that needs a long build sets
  # SNOW_E2E_BUILD_SECONDS.
  impureNix = pkgs.writeShellScriptBin "nix" ''
    case "$1" in
      build)
        sleep "''${SNOW_E2E_BUILD_SECONDS:-0}"
        exec ${pkgs.nix}/bin/nix build --impure "''${@:2}" ;;
      eval) exec ${pkgs.nix}/bin/nix eval --impure "''${@:2}" ;;
      *) exec ${pkgs.nix}/bin/nix "$@" ;;
    esac
  '';

  base = { config, ... }: {
    imports = [ snowglobeModule ];
    virtualisation.memorySize = 1024;
    # Test VMs leave it out, but snow activates pushed systems with it
    system.switch.enable = true;
    # Test VMs boot their kernel directly, so there is no boot loader to
    # install, and the demos shouldn't warn about it
    boot.loader.grub.enable = false;
    system.build.installBootLoader = "${pkgs.coreutils}/bin/true";
    nix.settings.experimental-features = [ "nix-command" "flakes" ];
    environment.systemPackages = [
      (lib.hiPrio impureNix)
      snow
      (demos.recorder { inherit pkgs; })
      pkgs.socat
    ];
    services.openssh.enable = true;
    users.users.root.openssh.authorizedKeys.keys = [ sshKeys.snakeOilPublicKey ];
    environment.etc."ssh/id_snakeoil" = {
      source = sshKeys.snakeOilPrivateKey;
      mode = "0600";
    };
    programs.ssh.extraConfig = ''
      IdentityFile /etc/ssh/id_snakeoil
      StrictHostKeyChecking no
      UserKnownHostsFile /dev/null
      LogLevel ERROR
    '';
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
  };

  hostsJson = pushed: pkgs.writeText "hosts.json" (builtins.toJSON {
    inherit (pkgs.stdenv.hostPlatform) system;
    bash = "${pkgs.bash}";
    hosts = lib.mapAttrs
      (_: config: {
        inherit (config.networking) hostName domain;
        inherit (config.snowglobe.push) phase tags rebootPolicy optional serialGroup sshPort;
        hooksFile = "${config.snowglobe.hooksFile}";
        toplevel = "${config.system.build.toplevel}";
      })
      pushed;
  });

  opsFlake = pushed: pkgs.runCommand "ops" { } ''
    mkdir $out
    cp ${./ops-flake.nix} $out/flake.nix
    cp ${hostsJson pushed} $out/hosts.json
  '';

  toplevels = lib.mapAttrs (_: config: "${config.system.build.toplevel}");
in
test:
let
  hosts = builtins.attrNames test.nodes;
  demo = if test ? demo then { runOn = deployer; } // test.demo else null;
  runsDemo = name: demo != null && name == demo.runOn;

  # The host's base system with the test's module on top, so it boots in the
  # same VM; marked, so there's always something to push
  pushedModule = name: { extendModules, ... }: {
    system.build.pushed = (extendModules {
      modules = [
        test.nodes.${name}
        { environment.etc.snowglobe-pushed.text = test.name; }
      ];
    }).config;
  };
in
assert lib.assertMsg (test ? demo || test ? testScript)
  "${test.name}: a test needs a demo, a testScript or both";
assert lib.assertMsg (!(test.nodes ? ${deployer}))
  "${test.name}: ${deployer} is the deployer, not a host to push to";
assert lib.assertMsg (demo == null || builtins.elem demo.runOn (hosts ++ [ deployer ]))
  "${test.name}: the demo runs on ${demo.runOn}, which has no VM";
pkgs.testers.runNixOSTest ({ config, ... }:
  let
    pushed = lib.genAttrs hosts (name: config.nodes.${name}.system.build.pushed);
    flake = opsFlake pushed;
    demoScript = pkgs.writeText "${test.name}-demo.py" demo.script;

    node = name: {
      imports = [ base (test.common or { }) (test.base.${name} or { }) ]
        ++ lib.optional (builtins.elem name hosts) (pushedModule name);
      # Registered in the VM's store without being part of its system, which
      # stays the base
      virtualisation.additionalPaths =
        lib.optional (name == deployer || runsDemo name) flake
        ++ lib.optional (runsDemo name) demoScript;
    };

    settings = pkgs.writeText "${test.name}-harness.json" (builtins.toJSON {
      inherit deployer hosts flake;
      base = toplevels (lib.getAttrs hosts config.nodes);
      sshPorts = lib.mapAttrs (_: node: node.snowglobe.push.sshPort) (lib.getAttrs hosts config.nodes);
      pushed = toplevels pushed;
      demo = if demo == null then null else {
        inherit (test) name;
        inherit (demo) title runOn;
        user = demo.user or null;
        script = demoScript;
      };
    });
  in
  {
    inherit (test) name;
    nodes = lib.genAttrs (hosts ++ [ deployer ]) node;
    testScript = ''
      HARNESS = "${settings}"
    '' + builtins.readFile ./harness.py + (test.testScript or "demo()\n");
  })
