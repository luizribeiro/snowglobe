{ ... }:

{
  languages.rust.enable = true;

  dotenv.enable = true;

  pre-commit.hooks = {
    rustfmt.enable = true;
    clippy = {
      enable = true;
      settings.denyWarnings = true;
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
