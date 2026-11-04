{ pkgs }:

let
  recorder = (import ./.).recorder { inherit pkgs; };

  script = name: text: pkgs.writeText "${name}.py" text;

  answered = script "answered" ''
    run("printf 'Proceed? '; read answer; echo \"got $answer\"; exit 3")
    expect("Proceed?")
    press("y", ENTER)
  '';
  missingText = script "missing-text" ''
    run("echo done")
    expect("Proceed?")
  '';
  slowText = script "slow-text" ''
    run("sleep 5; echo Proceed?")
    expect("Proceed?", timeout=1)
  '';
  killed = script "killed" ''
    run("kill -KILL $$")
  '';
  exitedBeforeKeys = script "exited-before-keys" ''
    run("echo done")
    press(ENTER)
  '';
  noCommand = script "no-command" ''
    pass
  '';
  twoCommands = script "two-commands" ''
    run("echo one")
    run("echo two")
  '';
in
pkgs.runCommand "record-tests" { nativeBuildInputs = [ recorder pkgs.jq ]; } ''
  record() { command record --prompt luiz@sodium:ops --name demo --title Demo "$@" > "$1.log" 2>&1; }
  # Records SCRIPT into OUT, expecting STATUS and, if given, MESSAGE
  check() {
    local out=$1 script=$2 expected=$3 message=''${4:-} status=0
    record "$out" "$script" || status=$?
    if [ "$status" != "$expected" ] || ! grep -qF -- "$message" "$out.log"; then
      echo "$out: exited $status, expected $expected and '$message'" >&2
      cat "$out.log" >&2
      exit 1
    fi
  }

  check answered ${answered} 3
  grep -q 'got y' answered/demo.cast
  test "$(jq -r .command answered/demo.json)" = "printf 'Proceed? '; read answer; echo \"got \$answer\"; exit 3"
  test "$(tail -n 1 answered/demo.cast | jq -r '.[2]')" = $'\e[1;32mluiz@sodium \e[1;34mops % \e[0m'

  check killed ${killed} 137
  check missing ${missingText} 125 "the command exited without showing 'Proceed?'"
  check slow ${slowText} 125 "no 'Proceed?' within 1s"
  check exited ${exitedBeforeKeys} 125 "the command exited before '\r'"
  check none ${noCommand} 125 "the script ran no command"

  check two ${twoCommands} 0
  test "$(jq -r .command two/demo.json)" = "echo one; echo two"
  grep -q one two/demo.cast
  grep -q two two/demo.cast

  touch $out
''
