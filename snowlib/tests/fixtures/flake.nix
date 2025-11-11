{
  description = "Test flake for snow operations";

  outputs = { self, ... }:
  let
    # Helper to create a minimal derivation for testing
    makeMockToplevel = name: derivation {
      name = "test-${name}-toplevel";
      system = "x86_64-linux";
      builder = "/bin/sh";
      args = [ "-c" "echo test > $out" ];
    };
  in
  {
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

          # Snowglobe-specific options for testing
          core.pushPhase = "production";
          core.tags = ["web" "database"];

          # Mock build output for testing
          system.build.toplevel = makeMockToplevel "hydrogen";
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

          core.pushPhase = "staging";
          core.tags = ["web"];

          system.build.toplevel = makeMockToplevel "nitrogen";
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

          core.pushPhase = "production";
          core.tags = ["database"];

          system.build.toplevel = makeMockToplevel "oxygen";
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

          # No pushPhase or tags - testing defaults

          system.build.toplevel = makeMockToplevel "helium";
        };
      };
    };
  };
}
