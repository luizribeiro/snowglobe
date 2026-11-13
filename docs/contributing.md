---
title: "Contributing"
group: "Project"
weight: 120
description: "Set up a development environment, run the tests, record demos, and understand how the code and releases are organised."
---

snowglobe welcomes issues, pull requests and questions about how things work. This page covers setting up a development environment, the layers of tests and what each one is for, recording the demos, how the code is organised, and how commits and releases are made.

snowglobe is pre-1.0. Breaking changes to the CLI, the module options and the library API are acceptable when they improve the design; note them in the changelog.

## Development environment

You need Nix with flakes enabled. Clone the repository and enter the development shell:

```bash
git clone https://github.com/luizribeiro/snowglobe
cd snowglobe
nix develop        # or, with direnv: direnv allow (.envrc is `use flake .`)
```

The shell provides the Rust toolchain and everything else the tests need, and installs a pre-commit hook that runs `cargo fmt --check`, `cargo clippy` with warnings denied, and `cargo test`.

Build and run the CLI from the checkout:

```bash
cargo build
cargo run --bin snow -- --help
```

If you have `FLAKE_PATH` set for your own flake, unset it while developing. `snow` reads it, and a development build pointed at a real fleet can deploy to it.

Some commands are behind the `unstable` Cargo feature and left out of release builds. Build them with:

```bash
cargo build --features unstable
```

The flake's `snow-unstable` package has them too.

## Tests

There are several layers of tests. Each one catches problems that the layer below cannot, and each is slower than the one below. Start with the fastest layer that can show your change works.

### Rust tests

```bash
cargo test --workspace
cargo test --workspace --all-features   # include the unstable commands
```

Most of the deployment logic is tested here. Nix, SSH deploys, reboots and hooks sit behind backend traits with mock implementations, and a test observer records every event each host reports. That lets a test run a whole deployment, with phases, serial groups, hook failures and reboots, without Nix or SSH, and assert on exactly what each host did and in what order. The deployment tests live in [`snowlib/src/deploy/tests/`](https://github.com/luizribeiro/snowglobe/tree/main/snowlib/src/deploy/tests).

### CLI tests

[`snow/tests/`](https://github.com/luizribeiro/snowglobe/tree/main/snow/tests) runs the real `snow` binary with `assert_cmd`. Debug builds accept a hidden fixture option that replaces Nix and SSH with scripted deploy scenarios, so these tests check argument parsing, output and exit codes end to end.

### Nix module tests

The NixOS and nix-darwin modules, the activation helpers, the store-path check and the demo recorder have their own flake checks, listed under `checks` in [`flake.nix`](https://github.com/luizribeiro/snowglobe/blob/main/flake.nix). Build one by name:

```bash
nix build -L .#checks.x86_64-linux.deploy-module
```

They check what options evaluate to and how the helper scripts behave, without booting anything.

The [module options](/snowglobe/options/) page is generated from the option descriptions, and the `options-doc` check fails when `docs/options.md` is out of date. After changing an option, or updating nixpkgs, which renders the types and defaults, regenerate it:

```bash
nix build .#options-md && cat result > docs/options.md
```

### VM tests

The VM tests in [`nix/e2e/tests/`](https://github.com/luizribeiro/snowglobe/tree/main/nix/e2e/tests) boot several NixOS virtual machines and run real deployments between them: a deployer VM runs `snow` against hosts that boot a shared base system. They catch what mocks cannot, such as a non-root deploy user missing a permission, a host that never comes back from a reboot, or a switch that races systemd. They need Linux with KVM.

Build one test by name:

```bash
nix build -L .#checks.x86_64-linux.e2e-switch
```

To poke at the VMs yourself, build the interactive driver:

```bash
nix build .#checks.x86_64-linux.e2e-switch.driverInteractive
./result/bin/nixos-test-driver
```

Build the test you need rather than running `nix flake check`, which evaluates every VM test at once and needs about 11 GB of memory. CI builds each check in its own job for the same reason.

A test file sets `name` (matching its file name), the `nodes` that `snow` deploys, an optional `demo`, or a list of `demos` that the `testScript` runs by name, and a `testScript`. New tests must also be added to `names` in [`nix/e2e/default.nix`](https://github.com/luizribeiro/snowglobe/blob/main/nix/e2e/default.nix); evaluation fails if the list and the directory disagree. New features should come with a VM test whenever they can.

## Demos

The demos on this site are recorded inside the VM tests. A test with a `demo`, or with `demos` as `tutorial.nix` has, records an asciinema cast of `snow` while the test runs, and the test then checks the output. A demo therefore cannot show behaviour the code does not have.

```bash
nix build .#deploy-recordings   # build every cast and its SVG
nix run .#update-demos          # copy them into docs/demos and rewrite docs/demos.md
```

The site plays the casts; the SVGs are for GitHub, where the README and `docs/demos.md` show them. A page shows a recording with an image link to its cast, such as `![Two hosts in a serial group take turns](demos/serial-group.cast)`.

## Architecture

The code is organised around a few ideas.

**Two crates.** [`snow/`](https://github.com/luizribeiro/snowglobe/tree/main/snow) is the CLI: argument parsing with clap, the progress board, prompts and the final report. [`snowlib/`](https://github.com/luizribeiro/snowglobe/tree/main/snowlib) does the work and never writes to the terminal, so another frontend could use it.

**Backends are traits.** Nix evaluation and builds (`NixBackend`), SSH deploys (`DeployBackend`), reboot detection and reboots (`RebootBackend`) and hooks (`HookBackend`) each sit behind a trait in `snowlib/src/backends/`, with a real implementation and a mock. The real ones run `nix` and `ssh` as subprocesses. The mocks are what make the deployment tests possible without Nix or SSH.

**Each host runs its own pipeline.** `deploy()` in [`snowlib/src/deploy/mod.rs`](https://github.com/luizribeiro/snowglobe/blob/main/snowlib/src/deploy/mod.rs) runs phases in order, and inside a phase runs every host's pipeline concurrently with tokio. [`pipeline.rs`](https://github.com/luizribeiro/snowglobe/blob/main/snowlib/src/deploy/pipeline.rs) holds the steps a host goes through, and [`serial.rs`](https://github.com/luizribeiro/snowglobe/blob/main/snowlib/src/deploy/serial.rs) the concurrency limit and serial groups, the one place where hosts wait for each other. [How a deployment works](/snowglobe/how-deploy-works/) describes the behaviour.

**Progress is events.** Each step a host runs goes through `HostRun::step`, which reports `HostEvent::Started`, then `Finished` or `Failed`, to a `DeployObserver`; every host ends with `HostEvent::Done` and its result. The CLI's observer draws one row per host from these events, `LogObserver` writes them to the log, and the tests assert on them. When you add a pipeline step, report it the same way.

To add a command, add a module under `snow/src/commands/`, define its arguments with clap derives, add it to the `Commands` enum in `snow/src/commands/mod.rs`, and add CLI tests. Put the logic in `snowlib`.

Comments explain why code is the way it is, not what it does; a well-named function is preferred over a comment.

## Commits

- Use [Conventional Commits](https://www.conventionalcommits.org/): `type(scope): Capitalized imperative subject`, for example `fix(deploy): Skip hosts whose serial group halted`. Mark breaking changes with `!`.
- The body explains why the change is made.
- Keep commits small, one idea each, with tests in the same commit as the code they test.
- Keep history linear: rebase onto `main` and fast-forward, without merge commits.

CI runs formatting, clippy, the Rust tests with and without the unstable feature, the documentation build, `cargo audit`, every flake check including each VM test, and `nix flake check` on macOS. See [`.github/workflows/ci.yml`](https://github.com/luizribeiro/snowglobe/blob/main/.github/workflows/ci.yml).

## Releases

1. Bump the version in `snow/Cargo.toml`, and run `cargo build` so `Cargo.lock` matches.
2. Add a `## [X.Y.Z]` section to [`CHANGELOG.md`](https://github.com/luizribeiro/snowglobe/blob/main/CHANGELOG.md). It becomes the release notes.
3. Push a `vX.Y.Z` tag on `main`. It must match the version in `snow/Cargo.toml`, the tagged commit must be on `main`, and CI must have passed on it, or the release workflow stops.

The [release workflow](https://github.com/luizribeiro/snowglobe/blob/main/.github/workflows/release.yml) publishes the release on the [releases page](https://github.com/luizribeiro/snowglobe/releases), with the changelog section as its notes. It builds nothing: people install snowglobe from the flake at the tag. Running the workflow by hand checks that `CHANGELOG.md` has a section for the current version, without publishing.

## License

snowglobe is dual licensed under the [Apache License 2.0](https://github.com/luizribeiro/snowglobe/blob/main/LICENSE-APACHE) and the [MIT license](https://github.com/luizribeiro/snowglobe/blob/main/LICENSE-MIT). Unless you state otherwise, contributions are licensed the same way.
