{
  description = "Test flake for snow list command";

  outputs = { self, ... }: {
    nixosConfigurations = {
      hydrogen = {
        config = {
          # Standard NixOS options
          services.openssh.enable = true;
          services.postgresql.enable = true;
          services.nginx.enable = true;
          networking.hostName = "hydrogen";
          networking.domain = "example.com";
          nixpkgs.hostPlatform = "x86_64-linux";
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
        };
      };
    };
  };
}
