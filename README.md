# snowglobe

[![CI](https://github.com/luizribeiro/snowglobe/workflows/CI/badge.svg)](https://github.com/luizribeiro/snowglobe/actions)

Self-contained cluster orchestration for NixOS.

> **Status**: Early design phase. Not ready for use.

[![snow push: three hosts in two phases](docs/demos/switch.svg)](https://asciinema.org/a/FG9AARHrB4eEO1jq)

More in [the demos](docs/demos.md), recorded from real pushes in the VM tests.

## Vision

snowglobe is a NixOS-native cluster orchestration tool that provides:
- **Declarative cluster configuration** - Define your cluster in Nix
- **Push-based deployment** - Deploy configurations to NixOS and nix-darwin hosts
- **Activation helper** - Passwordless deployment through one auditable sudo command
- **Reboot detection** - Automatically handle system updates requiring reboots
- **Agent-based deployment** (planned) - Nodes manage themselves (pull-based)
- **Built-in health checking** (planned) - Know when things break
- **etcd-backed state** (planned) - Distributed state management
- **VM testing** (planned) - Test deployments safely before production

Think: A modern alternative to `deploy-rs` with cluster awareness.

## Features

### Passwordless Deployment

Snowglobe includes NixOS and nix-darwin modules that provide passwordless system activation:

```nix
{
  imports = [ inputs.snowglobe.nixosModules.default ];

  snowglobe.push.allowedUsers = [ "deploy" ];
}
```

This installs the `snowglobe-activate` command that:
- Works around sudoers wildcard limitations on macOS
- Provides a stable path for sudo configuration
- Puts one auditable command in sudoers instead of `NOPASSWD: ALL`

Allowed users are root-equivalent on the host, as with any push deployment tool, so use a dedicated deploy user.

Snowglobe automatically detects and uses `snowglobe-activate` when available, falling back gracefully for compatibility.

See [docs/activation-helper.md](docs/activation-helper.md) for details.

### Smart Reboot Detection

Snowglobe automatically detects when deployments require system reboots by checking:
- Kernel and initrd changes (symlink comparison)
- Critical service restarts (systemd analysis)
- Per-host reboot policies

Hosts can configure their own reboot behavior:

```nix
{
  config.core.rebootPolicy = "auto";  # or "never", "always"
}
```

## Commands

```bash
# Cluster management
snow status                    # Show cluster state
snow deploy <host>             # Deploy to specific host
snow deploy --all              # Deploy all changed hosts

# Health and monitoring
snow health                    # Show health status of all nodes

# etcd cluster operations
snow etcd status               # Show etcd cluster state
snow etcd sync                 # Sync membership from config
snow etcd promote <member>     # Promote learner to voting member

# Future: VM testing
snow vm create <name>          # Create test VM
snow vm deploy <name>          # Deploy to test VM
```

## Why?

Existing NixOS deployment tools (NixOps, deploy-rs, colmena, morph) are push-based and lack:
- **Cluster awareness** - No understanding of cluster membership or distributed state
- **Built-in health checking** - External tools needed (gatus, custom scripts)
- **Agent-based deployment** - Can't pull updates from nodes
- **State management** - No shared state between deployments

snowglobe addresses these gaps with an agent-based, cluster-aware approach.

## Architecture

**Phase 1** (current focus):
- CLI tool (`snow`) for push-based deployments
- etcd for state management
- Replaces: deploy-rs, custom sync scripts, parts of gatus

**Phase 2** (future):
- Agent (`snow-agent`) on each node for pull-based deployments
- Self-healing cluster operations
- Automated member promotion and failure handling

**Phase 3** (future):
- VM support for testing
- Advanced health checks
- Rollback mechanisms

## Origin

Born from managing a NixOS homelab and iteratively building:
- Custom deployment scripts
- etcd cluster management automation
- Health checking with gatus
- Deployment orchestration with just

Eventually realized: "I'm building an orchestrator from scratch, might as well do it properly."

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in snowglobe by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.

## Contributing

Not ready for contributions yet - design phase. Watch this space!
