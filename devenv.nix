{ pkgs, ... }:

{
  languages.rust.enable = true;

  dotenv.enable = true;

  packages = [
    pkgs.cargo-audit
  ];

  git-hooks.hooks = {
    rustfmt.enable = true;
    clippy = {
      enable = true;
      settings.denyWarnings = true;
      entry = "cargo clippy --workspace --all-targets -- -D warnings";
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
