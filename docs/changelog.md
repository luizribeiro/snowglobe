---
title: "Changelog"
group: "Project"
weight: 125
description: "Where to find the list of changes in each snowglobe release, and what compatibility to expect before 1.0."
---

The changes in each release are recorded in [`CHANGELOG.md`](https://github.com/luizribeiro/snowglobe/blob/main/CHANGELOG.md) in the repository. It follows the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, with an `Unreleased` section at the top for changes that are merged but not yet released.

Each release on the [GitHub releases page](https://github.com/luizribeiro/snowglobe/releases) has its release notes and prebuilt `snow` binaries: static Linux binaries for x86_64 and aarch64, macOS binaries for x86_64 and aarch64, and a `SHA256SUMS` file to check them against.

## Compatibility before 1.0

snowglobe follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html), but it is pre-1.0. Until 1.0, a minor release (for example 0.1 to 0.2) may break the `snow` command line, the module options and the library API. Read the changelog entries between your version and the new one before you upgrade, and pay attention to changed or removed module options: a removed option makes your host configurations fail to evaluate until you update them.

If you pin snowglobe as a flake input, upgrading is a matter of updating that input:

```bash
nix flake update snowglobe
```

If you run `snow` from that same input (for example its `packages.<system>.default`), it is updated along with the module. If you installed a release binary instead, replace it with the same release, so that the command line and the module on your hosts match. Then build your hosts with `snow build` before deploying, so that any breaking change shows up as an evaluation or build error rather than during a deployment.
