{ ... }:

{
  languages.rust.enable = true;

  enterShell = ''
    export PATH="$PWD/snow/target/debug:$PATH"
  '';
}
