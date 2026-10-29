{
  description = "snowglobe - NixOS-native cluster orchestration tool";

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
      # Platforms we can build on: the CI runner and ferrum
      checkedSystems = [ "x86_64-linux" "aarch64-darwin" ];
    in
    {
      # NixOS module for snowglobe deployment support
      nixosModules.default = import ./modules/nixos.nix;

      # nix-darwin module for snowglobe deployment support
      darwinModules.default = import ./modules/darwin.nix;

      packages = forEachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          snow = pkgs.rustPlatform.buildRustPackage {
            pname = "snow";
            version = "0.1.0";

            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            nativeBuildInputs = [ pkgs.pkg-config ];

            # Some tests shell out to ssh (expecting it to fail), which and
            # hostname; the darwin sandbox has no /bin on PATH
            nativeCheckInputs = [ pkgs.openssh pkgs.which pkgs.hostname ];

            # These run the real nix CLI against snow/tests/fixtures, which the
            # build sandbox can't do; CI runs them through cargo test
            checkFlags = [
              "--skip=commands::build::tests"
              "--skip=commands::list::tests"
              "--skip=commands::status::tests"
            ];
            buildInputs = [ ];

            meta = {
              description = "NixOS-native cluster orchestration tool";
              homepage = "https://github.com/luizribeiro/snowglobe";
              license = pkgs.lib.licenses.mit;
              mainProgram = "snow";
            };
          };

          default = self.packages.${system}.snow;
        }
      );

      checks = nixpkgs.lib.genAttrs checkedSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          runNixTests = import ./nix/run-tests.nix { inherit pkgs; };
        in
        {
          snow-package = self.packages.${system}.snow;
          hooks-module = runNixTests "hooks-module" (import ./modules/hooks.test.nix { inherit pkgs; });
          push-module = runNixTests "push-module" (import ./modules/push.test.nix { inherit pkgs; });
          local-pkgs-module = runNixTests "local-pkgs-module" (import ./modules/local-pkgs.test.nix { inherit pkgs; });
        } // nixpkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          nixos-activate = import ./modules/nixos-activate.test.nix { inherit pkgs; };
        });

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

              # Bootstrap dependencies
              openssh                    # ssh-keygen
              ssh-to-age                 # age key derivation
              zstd                       # VMA compression
            ] ++ lib.optionals stdenv.isLinux [
              libguestfs-with-appliance  # guestfish for image provisioning
              (callPackage ./pkgs/vma { })  # Proxmox VMA tool
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
