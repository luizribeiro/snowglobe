# Shell helpers shared by the activation helper tests. sideEffect is a file
# the helper's faked privileged commands create, so a rejected call must not
# leave it behind.
{ helper, sideEffect, sideEffectName }:

''
  activate=${helper}/bin/snowglobe-activate
  hash=0123456789abcdfghijklmnpqrsvwxyz

  fail() { echo "FAIL: $*" >&2; exit 1; }

  expect_error() {
    local message="$1" output
    shift
    output=$("$activate" "$@" 2>&1) && fail "snowglobe-activate $* should have failed"
    [[ "$output" == *"$message"* ]] || fail "snowglobe-activate $* should fail with '$message', got: $output"
    [ ! -e "${sideEffect}" ] || fail "snowglobe-activate $* must not ${sideEffectName}"
  }

  expect_traversal_rejected() {
    local script="$1" mode="$2"
    local evil="$TMPDIR/evil/$script" traversal="/nix/store/../..$TMPDIR/evil"
    mkdir -p "$(dirname "$evil")"
    printf '#!/bin/sh\ntouch "$TMPDIR/pwned"\n' > "$evil"
    chmod +x "$evil"
    [ -x "$traversal/$script" ] || fail "$traversal does not reach $evil"
    expect_error "Invalid store path" "$traversal" "$mode"
    [ ! -e "$TMPDIR/pwned" ] || fail "ran a script outside the store"
  }
''
