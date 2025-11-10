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
  };

  enterShell = ''
    export PATH="$PWD/target/debug:$PATH"
  '';
}
