# snowTest: a NixOS VM test of snow deploying to hosts, from a test file in
# ./tests. The hosts boot a base system shared by every test; the test's
# `nodes` are what snow deploys to them, from the deployer `laptop`. A test's
# optional `common` module is booted by every VM, the deployer's included,
# and its optional `base.<host>` module goes on that host's base system;
# both are booted and deployed alike.
{ pkgs, snow, snowglobeModule }:

let
  inherit (pkgs) lib;

  deployer = "laptop";
  demos = import ../demos;
  sshKeys = import "${pkgs.path}/nixos/tests/ssh-keys.nix" pkgs;

  # snow evaluates and builds the flake purely; the ops flake needs
  # builtins.storePath to point at the prebuilt systems. Being prebuilt,
  # they build at once, so a demo that needs a long system build sets
  # SNOW_E2E_BUILD_SECONDS.
  impureNix = pkgs.writeShellScriptBin "nix" ''
    case "$1" in
      build)
        # Hooks files build while snow reads the hosts' settings, which a
        # long build shouldn't slow
        case "$*" in
          *.drv^out*) ;;
          *) sleep "''${SNOW_E2E_BUILD_SECONDS:-0}" ;;
        esac
        exec ${pkgs.nix}/bin/nix build --impure "''${@:2}" ;;
      eval) exec ${pkgs.nix}/bin/nix eval --impure "''${@:2}" ;;
      *) exec ${pkgs.nix}/bin/nix "$@" ;;
    esac
  '';

  base = { config, ... }: {
    imports = [ snowglobeModule ];
    virtualisation.memorySize = 1024;
    # Test VMs leave it out, but snow activates deployed systems with it
    system.switch.enable = true;
    # Test VMs boot their kernel directly, so there is no boot loader to
    # install, and the demos shouldn't warn about it
    boot.loader.grub.enable = false;
    system.build.installBootLoader = "${pkgs.coreutils}/bin/true";
    nix.settings.experimental-features = [ "nix-command" "flakes" ];
    # The VMs are offline, and each build would spend about 5s retrying
    # cache.nixos.org before building
    nix.settings.substituters = lib.mkForce [ ];
    environment.systemPackages = [
      (lib.hiPrio impureNix)
      snow
      (demos.recorder { inherit pkgs; })
      pkgs.socat
    ];
    services.openssh.enable = true;
    users.users.root.openssh.authorizedKeys.keys = [ sshKeys.snakeOilPublicKey ];
    # Waiting for a host's ssh logs in as root, and root's user manager
    # stops 10s after that session ends. A switch during that stop fails its
    # user activation (exit 4), so keep the manager running.
    users.users.root.linger = true;
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
    # they boot into and would warn on every deployment. Record the booted system
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

  hostsJson = deployed: pkgs.writeText "hosts.json" (builtins.toJSON {
    inherit (pkgs.stdenv.hostPlatform) system;
    bash = "${pkgs.bash}";
    hosts = lib.mapAttrs
      (_: config: {
        inherit (config.networking) hostName domain;
        inherit (config.nixpkgs.hostPlatform) system;
        inherit (config.snowglobe.deploy) phase tags rebootPolicy optional serialGroup sshPort
          ignoredFailedUnits;
        hooksFile = "${config.snowglobe.hooksFile}";
        toplevel = "${config.system.build.toplevel}";
      })
      deployed;
  });

  opsFlake = deployed: pkgs.runCommand "ops" { } ''
    mkdir $out
    cp ${./ops-flake.nix} $out/flake.nix
    cp ${hostsJson deployed} $out/hosts.json
  '';

  toplevels = lib.mapAttrs (_: config: "${config.system.build.toplevel}");
in
test:
let
  hosts = builtins.attrNames test.nodes;
  # A test records one `demo`, named after the test, or a list of `demos`,
  # each with its own name
  demos = map (demo: { runOn = deployer; user = null; } // demo)
    (test.demos or (lib.optional (test ? demo) ({ inherit (test) name; } // test.demo)));
  runsDemo = name: lib.any (demo: demo.runOn == name) demos;

  # The host's base system with the test's module on top, so it boots in the
  # same VM; marked, so there's always something to deploy
  deployedModule = name: { extendModules, ... }: {
    system.build.deployed = (extendModules {
      modules = [
        test.nodes.${name}
        { environment.etc.snowglobe-deployed.text = test.name; }
      ];
    }).config;
  };
in
assert lib.assertMsg (!(test ? demo && test ? demos))
  "${test.name}: a test has a demo or demos, not both";
assert lib.assertMsg (lib.allUnique (map (demo: demo.name) demos))
  "${test.name}: two demos share a name";
assert lib.all (demo: lib.assertMsg (builtins.match "[a-z0-9-]+" demo.name != null)
  "${test.name}: the demo name ${demo.name} names a unit and files, so it takes only a-z, 0-9 and -") demos;
assert lib.assertMsg (demos != [ ] || test ? testScript)
  "${test.name}: a test needs a demo, a testScript or both";
assert lib.assertMsg (demos == [ ] || builtins.length demos == 1 || test ? testScript)
  "${test.name}: with several demos, the testScript says when each one runs";
assert lib.assertMsg (!(test.nodes ? ${deployer}))
  "${test.name}: ${deployer} is the deployer, not a host to deploy to";
assert lib.all (demo: lib.assertMsg (builtins.elem demo.runOn (hosts ++ [ deployer ]))
  "${test.name}: the demo ${demo.name} runs on ${demo.runOn}, which has no VM") demos;
pkgs.testers.runNixOSTest ({ config, ... }:
  let
    deployed = lib.genAttrs hosts (name: config.nodes.${name}.system.build.deployed);
    flake = opsFlake deployed;
    demoScript = demo: pkgs.writeText "${demo.name}-demo.py" demo.script;

    node = name: {
      imports = [ base (test.common or { }) (test.base.${name} or { }) ]
        ++ lib.optional (builtins.elem name hosts) (deployedModule name);
      # Registered in the VM's store without being part of its system, which
      # stays the base
      virtualisation.additionalPaths =
        lib.optional (name == deployer || runsDemo name) flake
        ++ map demoScript (lib.filter (demo: demo.runOn == name) demos);
    };

    settings = pkgs.writeText "${test.name}-harness.json" (builtins.toJSON {
      inherit deployer hosts flake;
      base = toplevels (lib.getAttrs hosts config.nodes);
      sshPorts = lib.mapAttrs (_: node: node.snowglobe.deploy.sshPort) (lib.getAttrs hosts config.nodes);
      deployed = toplevels deployed;
      demos = map (demo: {
        inherit (demo) name title runOn user;
        script = demoScript demo;
      }) demos;
    });
  in
  {
    inherit (test) name;
    passthru.demos = map (demo: demo.name) demos;
    nodes = lib.genAttrs (hosts ++ [ deployer ]) node;
    testScript = ''
      HARNESS = "${settings}"
    '' + builtins.readFile ./harness.py + (test.testScript or "demo()\n");
  })
