{
  description = "snowglobe - Deploy NixOS and nix-darwin hosts, in phases";

  nixConfig = {
    extra-substituters = [ "https://snowglobe.cachix.org" ];
    extra-trusted-public-keys = [
      "snowglobe.cachix.org-1:GZ4X+iutIwg1WKW/sM/WIfvIoJs+EKUb3VjJikArRtg="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
  };

  outputs = { self, nixpkgs, systems, ... }:
    let
      # Define forEachSystem without referencing nixpkgs.lib to avoid circular deps
      allSystems = import systems;
      forEachSystem = f: builtins.listToAttrs (map
        (system: { name = system; value = f system; })
        allSystems
      );
      # Platforms we can build on: the CI runner and an Apple silicon Mac
      checkedSystems = [ "x86_64-linux" "aarch64-darwin" ];
      e2eFor = system: import ./nix/e2e {
        pkgs = nixpkgs.legacyPackages.${system};
        snow = self.packages.${system}.snow;
        snowglobeModule = self.nixosModules.default;
      };
    in
    {
      # NixOS module for snowglobe deployment support
      nixosModules.default = import ./modules/nixos.nix;

      # nix-darwin module for snowglobe deployment support
      darwinModules.default = import ./modules/darwin.nix;

      # Demo recording for other projects too; they pass their own pkgs
      lib = { inherit (import ./nix/demos) terminalLook recorder renderDemos; };

      packages = forEachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          snow = pkgs.rustPlatform.buildRustPackage {
            pname = "snow";
            version = (builtins.fromTOML (builtins.readFile ./snow/Cargo.toml)).package.version;

            # Only what cargo reads, so editing docs or tests in nix/ doesn't
            # rebuild snow and every VM test that runs it
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./snow
                ./snowlib
              ];
            };

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            nativeBuildInputs = [ pkgs.pkg-config ];

            # Some tests shell out to ssh (expecting it to fail), which and
            # hostname; the darwin sandbox has no /bin on PATH
            nativeCheckInputs = [ pkgs.openssh pkgs.which pkgs.hostname ];

            # These run the real nix CLI, which the build sandbox can't do; CI
            # runs them through cargo test
            checkFlags = [
              "--skip=commands::build::tests"
              "--skip=commands::list::tests"
              "--skip=nix::command::tests::test_tool_runs_without_a_flake"
              "--skip=nix::command::tests::test_failure_carries_stderr"
              "--skip=nix::command::tests::test_out_link_roots_the_built_path"
            ];
            buildInputs = [ ];

            meta = {
              description = "Deploy NixOS and nix-darwin hosts, in phases";
              homepage = "https://github.com/luizribeiro/snowglobe";
              license = with pkgs.lib.licenses; [ mit asl20 ];
              mainProgram = "snow";
            };
          };

          default = self.packages.${system}.snow;
        });

      # Kept out of packages: rendering the demos evaluates every VM test that
      # records one, more than the CI runner that evaluates all packages at
      # once has memory for
      legacyPackages = forEachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          options-md = import ./nix/options-doc {
            inherit pkgs;
            snowglobeModule = self.nixosModules.default;
          };
          deploy-recordings = (e2eFor system).recordings;
          update-demos = pkgs.writeShellApplication {
            name = "update-demos";
            text = ''
              ${pkgs.writers.writePython3 "update-demos" { } (builtins.readFile ./nix/e2e/update_demos.py)} \
                ${self.legacyPackages.${system}.deploy-recordings} "$@"
            '';
          };
        }
      );

      # The e2e VM tests take about 11 GB to evaluate together, so CI
      # evaluates and builds each check in a job of its own
      checks = nixpkgs.lib.genAttrs checkedSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          runNixTests = import ./nix/run-tests.nix { inherit pkgs; };
        in
        {
          snow-package = self.packages.${system}.snow;
          recorder = import ./nix/demos/record.test.nix { inherit pkgs; };
          render-demos = import ./nix/demos/render.test.nix { inherit pkgs; };
          hooks-module = runNixTests "hooks-module" (import ./modules/hooks.test.nix { inherit pkgs; });
          deploy-module = runNixTests "deploy-module" (import ./modules/deploy.test.nix { inherit pkgs; });
          local-pkgs-module = runNixTests "local-pkgs-module" (import ./modules/local-pkgs.test.nix { inherit pkgs; });
          host-metadata = runNixTests "host-metadata" (import ./snowlib/src/backends/nix/metadata.test.nix {
            inherit pkgs;
            snowglobeModule = self.nixosModules.default;
          });
          store-path-check = import ./modules/store-path-check.test.nix { inherit pkgs; };
          darwin-activate = import ./modules/darwin-activate.test.nix { inherit pkgs; };
        } // nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux ({
          nixos-activate = import ./modules/nixos-activate.test.nix { inherit pkgs; };
          options-doc = import ./nix/options-doc/check.nix {
            inherit pkgs;
            page = self.legacyPackages.${system}.options-md;
          };
        } // nixpkgs.lib.mapAttrs'
          (name: test: nixpkgs.lib.nameValuePair "e2e-${name}" test)
          (e2eFor system).tests));

      devShells = forEachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              cargo-audit
              rust-analyzer
              pre-commit
              openssh
            ];

            shellHook = ''
              export PATH="$PWD/target/debug:$PATH"

              # Load .env file if it exists
              if [ -f .env ]; then
                set -a
                source .env
                set +a
              fi

              # Install pre-commit hooks if not already installed
              if [ ! -f .git/hooks/pre-commit ] || ! grep -q "pre-commit" .git/hooks/pre-commit 2>/dev/null; then
                echo "Installing pre-commit hooks..."
                pre-commit install
              fi
            '';
          };
        }
      );
    };
}
