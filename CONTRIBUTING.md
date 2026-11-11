# Contributing

Thanks for looking. snowglobe is young and small, so issues and PRs are
welcome, and so are questions about how things work.

## Setup

```bash
nix develop        # or direnv: .envrc is `use flake .`
cargo run --bin snow -- list
```

The dev shell has everything, and installs the pre-commit hook (fmt, clippy,
tests). If you have `FLAKE_PATH` set for your own flake, unset it while
developing, or snow will read it.

## Tests

There are a few layers, each catching something the one below can't.

**Rust tests**, `cargo test --workspace` (add `--all-features` for the
unstable commands). Deploy logic runs against mock backends and records the
events each host reports, so you can test a whole deployment, failures included,
without Nix or SSH. Start here.

**CLI tests**, in `snow/tests/`. They run the real binary, with mock deploy
scenarios standing in for Nix and SSH.

**Nix module tests**, as flake checks. They cover the options and the
activation helper:

```bash
nix build -L .#checks.x86_64-linux.deploy-module
```

**VM tests** (Linux), in `nix/e2e/tests/`. Real deployments between NixOS VMs.
They catch what mocks can't, like a non-root deploy user missing a
permission. New features should come with one when they can.

```bash
nix build -L .#checks.x86_64-linux.e2e-switch

# interactive
nix build .#checks.x86_64-linux.e2e-switch.driverInteractive
./result/bin/nixos-test-driver
```

Build the test you need rather than `nix flake check`, which evaluates every
VM test (about 11 GB of memory). New tests go in `names` in
`nix/e2e/default.nix`.

### Demos

The demos are recorded inside the VM tests. A test with a `demo`, or a list of
`demos`, records an asciinema cast while it runs, so a demo can't show something the code
doesn't do.

```bash
nix build .#deploy-recordings   # casts and SVGs
nix run .#update-demos          # copy them into docs/demos, rewrite docs/demos.md
```

CI runs all of the above; see `.github/workflows/ci.yml`.

## Design

A few ideas the code is built around:

- **Two crates.** `snow/` is the CLI: arguments, the board, prompts.
  `snowlib/` does the work and never touches the terminal, so another
  frontend could use it.
- **Backends are traits.** Nix, SSH deploys, reboots and hooks each sit
  behind a trait with a real and a mock version. That's what lets the deployment
  tests run without Nix or SSH.
- **Each host runs its own pipeline.** Build, check, copy, plan, deploy,
  reboot, per host, all at once. Fast hosts don't wait for slow ones. Serial
  groups are the one place hosts wait for each other.
- **Progress is events.** Hosts report what they're doing to an observer. The
  CLI draws the board from them, another observer writes the log, and the
  tests assert on them.

## Commits

- Conventional Commits: `type(scope): Capitalized imperative subject`, with
  `!` for breaking changes.
- The body says why.
- Small commits, one idea each, tests in the same commit.
- Linear history: rebase, then fast-forward. No merge commits.

## Releases

1. Bump the version in `snow/Cargo.toml` and `flake.nix`.
2. Add a `## [X.Y.Z]` section to `CHANGELOG.md`. It becomes the release
   notes.
3. Push a `vX.Y.Z` tag on main, matching `snow/Cargo.toml`.

The release workflow publishes the tag's changelog section as release notes.
It builds no binaries, since snowglobe is installed from the flake. You can
run it by hand to check the notes without publishing.

## License

Contributions are dual licensed under Apache-2.0 and MIT, like the rest of
the project. See the README.
