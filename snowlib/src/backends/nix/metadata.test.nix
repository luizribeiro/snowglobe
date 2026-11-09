{ pkgs, snowglobeModule }:

let
  metadata = import ./metadata.nix;

  hostSystem = pkgs.stdenv.hostPlatform.system;
  otherSystem = if hostSystem == "aarch64-linux" then "x86_64-linux" else "aarch64-linux";

  nixosHost = modules: import (pkgs.path + "/nixos/lib/eval-config.nix") {
    system = null;
    modules = [ { nixpkgs.hostPlatform = hostSystem; } ] ++ modules;
  };

  web1 = nixosHost [
    snowglobeModule
    {
      networking = { hostName = "web1"; domain = "example.com"; };
      snowglobe.deploy = {
        phase = "p1";
        tags = [ "web" ];
        rebootPolicy = "never";
        serialGroup = "web";
        sshPort = 2222;
        optional = true;
        ignoredFailedUnits = [ "storage-agent.service" ];
      };
    }
  ];

  withoutModule = nixosHost [ { networking.hostName = "plain"; } ];

  hooksFileFor = system: (web1.extendModules {
    modules = [ { snowglobe.localSystem = pkgs.lib.mkForce system; } ];
  }).config.snowglobe.hooksFile.drvPath;

  withoutHooksFile = meta: removeAttrs meta [ "hooksFile" ];
in
{
  testReadsTheDeploySettings = {
    expr = withoutHooksFile (metadata hostSystem web1);
    expected = {
      hostName = "web1";
      domain = "example.com";
      system = hostSystem;
      phase = "p1";
      tags = [ "web" ];
      rebootPolicy = "never";
      serialGroup = "web";
      sshPort = 2222;
      optional = true;
      ignoredFailedUnits = [ "storage-agent.service" ];
    };
  };

  testDefaultsHostsWithoutTheModule = {
    expr = metadata otherSystem withoutModule;
    expected = {
      hostName = "plain";
      domain = null;
      system = hostSystem;
      phase = null;
      tags = [ ];
      rebootPolicy = "auto";
      serialGroup = null;
      sshPort = null;
      optional = false;
      ignoredFailedUnits = [ ];
      hooksFile = null;
    };
  };

  testBuildsHooksForThisMachine = {
    expr = (metadata otherSystem web1).hooksFile == hooksFileFor otherSystem;
    expected = true;
  };

  testReusesTheHooksFileOnTheHostsPlatform = {
    expr = (metadata hostSystem web1).hooksFile == hooksFileFor hostSystem;
    expected = true;
  };

  # Not every flake builds its hosts with nixosSystem
  testReadsAPlainAttrsetHost = {
    expr = withoutHooksFile (metadata otherSystem {
      config = {
        networking.hostName = "web2";
        nixpkgs.hostPlatform.system = "aarch64-linux";
        snowglobe.deploy.phase = "p2";
      };
    });
    expected = {
      hostName = "web2";
      domain = "";
      system = "aarch64-linux";
      phase = "p2";
      tags = [ ];
      rebootPolicy = "auto";
      serialGroup = null;
      sshPort = null;
      optional = false;
      ignoredFailedUnits = [ ];
    };
  };
}
