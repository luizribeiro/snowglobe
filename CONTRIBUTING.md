# Contributing

Thanks for looking. snowglobe is young and small, so issues and PRs are
welcome, and so are questions about how things work.

## Setup

```bash
nix develop        # or direnv: .envrc is `use flake .`
```

The dev shell has cargo, rustc, rustfmt, clippy, cargo-audit, rust-analyzer,
pre-commit, nix-eval-jobs, openssh, ssh-to-age and zstd, plus libguestfs and
vma on Linux. On entry it puts `target/debug` on `PATH`, sources `.env` and
installs the pre-commit hook if it isn't there yet.

Run from source:

```bash
cargo run --bin snow -- list
```

If you have `FLAKE_PATH` set (say, pointing at your own flake), snow reads it.
Unset it while developing.

The pre-commit hook runs on Rust files:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Tests

There are four layers. Most changes touch one or two.

### 1. Rust tests

```bash
cargo test --workspace
cargo test --workspace --all-features   # includes the unstable commands
```

Unit tests are inline `#[cfg(test)]`. Push orchestration tests are in
`snowlib/src/push/tests/`, with mock backends and a `TestObserver` that
records the events each host reports. A few tests in `snow/src` run the real
`nix` against fixture flakes, so they need it installed. The Nix package
build skips them; CI runs them through cargo.

### 2. CLI tests

`snow/tests/cli_tests.rs`, with `assert_cmd`, against the fixture flakes in
`snow/tests/fixtures/` and mock push scenarios (success, partial failure,
build failure, network error).

### 3. Nix module tests

Flake checks on x86_64-linux and aarch64-darwin: `hooks-module`,
`push-module`, `local-pkgs-module`, `store-path-check`, `darwin-activate`,
`nixos-activate` (Linux only), `recorder`, `render-demos`, `snow-package`,
`snow-unstable-package`. They use `lib.debug.runTests`, so test attributes
must be named `test*`.

```bash
nix build -L .#checks.x86_64-linux.push-module
```

### 4. VM tests (Linux)

Real pushes between NixOS VMs, on `pkgs.testers.runNixOSTest`. One file per
test in `nix/e2e/tests/<name>.nix`, checked as `e2e-<name>`. A VM called
`sodium` runs snow and pushes to the others.

```bash
nix build -L .#checks.x86_64-linux.e2e-switch

# interactive
nix build .#checks.x86_64-linux.e2e-switch.driverInteractive
./result/bin/nixos-test-driver
```

A test file has `name` (same as the file), `nodes`, and optionally `common`,
`base.<host>`, `testScript` and `demo`. It needs a `demo` or a `testScript`.
Add new tests to `names` in `nix/e2e/default.nix`. That order is also the
order of the demos page.

A full `nix flake check` on Linux evaluates all of them, which takes about
11 GB of memory. Build the one you need.

New features should come with a VM test when they can.

### Demos

A test's `demo = { title; script; runOn?; user?; }` records an asciinema cast
during the VM test. So every demo is also a test.

```bash
nix build .#push-recordings   # casts, SVGs and demos.json
nix run .#upload-demos        # upload, rewrite docs/demos.md, replace the SVGs
```

Run `asciinema auth` once first, or the uploads expire. Uploading changes
the asciinema URLs, so update the README's demo links by hand afterwards.

The flake's `lib` exports `terminalLook`, `recorder` and `renderDemos` if
you want the same setup for your own project.

## CI

Through `nix develop --command`:

- `cargo fmt --check`
- clippy with `-D warnings`, default and `--all-features`
- `cargo test --workspace`, default and `--all-features`
- `cargo doc --workspace --no-deps`
- `cargo audit`
- `cargo build --workspace --release`
- each flake check that isn't cached, built on its own:
  `nix build -L --no-link .#checks.x86_64-linux.<check>`. On macOS,
  `nix flake check -L`.

## How it's built

Two crates:

- `snow/`: the CLI. clap, the board with one row per host, prompts.
- `snowlib/`: the library. No terminal I/O.

`snow/src/commands/` has `list`, `build`, `push/` (board, observer,
summary…) and `rebuild`, plus the unstable `bootstrap`, `plan` and `status`.
Host selection is shared in `host_selection.rs`.

`snowlib/src/backends/` has the backend traits, each with a real and a mock
implementation, so tests run without Nix or SSH:

- `NixBackend`: eval and build
- `DeployBackend`: SSH for push, local for rebuild
- `RebootBackend`
- `HookBackend`

`snowlib/src/push/`:

- `push()` runs the phases in order.
- `execute_phase()` runs every host's pipeline at once: Build, Check
  (unchanged?), Copy, Plan, PreDeploy, Activate, Reboot, PostDeploy. Serial
  groups take turns from PreDeploy through PostDeploy.
- Hosts report `HostEvent`s to a `PushObserver`. The CLI's observer draws the
  board; `LogObserver` writes the log file.

`modules/`:

- `nixos.nix` and `darwin.nix` are the flake's modules.
- Options: `push.nix`, `hooks.nix`, `local-pkgs.nix`, `bootstrap.nix`.
- The helper: `nixos-activate.nix`, `darwin-activate.nix`,
  `store-path-check.nix`. How snow finds and uses it is in
  [docs/activation-helper.md](docs/activation-helper.md).

## Commits

- Conventional Commits: `type(scope): Capitalized imperative subject`. Types:
  `feat`, `fix`, `refactor`, `test`, `docs`, `ci`. `!` for breaking changes.
- The body says why.
- Small commits, one idea each, tests in the same commit.
- Linear history: rebase, then fast-forward. No merge commits.

## Releases

1. Bump the version in `snow/Cargo.toml` and `flake.nix` (it's hard-coded in
   both).
2. Add a `## [X.Y.Z]` section to `CHANGELOG.md`. The release notes come from
   it.
3. Push a `vX.Y.Z` tag on main. It must match `snow/Cargo.toml`.

To try the archives first, run the release workflow by hand: it builds the
same archives as workflow artifacts, without publishing anything.

The release builds static binaries for Linux (x86_64 and aarch64, musl) and
macOS (x86_64 and aarch64), plus `SHA256SUMS`.

## License

Contributions are dual licensed under Apache-2.0 and MIT, like the rest of
the project. See the README.
