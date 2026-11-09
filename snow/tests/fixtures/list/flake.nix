{
  description = "Hosts with every deploy setting, for snow list";

  outputs = { self, ... }: {
    nixosConfigurations = {
      alpha.config = {
        networking.hostName = "alpha";
        nixpkgs.hostPlatform.system = "x86_64-linux";
        snowglobe.deploy = {
          phase = "p1";
          tags = [ "web" "dns" ];
          rebootPolicy = "never";
          serialGroup = "dns";
        };
      };

      bravo.config = {
        networking.hostName = "bravo";
        nixpkgs.hostPlatform.system = "aarch64-linux";
        snowglobe.deploy = {
          phase = "p2";
          tags = [ "web" ];
          optional = true;
        };
        # Unbuildable on purpose: list and build need no hooks, so must not build them
        snowglobe.hooksFile.drvPath = "/nix/store/00000000000000000000000000000000-snowglobe-hooks.json.drv";
      };

      condor.config = {
        networking.hostName = "condor";
        nixpkgs.hostPlatform.system = "x86_64-linux";
        snowglobe.deploy = {
          phase = "never";
          tags = [ "web" ];
        };
      };
    };

    darwinConfigurations = {
      falcon.config = {
        networking.hostName = "falcon";
        nixpkgs.hostPlatform.system = "aarch64-darwin";
      };
    };
  };
}
