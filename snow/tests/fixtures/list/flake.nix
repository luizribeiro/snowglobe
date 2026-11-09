{
  description = "Hosts with every push setting, for snow list";

  outputs = { self, ... }: {
    nixosConfigurations = {
      argon.config = {
        networking.hostName = "argon";
        nixpkgs.hostPlatform.system = "x86_64-linux";
        snowglobe.deploy = {
          phase = "p1";
          tags = [ "web" "dns" ];
          rebootPolicy = "never";
          serialGroup = "dns";
        };
      };

      boron.config = {
        networking.hostName = "boron";
        nixpkgs.hostPlatform.system = "aarch64-linux";
        snowglobe.deploy = {
          phase = "p2";
          tags = [ "web" ];
          optional = true;
        };
        # Unbuildable on purpose: list and build need no hooks, so must not build them
        snowglobe.hooksFile.drvPath = "/nix/store/00000000000000000000000000000000-snowglobe-hooks.json.drv";
      };

      curium.config = {
        networking.hostName = "curium";
        nixpkgs.hostPlatform.system = "x86_64-linux";
        snowglobe.deploy = {
          phase = "never";
          tags = [ "web" ];
        };
      };
    };

    darwinConfigurations = {
      ferrum.config = {
        networking.hostName = "ferrum";
        nixpkgs.hostPlatform.system = "aarch64-darwin";
      };
    };
  };
}
