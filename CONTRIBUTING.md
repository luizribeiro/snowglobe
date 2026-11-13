# Contributing

Issues, pull requests and questions are welcome. The contributing guide is at
[l9o.dev/snowglobe/contributing](https://l9o.dev/snowglobe/contributing/),
and its source is [docs/contributing.md](docs/contributing.md).

Before you open a pull request:

- Work in `nix develop`. It installs the pre-commit hook (fmt, clippy, tests).
- Build the flake checks your change touches, such as
  `nix build -L .#checks.x86_64-linux.deploy-module`. `nix flake check` runs
  them all, VM tests included, but needs about 11 GB of memory.
- Write Conventional Commits whose body says why, with tests in the same
  commit as the code.

Contributions are dual licensed under Apache-2.0 and MIT, like the rest of
the project.
