{
  description = "Test flake for snow CLI and mock testing";

  outputs = { self, ... }: {
    nixosConfigurations = {
      hedgehog = {
        config = {
          services.openssh.enable = true;
          services.postgresql.enable = true;
          services.nginx.enable = true;
          networking.hostName = "hedgehog";
          networking.domain = "example.com";
          nixpkgs.hostPlatform = "x86_64-linux";

          # For mock testing
          snowglobe.deploy.phase = "production";
          snowglobe.deploy.tags = [ "web" "frontend" ];
        };
      };

      nuthatch = {
        config = {
          services.openssh.enable = true;
          services.postgresql.enable = false;
          services.nginx.enable = true;
          networking.hostName = "nuthatch";
          networking.domain = "example.com";
          nixpkgs.hostPlatform = "aarch64-linux";

          # For mock testing
          snowglobe.deploy.phase = "production";
          snowglobe.deploy.tags = [ "database" ];
        };
      };

      osprey = {
        config = {
          services.openssh.enable = true;
          services.postgresql.enable = true;
          services.nginx.enable = false;
          networking.hostName = "osprey";
          networking.domain = "";
          nixpkgs.hostPlatform = "x86_64-linux";

          # For mock testing
          snowglobe.deploy.phase = "staging";
          snowglobe.deploy.tags = [ "web" "backend" ];
        };
      };

      wren = {
        config = {
          services.openssh.enable = false;
          services.postgresql.enable = false;
          services.nginx.enable = false;
          networking.hostName = "wren";
          networking.domain = "local";
          nixpkgs.hostPlatform = "aarch64-linux";

          # For mock testing
          snowglobe.deploy.phase = "dev";
          snowglobe.deploy.tags = [ "test" ];
        };
      };
    };
  };
}
