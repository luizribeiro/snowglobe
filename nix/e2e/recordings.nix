{ pkgs, pushTest }:

let
  scenarios = import ./scenarios.nix;
  index = pkgs.writeText "index.md" (pkgs.lib.concatMapStrings
    (s: ''
      ## ${s.title}

      `snow push ${s.args}` · [log](${s.name}.log) · [cast](${s.name}.cast)

      ![${s.name}](${s.name}.gif)

    '')
    scenarios);
in
pkgs.runCommand "snow-push-recordings"
{
  nativeBuildInputs = [ pkgs.asciinema-agg ];
}
  ''
    mkdir -p $out
    cp ${pushTest}/casts/* $out/
    cp ${index} $out/index.md
    for cast in $out/*.cast; do
      agg --quiet \
        --font-dir ${pkgs.jetbrains-mono}/share/fonts \
        --font-dir ${pkgs.dejavu_fonts}/share/fonts \
        --font-family "JetBrains Mono,DejaVu Sans Mono" \
        --theme github-dark \
        --idle-time-limit 3 \
        --last-frame-duration 6 \
        "$cast" "''${cast%.cast}.gif"
    done
  ''
