{
  description = "snowglobe - NixOS-native cluster orchestration tool";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems.url = "github:nix-systems/default";
  };

  outputs = { self, nixpkgs, systems, ... }@inputs:
    let
      forEachSystem = nixpkgs.lib.genAttrs (import systems);
    in
    {
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
            ];

            shellHook = ''
              export PATH="$PWD/target/debug:$PATH"

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
