{ ... }:

{
  languages.rust.enable = true;

  enterShell = ''
    export PATH="$PWD/target/debug:$PATH"
  '';
}
