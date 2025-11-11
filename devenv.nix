{ pkgs, ... }:

{
  languages.rust.enable = true;

  dotenv.enable = true;

  packages = [
    pkgs.cargo-audit
  ];

  pre-commit.hooks = {
    rustfmt.enable = true;
    clippy = {
      enable = true;
      settings.denyWarnings = true;
      settings.allTargets = true;
    };
    cargo-test = {
      enable = true;
      entry = "cargo test --workspace";
      files = "\\.rs$";
      pass_filenames = false;
    };
  };

  enterShell = ''
    export PATH="$PWD/target/debug:$PATH"
  '';
}
