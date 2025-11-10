{ ... }:

{
  languages.rust.enable = true;

  enterShell = ''
    export PATH="$PWD/target/debug:$PATH"

    # Load .env file if it exists
    if [ -f .env ]; then
      set -a
      source .env
      set +a
    fi
  '';
}
