{ pkgs }:

let
  demos = import ./.;
  recorder = demos.recorder { inherit pkgs; };

  recorded = name: command: pkgs.runCommand "demo-${name}" { nativeBuildInputs = [ recorder ]; } ''
    record --prompt luiz@sodium:ops --name ${name} --title "Demo ${name}" $out \
      ${pkgs.writeText "${name}.py" ''run("${command}")''}
  '';

  zulu = recorded "zulu" "echo zulu";
  alpha = recorded "alpha" "echo alpha";

  rendered = demos.renderDemos { inherit pkgs; demos = [ zulu alpha ]; };
  duplicated = pkgs.testers.testBuildFailure
    (demos.renderDemos { inherit pkgs; demos = [ alpha alpha ]; });
in
pkgs.runCommand "render-demos-tests" { nativeBuildInputs = [ pkgs.jq ]; } ''
  test "$(jq -r 'map(.name) | join(" ")' ${rendered}/demos.json)" = "zulu alpha"
  test "$(jq -r '.[0].command' ${rendered}/demos.json)" = "echo zulu"
  cmp ${rendered}/zulu.cast ${zulu}/demo.cast
  cmp ${rendered}/alpha.cast ${alpha}/demo.cast
  grep -q '<svg' ${rendered}/zulu.svg
  grep -q '<svg' ${rendered}/alpha.svg
  grep -q '<svg[^>]* x="12"[^>]* y="12"' ${rendered}/zulu.svg || { echo "zulu.svg isn't padded" >&2; exit 1; }
  grep -q '<path class="[^"]*" d="M0 0h[0-9.]*v[0-9.]*H0z"/>' ${rendered}/zulu.svg || { echo "zulu.svg has no cursor" >&2; exit 1; }
  grep -q "two demos are named alpha" ${duplicated}/testBuildFailure.log
  touch $out
''
