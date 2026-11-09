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
      hedgehog = {
        config = {
          # Standard NixOS options
          services.openssh.enable = true;
          services.postgresql.enable = true;
          services.nginx.enable = true;
          networking.hostName = "hedgehog";
          networking.domain = "example.com";
          nixpkgs.hostPlatform = "x86_64-linux";

          # Snowglobe-specific options for testing
          snowglobe.deploy.phase = "production";
          snowglobe.deploy.tags = ["web" "database"];

          # Mock build output for testing
          system.build.toplevel = makeMockToplevel "hedgehog";
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

          snowglobe.deploy.phase = "staging";
          snowglobe.deploy.tags = ["web"];

          system.build.toplevel = makeMockToplevel "nuthatch";
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

          snowglobe.deploy.phase = "production";
          snowglobe.deploy.tags = ["database"];

          system.build.toplevel = makeMockToplevel "osprey";
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

          # No phase or tags - testing defaults

          system.build.toplevel = makeMockToplevel "helium";
        };
      };
    };
  };
}
