# snowglobe Design Document

**Status**: Draft
**Last Updated**: 2025-11-03
**Author**: Initial design from homelab experience

---

## Table of Contents

- [Problem Statement](#problem-statement)
- [Existing Solutions](#existing-solutions)
- [Goals and Non-Goals](#goals-and-non-goals)
- [Architecture Overview](#architecture-overview)
- [Phase 1: Push-Based Orchestration](#phase-1-push-based-orchestration)
- [Phase 2: Agent-Based Architecture](#phase-2-agent-based-architecture)
- [Phase 3: VM Support](#phase-3-vm-support)
- [CLI Design](#cli-design)
- [etcd Schema](#etcd-schema)
- [Integration with NixOS](#integration-with-nixos)
- [Open Questions](#open-questions)

---

## Problem Statement

### The Challenge

Managing a NixOS cluster involves orchestrating multiple concerns:

1. **Deployment** - Building and activating NixOS configurations
2. **Cluster state** - Managing cluster membership (etcd, k8s, etc.)
3. **Health monitoring** - Knowing when services are down
4. **Coordination** - Ensuring operations happen in the right order
5. **Testing** - Validating changes before production

Current tools handle these concerns in isolation, requiring custom glue code:

```bash
# Current workflow (from real homelab)
just sync-etcd              # Sync etcd cluster membership
just push-host hydrogen     # Deploy with deploy-rs
just etcd-promote-member    # Manually promote etcd learners
# Check gatus dashboard for health
```

This works, but:
- Multiple tools with different UX patterns
- State scattered across scripts, etcd, and monitoring systems
- Manual coordination required
- No testing story for deployments
- Push-based only (can't pull from nodes)

### Real-World Example: etcd Cluster Management

Managing an etcd cluster with current tools requires:

1. Edit `config/clusters.nix` to add/remove members
2. Run `sync-etcd` script to reconcile cluster membership
3. Deploy new configuration with deploy-rs
4. Manually promote learners with `etcdctl member promote`
5. Check gatus for health
6. Update monitoring config separately

This should be: "Edit config, run `snow deploy`, done."

---

## Existing Solutions

### NixOS Deployment Tools

| Tool | Push/Pull | State | Clustering | Health | Agent |
|------|-----------|-------|------------|--------|-------|
| **NixOps** | Push | Yes (stateful) | No | No | No |
| **deploy-rs** | Push | No | No | No | No |
| **colmena** | Push | No | No | No | No |
| **morph** | Push | No | No | No | No |

All are excellent for basic deployment, but lack cluster awareness.

### Non-NixOS Orchestrators

**Kubernetes**:
- ✅ Cluster-aware, agent-based, self-healing
- ❌ Heavy, complex, overkill for simple clusters
- ❌ Doesn't understand NixOS deployments

**Consul**:
- ✅ Service discovery, health checking, KV store
- ❌ Doesn't handle deployments
- ❌ Would need wrapper to trigger `nixos-rebuild`

**Nomad**:
- ✅ Lightweight orchestration
- ❌ Focused on workloads, not host configuration
- ❌ Doesn't integrate with NixOS

### The Gap

**There is no NixOS-native cluster orchestrator that combines:**
- Deployment automation (like deploy-rs)
- Cluster state management (like etcd operator)
- Health checking (like gatus)
- Agent-based architecture (like Consul)

---

## Goals and Non-Goals

### Goals

**Phase 1 (MVP):**
- ✅ Replace deploy-rs for cluster deployments
- ✅ Integrate cluster state management (etcd, etc.)
- ✅ Provide single CLI for common operations
- ✅ Store cluster state in etcd
- ✅ Show health status of nodes and services

**Phase 2:**
- ✅ Agent-based pull deployments
- ✅ Self-healing cluster operations
- ✅ Automated failure recovery

**Phase 3:**
- ✅ VM-based testing before production deployment
- ✅ Rollback mechanisms
- ✅ Advanced health checking

### Non-Goals

**What we're NOT building:**
- ❌ Full Kubernetes replacement
- ❌ Container orchestration (use k8s for that)
- ❌ CI/CD pipelines (use GitHub Actions, etc.)
- ❌ Configuration management language (use Nix)
- ❌ Monitoring/metrics storage (use existing tools)
- ❌ Secret management (use SOPS, agenix, etc.)

### Success Criteria

Phase 1 is successful if:
1. Can replace `deploy-rs` + `sync-etcd` + parts of `gatus` in real homelab
2. Reduces deployment commands from 3-5 to 1-2
3. Provides better visibility into cluster state
4. Code is maintainable and extensible for future phases

---

## Architecture Overview

### Three-Phase Evolution

```
Phase 1: Push-Based              Phase 2: Agent-Based           Phase 3: VM Testing
┌─────────────┐                  ┌─────────────┐                ┌─────────────┐
│  Developer  │                  │  Developer  │                │  Developer  │
│   Laptop    │                  │   Laptop    │                │   Laptop    │
└──────┬──────┘                  └──────┬──────┘                └──────┬──────┘
       │                                 │                               │
       │ snow deploy                     │ git push                      │ snow vm test
       ▼                                 ▼                               ▼
┌─────────────┐                  ┌─────────────┐                ┌─────────────┐
│    etcd     │◄────────────────►│    etcd     │◄──────────────►│   Test VM   │
│   Cluster   │                  │   Cluster   │                │             │
└──────┬──────┘                  └──────┬──────┘                └──────┬──────┘
       │                                 │                               │
       │ ssh + nixos-rebuild             │ poll state                    │ apply then
       ▼                                 ▼                               ▼ promote
┌─────────────┐                  ┌─────────────┐                ┌─────────────┐
│   Nodes     │                  │   Nodes     │                │  Prod Nodes │
│ (hydrogen,  │                  │   (with     │                │             │
│  nitrogen,  │                  │  snow-agent)│                │             │
│  oxygen)    │                  │             │                │             │
└─────────────┘                  └─────────────┘                └─────────────┘
```

### Core Components

**`snow` (CLI tool)**:
- User-facing command-line interface
- Reads NixOS flake configuration
- Manages etcd state
- Orchestrates deployments
- Shows health and status

**`etcd` (State store)**:
- Single source of truth for cluster state
- Stores: membership, health, deployment status
- Already exists in target clusters
- Provides watch API for agents (Phase 2+)

**`snow-agent` (Phase 2+)**:
- Runs on each NixOS host
- Polls etcd for desired state
- Pulls and applies configurations
- Reports health and status
- Handles local operations (etcd promotion, etc.)

### Data Flow (Phase 1)

```
1. User runs: snow deploy hydrogen

2. snow reads flake.nix to get hydrogen's config

3. snow checks etcd for current state:
   - What generation is currently deployed?
   - Any cluster operations needed (etcd sync)?

4. snow performs cluster operations:
   - Sync etcd membership if needed
   - Add/remove cluster members

5. snow builds configuration:
   - nix build .#nixosConfigurations.hydrogen...

6. snow deploys via SSH:
   - Copy closure to host
   - Activate configuration
   - nixos-rebuild switch

7. snow updates etcd state:
   - Record deployment time, generation, status
   - Update health check timestamp

8. snow reports success/failure to user
```

---

## Phase 1: Push-Based Orchestration

### Scope

Phase 1 focuses on **consolidating existing push-based workflows** into a single tool.

**What we're building:**
- CLI tool that reads NixOS flake configurations
- Integration with etcd for state storage
- Automatic cluster membership management
- Health status reporting
- Single-command deployments

**What we're NOT building yet:**
- Agents on hosts (Phase 2)
- Pull-based deployments (Phase 2)
- VM testing (Phase 3)
- Web UI (maybe never?)

### CLI Commands (Phase 1)

```bash
# Cluster status
snow status                    # Show all nodes and their status
snow status hydrogen           # Show specific node status

# Deployment
snow deploy hydrogen           # Deploy single host
snow deploy --all              # Deploy all changed hosts
snow deploy --changed          # Deploy only hosts with config changes

# Health
snow health                    # Show health of all nodes
snow health --watch            # Watch health status (live updates)

# etcd cluster operations
snow etcd status               # Show etcd cluster members and roles
snow etcd sync                 # Sync membership from config
snow etcd promote <member>     # Promote learner to voting member
snow etcd list-members         # List all members (including learners)

# Configuration
snow config show               # Show current configuration
snow config validate           # Validate flake.nix configuration
```

### Example Usage

**Scenario: Add new node to etcd cluster**

```bash
# 1. Edit config/clusters.nix to add "oxygen"

# 2. Deploy with automatic cluster sync
$ snow deploy oxygen

→ Analyzing configuration changes...
  ✓ Found 1 host to deploy: oxygen

→ Syncing cluster state...
  ! etcd cluster membership differs from config
  + Adding member: oxygen (as learner)
  ✓ etcd membership synced

→ Building configuration for oxygen...
  ✓ Built: /nix/store/abc123-nixos-system-oxygen

→ Deploying to oxygen.i.thepromisedlan.club...
  ✓ Copied closure (245 MB)
  ✓ Activated configuration (generation 42)
  ✓ Services started: etcd.service

→ Updating cluster state...
  ✓ Recorded deployment in etcd

→ Success! oxygen deployed (generation 42)

  Next step: Promote etcd learner to voting member
  $ snow etcd promote oxygen
```

**Scenario: Check cluster status**

```bash
$ snow status

Cluster Status
╭─────────────┬──────────┬──────────┬────────────────────────┬────────────╮
│ Host        │ Status   │ Health   │ Last Deploy            │ Generation │
├─────────────┼──────────┼──────────┼────────────────────────┼────────────┤
│ hydrogen    │ deployed │ healthy  │ 2025-11-03 14:23:11   │ 156        │
│ nitrogen    │ deployed │ healthy  │ 2025-11-03 14:23:15   │ 143        │
│ oxygen      │ deployed │ degraded │ 2025-11-03 14:25:32   │ 42         │
│ sodium      │ pending  │ healthy  │ 2025-11-02 09:15:00   │ 89         │
╰─────────────┴──────────┴──────────┴────────────────────────┴────────────╯

Clusters
╭──────────┬───────────────────────────────┬──────────╮
│ Type     │ Members                       │ Status   │
├──────────┼───────────────────────────────┼──────────┤
│ etcd     │ hydrogen, nitrogen, oxygen    │ healthy  │
│ k8s      │ lanthanum, cerium, ...        │ healthy  │
╰──────────┴───────────────────────────────┴──────────╯

4 hosts, 2 clusters, 1 pending deployment
```

### Implementation Plan

**Language: Rust**

Rationale:
- Type safety for reliable tool
- Great CLI libraries (clap, indicatif)
- Good etcd client support
- Fits NixOS ecosystem (rust toolchain well-supported)
- Fast compilation with cargo

**Key Dependencies:**

```toml
[dependencies]
clap = "4.0"              # CLI argument parsing
etcd-client = "0.12"      # etcd v3 API client
tokio = "1.0"             # Async runtime
serde = "1.0"             # Serialization
serde_json = "1.0"        # JSON handling
anyhow = "1.0"            # Error handling
tabled = "0.14"           # Table formatting
indicatif = "0.17"        # Progress bars
```

**Module Structure:**

```
snow/
├── Cargo.toml
└── src/
    ├── main.rs           # CLI entry point, arg parsing
    ├── commands/         # Command implementations
    │   ├── mod.rs
    │   ├── status.rs     # snow status
    │   ├── deploy.rs     # snow deploy
    │   ├── health.rs     # snow health
    │   └── etcd.rs       # snow etcd *
    ├── lib/
    │   ├── config.rs     # Read flake.nix, parse hosts
    │   ├── etcd.rs       # etcd client wrapper
    │   ├── nix.rs        # nix eval, nix build wrappers
    │   ├── deploy.rs     # Deployment logic (SSH, activate)
    │   ├── cluster.rs    # Cluster operations (sync, etc.)
    │   └── health.rs     # Health checking logic
    └── types/
        ├── host.rs       # Host configuration types
        ├── cluster.rs    # Cluster types
        └── state.rs      # State types (deployment records)
```

### Integration with Existing Flake

**Read-only consumer** (Phase 1):

```nix
# User's flake.nix stays unchanged
{
  nixosConfigurations.hydrogen = {
    # ... existing config
  };
}

# snow reads this directly via:
# nix eval .#nixosConfigurations --apply 'builtins.attrNames'
```

**Optional: snowglobe-specific metadata** (future):

```nix
# User can optionally add snowglobe config
{
  snowglobe = {
    clusters.etcd = {
      nodes = [ "hydrogen" "nitrogen" "oxygen" ];
      auto-promote = false;  # Manual promotion (Phase 1)
    };

    hosts.hydrogen = {
      health-checks = [ "ssh" "etcd" "disk-space" ];
      deploy-timeout = "5m";
    };
  };
}
```

But this is **optional** - snow should work with any NixOS flake.

---

## Phase 2: Agent-Based Architecture

### Vision

Agents running on each host enable:
- **Pull-based deployments** - Nodes pull configs from git/etcd
- **Self-healing** - Agents detect and fix issues
- **Distributed operations** - Operations happen on the nodes themselves
- **Reduced central coordination** - No SSH from deployment machine

### Architecture

```
Developer                etcd Cluster              Nodes
────────────────────────────────────────────────────────────
                              ┌─────┐
git push                      │etcd │
  │                           └──┬──┘
  │                              │
  │                              │ watch /cluster/config
  │                              │
  │                         ┌────▼────┐
  │                         │ Agent   │
  │                         │ (hydro) │
  │                         └────┬────┘
  │                              │
  │                              │ poll & apply
  │                              ▼
  │                         nixos-rebuild switch
  │                              │
  │                              │ report status
  │                              ▼
  │                         update /hosts/hydrogen/status
  │
  └─────────────────────► Optional: trigger via CLI
```

### Agent Behavior

```rust
// Simplified agent logic
loop {
    // 1. Get desired state from etcd
    let desired_generation = etcd.get("/hosts/hydrogen/desired-generation");

    // 2. Compare with current state
    let current_generation = read_current_generation();

    // 3. If mismatch, deploy
    if desired_generation != current_generation {
        deploy_configuration(desired_generation);
        etcd.put("/hosts/hydrogen/status", "deployed");
    }

    // 4. Check if we're part of etcd cluster
    if is_etcd_node() && is_learner() {
        // Auto-promote if configured
        if should_auto_promote() {
            promote_self();
        }
    }

    // 5. Report health
    etcd.put("/hosts/hydrogen/health", check_health());

    // 6. Sleep
    sleep(30s);
}
```

### Phase 2 Scope

**Agent capabilities:**
- Pull configuration from etcd/git
- Apply configuration locally
- Report health and status
- Handle etcd membership (auto-promote)
- Local failure recovery

**New CLI commands:**
```bash
snow agent install <host>      # Install agent on host
snow agent status              # Show all agent statuses
snow agent restart <host>      # Restart agent
```

**Deployment workflow changes:**
```bash
# Phase 1: Push-based
snow deploy hydrogen           # SSHs and deploys directly

# Phase 2: Pull-based (with agent)
snow deploy hydrogen           # Updates etcd, agent does the work
                               # (CLI waits for agent to complete)
```

### Agent Implementation

**Language: Rust** (same as CLI for code sharing)

**Key differences from CLI:**
- Runs as systemd service on each host
- Watches etcd for changes (long-polling)
- Has local state (current generation, etc.)
- Privileged access (runs as root)

**NixOS Module:**

```nix
# nix/module.nix
{ config, lib, pkgs, ... }:
{
  options.services.snow-agent = {
    enable = lib.mkEnableOption "snow agent";

    etcdEndpoints = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      description = "etcd cluster endpoints";
    };

    pollInterval = lib.mkOption {
      type = lib.types.str;
      default = "30s";
    };
  };

  config = lib.mkIf config.services.snow-agent.enable {
    systemd.services.snow-agent = {
      description = "snowglobe cluster agent";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        ExecStart = "${pkgs.snow-agent}/bin/snow-agent";
        Restart = "always";
      };

      environment = {
        SNOW_ETCD_ENDPOINTS = lib.concatStringsSep "," config.services.snow-agent.etcdEndpoints;
        SNOW_POLL_INTERVAL = config.services.snow-agent.pollInterval;
      };
    };
  };
}
```

---

## Phase 3: VM Support

### Vision

Test deployments in VMs before applying to production:

```bash
# Create VM from production config
snow vm create test-hydrogen --profile=hydrogen

# Deploy to VM
snow vm deploy test-hydrogen

# Test it
snow vm ssh test-hydrogen
# ... run tests ...

# If good, deploy to production
snow deploy hydrogen

# Clean up
snow vm destroy test-hydrogen
```

### Use Cases

1. **Testing config changes** - Try before production
2. **Upgrade testing** - Test NixOS version upgrades
3. **Destructive testing** - Break things safely
4. **Learning** - New users can experiment

### Implementation Options

**Option A: NixOS VM built into flake**

```nix
# Use existing nixosConfigurations
# Add VM-specific overrides
```

**Option B: libvirt/QEMU integration**

```bash
# Create VM using existing tools
# snowglobe wraps and manages them
```

**Option C: Cloud VMs** (AWS/GCP/etc.)

```bash
# For more realistic testing
# Costs money, but closer to prod
```

Phase 3 is furthest out, so details are intentionally vague.

---

## CLI Design

### Design Principles

1. **Consistent patterns** - Similar commands work similarly
2. **Progressive disclosure** - Simple tasks are simple, complex tasks are possible
3. **Predictable defaults** - Reasonable defaults for common cases
4. **Composable** - Commands work together via stdin/stdout where appropriate

### Output Formats

**Human-readable by default:**
```bash
$ snow status
Cluster Status
╭─────────────┬──────────┬─────────╮
│ Host        │ Status   │ Health  │
...
```

**Machine-readable with --json:**
```bash
$ snow status --json
{
  "hosts": [
    {"name": "hydrogen", "status": "deployed", "health": "healthy"},
    ...
  ]
}
```

### Progress Indication

Use `indicatif` for long operations:

```bash
$ snow deploy --all
→ Building configurations... ████████████████████ 3/3
→ Deploying hydrogen... ████████████░░░░░░░░ 245/500 MB
```

### Error Handling

**Clear, actionable errors:**

```bash
$ snow deploy oxygen
✗ Error: Cannot connect to etcd cluster
  Tried endpoints:
    - http://hydrogen.i.thepromisedlan.club:2379 (connection refused)
    - http://nitrogen.i.thepromisedlan.club:2379 (connection refused)

  Suggestion: Check that etcd cluster is running

$ snow etcd sync
✗ Error: Cluster has unstarted members

  Member with ID f0a943cc2fc69c8b has no name (likely unstarted)

  To fix:
    1. Check which member is unstarted:
       snow etcd list-members
    2. Remove the unstarted member:
       snow etcd remove-member f0a943cc2fc69c8b
    3. Try again:
       snow etcd sync
```

### Configuration Discovery

**Auto-detect flake location:**

1. Check `SNOW_FLAKE` environment variable
2. Look for `flake.nix` in current directory
3. Walk up directory tree looking for `flake.nix`
4. Fail with helpful error

```bash
$ cd ~/ops
$ snow status              # Uses ~/ops/flake.nix

$ cd ~/ops/hosts
$ snow status              # Still finds ~/ops/flake.nix

$ SNOW_FLAKE=/other/path snow status  # Override
```

---

## etcd Schema

### Design Principles

1. **Hierarchical** - Use directories for grouping
2. **Namespaced** - Prefix with `/snowglobe/` to avoid conflicts
3. **Versioned** - Include schema version for future migrations
4. **Minimal** - Only store what's needed, avoid duplication

### Schema v1

```
/snowglobe/
  /version                           → "1"

  /cluster/
    /etcd/
      /members                       → ["hydrogen", "nitrogen", "oxygen"]
      /leader                        → "hydrogen"
      /auto-promote                  → "false"
    /kubernetes/
      /members                       → ["lanthanum", "cerium", ...]

  /hosts/
    /hydrogen/
      /status                        → "deployed" | "deploying" | "failed" | "pending"
      /health                        → "healthy" | "degraded" | "unhealthy" | "unknown"
      /last-deploy                   → "2025-11-03T14:23:11Z"
      /last-health-check             → "2025-11-03T14:25:00Z"
      /generation                    → "156"
      /profile-hash                  → "abc123..." (hash of configuration)
      /architecture                  → "x86_64-linux"
      /reboot-required               → "false"
      /agent-version                 → "0.1.0" (Phase 2+)
    /nitrogen/
      ...

  /deployments/                      # Deployment history (optional)
    /2025-11-03-142311/
      /id                            → "dep_abc123"
      /targets                       → ["oxygen"]
      /status                        → "success" | "failed" | "in_progress"
      /started                       → "2025-11-03T14:23:11Z"
      /completed                     → "2025-11-03T14:25:32Z"
      /duration                      → "141s"
      /triggered-by                  → "luiz@x1-carbon"
```

### Key Design Decisions

**Why hierarchical?**
- Easy to query all hosts: `GET /snowglobe/hosts/ --prefix`
- Easy to watch a specific host: `WATCH /snowglobe/hosts/hydrogen/`
- Natural grouping of related data

**Why separate `/cluster/` and `/hosts/`?**
- Cluster membership is shared knowledge
- Host state is per-host data
- Clear separation of concerns

**Why store deployment history?**
- Audit trail
- Can analyze deployment patterns
- Useful for debugging "what changed?"

**Why store profile-hash?**
- Detect configuration changes without building
- Can show "drift" (deployed hash != desired hash)
- Enables smarter `snow deploy --changed`

### Operations

**Read all hosts:**
```bash
$ etcdctl get /snowglobe/hosts/ --prefix
```

**Watch for changes to hydrogen:**
```bash
$ etcdctl watch /snowglobe/hosts/hydrogen/ --prefix
```

**Update host status:**
```bash
$ etcdctl put /snowglobe/hosts/hydrogen/status deployed
```

**Get cluster membership:**
```bash
$ etcdctl get /snowglobe/cluster/etcd/members
```

---

## Integration with NixOS

### Reading Flake Configuration

**Get all NixOS configurations:**

```bash
nix eval .#nixosConfigurations --apply 'builtins.attrNames' --json
# => ["hydrogen", "nitrogen", "oxygen", ...]
```

**Get specific host configuration:**

```bash
nix eval .#nixosConfigurations.hydrogen.config.system.build.toplevel
# => /nix/store/abc123-nixos-system-hydrogen-25.05
```

**Get host metadata:**

```bash
# Hostname
nix eval .#nixosConfigurations.hydrogen.config.networking.hostName --raw

# Architecture
nix eval .#nixosConfigurations.hydrogen.config.nixpkgs.system --raw

# IP address (if configured)
nix eval .#nixosConfigurations.hydrogen.config.networking.interfaces.eth0.ipv4.addresses --json
```

**Get cluster configuration** (if using our clusters.nix pattern):

```bash
nix eval .#opsLib.cluster.nodes \
  --apply 'f: builtins.toJSON (f "etcd")' \
  --json
# => ["hydrogen", "nitrogen", "oxygen"]
```

### Building Configurations

```bash
# Build a specific host
nix build .#nixosConfigurations.hydrogen.config.system.build.toplevel

# Build multiple hosts in parallel
nix build \
  .#nixosConfigurations.hydrogen.config.system.build.toplevel \
  .#nixosConfigurations.nitrogen.config.system.build.toplevel \
  --max-jobs 4
```

### Deploying

**Phase 1: SSH-based (like deploy-rs):**

```bash
# Copy closure to host
nix copy --to ssh://hydrogen.i.thepromisedlan.club \
  /nix/store/abc123-nixos-system-hydrogen

# Activate on host
ssh hydrogen.i.thepromisedlan.club \
  "sudo /nix/store/abc123-nixos-system-hydrogen/bin/switch-to-configuration switch"
```

**Phase 2: Agent-based:**

```bash
# Just update etcd, agent handles deployment
etcdctl put /snowglobe/hosts/hydrogen/desired-generation 157
etcdctl put /snowglobe/hosts/hydrogen/desired-profile-hash abc123

# Agent on hydrogen sees change and deploys
```

### snowglobe NixOS Module (Phase 2+)

```nix
# Enable in your configuration
{
  imports = [ inputs.snowglobe.nixosModules.default ];

  services.snow-agent = {
    enable = true;
    etcdEndpoints = [
      "http://hydrogen.i.thepromisedlan.club:2379"
      "http://nitrogen.i.thepromisedlan.club:2379"
      "http://oxygen.i.thepromisedlan.club:2379"
    ];
  };
}
```

---

## Open Questions

### For Phase 1

**Q: How to handle deploy-rs features we're not replicating?**
- Remote builders
- Magic rollback
- Concurrent deployment with dependency ordering

**A:** Start simple, add features as needed. Remote builders can be configured in flake.nix and nix will use them automatically.

**Q: How to handle authentication to etcd?**

**A:** Phase 1: Assume etcd is unauthenticated (internal cluster). Phase 2: Support TLS client certificates.

**Q: What format for health checks?**

**A:** Phase 1: Simple ping/SSH check. Phase 2: Configurable health checks (HTTP endpoints, systemd services, custom scripts).

**Q: How to handle secrets during deployment?**

**A:** Out of scope - continue using SOPS/agenix. snowglobe just deploys, secret management is orthogonal.

**Q: Support for non-NixOS hosts (macOS with nix-darwin)?**

**A:** Yes, but Phase 1 focuses on NixOS. darwin support can come later (mostly same logic, different activation).

### For Phase 2

**Q: How do agents discover etcd cluster?**

**A:** Options:
1. Static configuration in NixOS module
2. DNS SRV records
3. Multicast discovery (probably overkill)

Start with static config (option 1).

**Q: How to handle network partitions?**

**A:** etcd handles this via Raft. Agents should respect etcd consensus and not take action during partitions.

**Q: What happens if agent crashes mid-deployment?**

**A:** systemd restarts it. Agent should be idempotent and pick up where it left off.

### For Phase 3

**Q: VM backend - libvirt, QEMU, cloud?**

**A:** Start with local QEMU (simplest), add others as needed.

**Q: How to map VM networking to production?**

**A:** Use same internal network ranges, separate VLAN/subnet for VMs.

---

## Success Metrics

### Phase 1 Success Criteria

1. **Reduced complexity:**
   - Before: deploy-rs + sync-etcd + gatus + custom scripts
   - After: snow CLI

2. **Fewer commands:**
   - Before: 3-5 commands for typical deployment
   - After: 1-2 commands

3. **Better visibility:**
   - Single command shows complete cluster state
   - Health status integrated

4. **Working in production:**
   - Deployed to real homelab
   - Handles etcd cluster management
   - No regressions vs deploy-rs

5. **Foundation for Phase 2:**
   - Code is modular and testable
   - etcd schema supports future agents
   - Clear path to adding agent support

### Non-Functional Requirements

- **Fast:** Deployments should not be slower than deploy-rs
- **Reliable:** Should not make clusters less stable
- **Debuggable:** Good logging, clear error messages
- **Maintainable:** Code should be easy to understand and modify

---

## Next Steps

### Immediate (Phase 1 MVP)

1. **Set up Rust project**
   - Initialize cargo workspace
   - Add dependencies
   - Set up CI (GitHub Actions)

2. **Implement `snow status`**
   - Read flake.nix
   - Connect to etcd
   - Display current state
   - This validates etcd schema and nix integration

3. **Implement `snow etcd sync`**
   - Port logic from bash script to Rust
   - This validates cluster operations

4. **Implement `snow deploy <host>`**
   - Build configuration
   - Deploy via SSH
   - Update etcd state

5. **Test in real homelab**
   - Use alongside existing deploy-rs
   - Gradually replace deploy-rs workflows
   - Fix issues as they arise

6. **Documentation**
   - User guide
   - Architecture docs
   - Contributing guide

### Future (Phase 2+)

1. Implement agent
2. Add pull-based deployment
3. VM support
4. Advanced features as needed

---

## References

- [etcd documentation](https://etcd.io/docs/)
- [deploy-rs](https://github.com/serokell/deploy-rs)
- [NixOS manual](https://nixos.org/manual/nixos/stable/)
- [Kubernetes Operator pattern](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)
- [Consul architecture](https://www.consul.io/docs/architecture)

---

## Appendix: Learning from the Journey

This design is informed by iteratively building homelab automation:

1. **Started with deploy-rs** - Great for basic deployments
2. **Added custom scripts** - For cluster management (etcd, k8s)
3. **Added gatus** - For health monitoring
4. **Created sync-etcd** - Automated etcd membership
5. **Realized the pattern** - We're building an orchestrator piecemeal

Key lessons:

- **Simple first** - sync-etcd started complex (state files, auto-promotion), simplified over time
- **Fail explicitly** - Better to fail with clear error than handle edge cases poorly
- **Automate the happy path** - Manual intervention for edge cases is fine
- **DRY principle** - Refactor when you find yourself repeating (etcdctl-auto wrapper)
- **Operator pattern works** - Declarative config + reconciliation is the right model

These lessons inform snowglobe's design philosophy.
