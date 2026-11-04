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

      # Demo recording for other projects too; they pass their own pkgs
      lib = { inherit (import ./nix/demos) terminalLook recorder renderDemos; };

      packages = forEachSystem (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          snow = pkgs.rustPlatform.buildRustPackage {
            pname = "snow";
            version = "0.1.0";

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

            nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];

            # Some tests shell out to ssh (expecting it to fail), which and
            # hostname; the darwin sandbox has no /bin on PATH
            nativeCheckInputs = [ pkgs.openssh pkgs.which pkgs.hostname ];

            # These run the real nix CLI, which the build sandbox can't do; CI
            # runs them through cargo test
            checkFlags = [
              "--skip=commands::build::tests"
              "--skip=commands::list::tests"
              "--skip=commands::status::tests"
              "--skip=commands::plan::tests::test_plan_invalid_host"
              "--skip=nix::command::tests::test_tool_runs_without_a_flake"
              "--skip=nix::command::tests::test_failure_carries_stderr"
              "--skip=nix::command::tests::test_out_link_roots_the_built_path"
            ];
            buildInputs = [ ];

            # snow plan shells out to nix-eval-jobs; a user's own copy on PATH takes precedence.
            postInstall = ''
              wrapProgram $out/bin/snow \
                --suffix PATH : ${pkgs.lib.makeBinPath [ pkgs.nix-eval-jobs ]}
            '';

            meta = {
              description = "NixOS-native cluster orchestration tool";
              homepage = "https://github.com/luizribeiro/snowglobe";
              license = pkgs.lib.licenses.mit;
              mainProgram = "snow";
            };
          };

          default = self.packages.${system}.snow;
        } // nixpkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          # NixOS VMs pushing to each other, too slow for every check
          push-e2e = import ./nix/e2e/push.nix {
            inherit pkgs;
            snow = self.packages.${system}.snow;
            snowglobeModule = self.nixosModules.default;
          };
          push-recordings = import ./nix/e2e/recordings.nix {
            inherit pkgs;
            pushTest = self.packages.${system}.push-e2e;
          };
          upload-demos = pkgs.writeShellApplication {
            name = "upload-demos";
            runtimeInputs = [ pkgs.asciinema ];
            text = ''
              ${pkgs.writers.writePython3 "upload-demos" { } (builtins.readFile ./nix/e2e/upload_demos.py)} \
                ${self.packages.${system}.push-recordings} "$@"
            '';
          };
        } // nixpkgs.lib.optionalAttrs pkgs.stdenv.isLinux (nixpkgs.lib.mapAttrs'
          (name: test: nixpkgs.lib.nameValuePair "e2e-${name}" test)
          (import ./nix/e2e {
            inherit pkgs;
            snow = self.packages.${system}.snow;
            snowglobeModule = self.nixosModules.default;
          }))
      );

      checks = nixpkgs.lib.genAttrs checkedSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          runNixTests = import ./nix/run-tests.nix { inherit pkgs; };
        in
        {
          snow-package = self.packages.${system}.snow;
          recorder = import ./nix/demos/record.test.nix { inherit pkgs; };
          hooks-module = runNixTests "hooks-module" (import ./modules/hooks.test.nix { inherit pkgs; });
          push-module = runNixTests "push-module" (import ./modules/push.test.nix { inherit pkgs; });
          local-pkgs-module = runNixTests "local-pkgs-module" (import ./modules/local-pkgs.test.nix { inherit pkgs; });
          darwin-activate = import ./modules/darwin-activate.test.nix { inherit pkgs; };
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
              nix-eval-jobs

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
