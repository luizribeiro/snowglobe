{ pkgs, pushTest }:

let
  inherit (pkgs) lib;
  scenarios = import ./scenarios.nix;
  index = pkgs.writeText "index.md" (lib.concatMapStrings
    (s: ''
      ## ${s.title}

      `snow ${s.command or "push"} ${s.args}` · [log](${s.name}.log) · [cast](${s.name}.cast)

      ![${s.name}](${s.name}.gif)

    '')
    scenarios);

  # The maintainer's terminal colors ("Custom Monokai"), so the GIFs look like
  # a push in a real session: background, foreground, then 8 normal and 8 bright
  monokai = lib.concatStringsSep "," [
    "1c1c1c" "f8f7f2"
    "262621" "ca4238" "bbd756" "ded685" "85c2d6" "7b72c2" "cf5a78" "fefefe"
    "717171" "df3f67" "bbd756" "ded685" "85c2d6" "7b72c2" "cf5a78" "fefefe"
  ];
  # Twice the 12pt terminal font they match, so shown at half size on a Retina screen the
  # text is as big as in that terminal, and sharp
  fontSize = 24;
  # agg can't draw color emoji; this covers the ✨ in snow's output
  emojiFont = pkgs.noto-fonts-monochrome-emoji;
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
        --font-dir ${pkgs.nerd-fonts.hack}/share/fonts \
        --font-dir ${pkgs.dejavu_fonts}/share/fonts \
        --font-dir ${emojiFont}/share/fonts \
        --font-family "Hack Nerd Font Mono,DejaVu Sans Mono,Noto Emoji" \
        --theme ${monokai} \
        --font-size ${toString fontSize} \
        --idle-time-limit 3 \
        --last-frame-duration 6 \
        "$cast" "''${cast%.cast}.gif"
    done
  ''
