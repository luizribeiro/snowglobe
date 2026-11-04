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
- **Secure activation helper** - Passwordless deployment without compromising security
- **Reboot detection** - Automatically handle system updates requiring reboots
- **Agent-based deployment** (planned) - Nodes manage themselves (pull-based)
- **Built-in health checking** (planned) - Know when things break
- **etcd-backed state** (planned) - Distributed state management
- **VM testing** (planned) - Test deployments safely before production

Think: A modern, security-focused alternative to `deploy-rs` with cluster awareness.

## Features

### Secure Passwordless Deployment

Snowglobe includes NixOS and nix-darwin modules that provide secure passwordless system activation:

```nix
{
  imports = [ inputs.snowglobe.nixosModules.default ];

  snowglobe.push.allowedUsers = [ "deploy-user" ];
}
```

This installs the `snowglobe-activate` command that:
- Works around sudoers wildcard limitations on macOS
- Validates all inputs before execution
- Provides a stable path for sudo configuration
- Is more secure than `NOPASSWD: ALL`

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

See [docs/design.md](docs/design.md) for detailed design.

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

## Development

This project is in early design phase. See:
- [docs/design.md](docs/design.md) - Full design document (user-facing)
- [docs/context.md](docs/context.md) - Complete development context and history
- [docs/architecture.md](docs/architecture.md) - Architecture details (TBD)
- [docs/etcd-schema.md](docs/etcd-schema.md) - etcd data model (TBD)

**For developers/contributors**: Start with [docs/context.md](docs/context.md) to understand the full journey, all design decisions, and rationale behind this project.

## Origin

Born from managing a NixOS homelab and iteratively building:
- Custom deployment scripts
- etcd cluster management automation
- Health checking with gatus
- Deployment orchestration with just

Eventually realized: "I'm building an orchestrator from scratch, might as well do it properly."

## License

MIT (to be confirmed)

## Contributing

Not ready for contributions yet - design phase. Watch this space!
