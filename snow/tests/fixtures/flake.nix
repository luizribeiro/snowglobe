{
  description = "Test flake for snow CLI and mock testing";

  outputs = { self, ... }: {
    nixosConfigurations = {
      hydrogen = {
        config = {
          services.openssh.enable = true;
          services.postgresql.enable = true;
          services.nginx.enable = true;
          networking.hostName = "hydrogen";
          networking.domain = "example.com";
          nixpkgs.hostPlatform = "x86_64-linux";

          # For mock testing
          snowglobe.deploy.phase = "production";
          snowglobe.deploy.tags = [ "web" "frontend" ];
        };
      };

      nitrogen = {
        config = {
          services.openssh.enable = true;
          services.postgresql.enable = false;
          services.nginx.enable = true;
          networking.hostName = "nitrogen";
          networking.domain = "example.com";
          nixpkgs.hostPlatform = "aarch64-linux";

          # For mock testing
          snowglobe.deploy.phase = "production";
          snowglobe.deploy.tags = [ "database" ];
        };
      };

      oxygen = {
        config = {
          services.openssh.enable = true;
          services.postgresql.enable = true;
          services.nginx.enable = false;
          networking.hostName = "oxygen";
          networking.domain = "";
          nixpkgs.hostPlatform = "x86_64-linux";

          # For mock testing
          snowglobe.deploy.phase = "staging";
          snowglobe.deploy.tags = [ "web" "backend" ];
        };
      };

      helium = {
        config = {
          services.openssh.enable = false;
          services.postgresql.enable = false;
          services.nginx.enable = false;
          networking.hostName = "helium";
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
