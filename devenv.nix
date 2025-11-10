{ ... }:

{
  languages.rust.enable = true;

  dotenv.enable = true;

  enterShell = ''
    export PATH="$PWD/target/debug:$PATH"
  '';
}
