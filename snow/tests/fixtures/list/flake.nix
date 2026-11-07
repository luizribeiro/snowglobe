{
  description = "Hosts with every push setting, for snow list";

  outputs = { self, ... }: {
    nixosConfigurations = {
      argon.config = {
        networking.hostName = "argon";
        nixpkgs.hostPlatform.system = "x86_64-linux";
        snowglobe.push = {
          phase = "p1";
          tags = [ "web" "dns" ];
          rebootPolicy = "never";
          serialGroup = "dns";
        };
      };

      boron.config = {
        networking.hostName = "boron";
        nixpkgs.hostPlatform.system = "aarch64-linux";
        snowglobe.push = {
          phase = "p2";
          tags = [ "web" ];
          optional = true;
        };
      };

      curium.config = {
        networking.hostName = "curium";
        nixpkgs.hostPlatform.system = "x86_64-linux";
        snowglobe.push = {
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
