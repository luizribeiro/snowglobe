# snowglobe

Self-contained cluster orchestration for NixOS.

> **Status**: Early design phase. Not ready for use.

## Vision

snowglobe is a NixOS-native cluster orchestration tool that provides:
- **Declarative cluster configuration** - Define your cluster in Nix
- **Agent-based deployment** - Nodes manage themselves (pull-based)
- **Built-in health checking** - Know when things break
- **etcd-backed state** - Distributed state management
- **VM testing** - Test deployments safely before production

Think: `deploy-rs` + `gatus` + custom orchestration scripts, redesigned as a cohesive system.

## Commands (Planned)

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
