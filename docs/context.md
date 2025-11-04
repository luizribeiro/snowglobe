# snowglobe: Project Context and History

**Document Purpose**: This document provides comprehensive context for continuing development of snowglobe. It captures the journey, decisions, and rationale that led to this project.

**Last Updated**: 2025-11-03
**Current Status**: Initial design phase, no code yet

---

## Table of Contents

- [Origin Story](#origin-story)
- [The etcd Automation Journey](#the-etcd-automation-journey)
- [Key Design Decisions](#key-design-decisions)
- [Design Philosophy](#design-philosophy)
- [Current State](#current-state)
- [What We've Built](#what-weve-built)
- [Next Steps](#next-steps)
- [Important Technical Details](#important-technical-details)

---

## Origin Story

### The Question That Started It All

> "Right now when adding/removing nodes to our etcd cluster I have to use `etcdctl member add/remove`. Is there any way to avoid that?"

The user was managing a NixOS homelab with:
- Multiple hosts (`hydrogen`, `nitrogen`, `oxygen`, `sodium`, `boron`, etc.)
- etcd cluster for distributed state
- Kubernetes cluster
- Various other clustered services (MySQL, LDAP, Gluster, etc.)
- Configuration in `config/clusters.nix` (declarative)
- But cluster membership management was **imperative** (manual `etcdctl` commands)

### The Gap

The tension: **Declarative configuration with imperative operations**.

```nix
# config/clusters.nix - Declarative!
etcd = {
  nodes = [ "hydrogen" "nitrogen" "oxygen" ];
};
```

But to actually change membership:
```bash
# Imperative! Manual! Error-prone!
etcdctl member add oxygen --peer-urls=http://oxygen.i.thepromisedlan.club:2380
# Deploy oxygen
etcdctl member promote <ID>
```

### The Realization

After building automated etcd cluster management (see below), we realized:

> "I'm already doing cluster orchestration here. What if we expand this to handle all NixOS cluster operations?"

This led to snowglobe.

---

## The etcd Automation Journey

Before snowglobe existed, we built automated etcd cluster management. This work became the foundation for snowglobe's design.

### What We Built (in ops repo)

**Files created:**
1. `scripts/sync-etcd` - Reconciliation script for etcd membership
2. `scripts/etcdctl-auto` - Auto-discovering etcd endpoint wrapper
3. `.justfile` updates - Integrated sync into deployment workflow
4. `features/linux/etcd.nix` updates - Fixed tmpfiles race condition

**Git commits in ops repo:**
- `87165d10` - "Add automatic etcd cluster membership management"
- `bea6f8cd` - "Remove hardcoded etcd endpoints via auto-discovery wrapper"
- `6349cdd2` - "Make learner promotion explicit and manual"

### How sync-etcd Works

**Core concept**: Reconciliation loop (operator pattern!)

```bash
#!/usr/bin/env bash
# Simplified logic

# 1. Get desired state from Nix
desired=$(nix eval .#opsLib.cluster.nodes --apply 'f: f "etcd"')

# 2. Get actual state from etcd
current=$(etcdctl member list -w json | jq '.members[].name')

# 3. Calculate diff
to_add=$(comm -13 <(echo "$current") <(echo "$desired"))
to_remove=$(comm -23 <(echo "$current") <(echo "$desired"))

# 4. Reconcile
for member in $to_add; do
    etcdctl member add "$member" --learner
done

for member in $to_remove; do
    etcdctl member remove "$member"
done
```

**Integration**: Runs automatically before every deployment:

```bash
# .justfile
push-host target:
    just sync-etcd        # ← Automatic!
    deploy .#{{target}}
```

### Key Insights From This Work

#### 1. The Operator Pattern Works

We initially called this "executing operations" but learned:

**Operator = Software that operates the cluster like a human operator would**

This is the core pattern of Kubernetes operators, and it worked beautifully for etcd management.

#### 2. Simplicity > Cleverness

**Initial attempt**: Complex state tracking, auto-promotion, edge case handling
**Final version**: Fail explicitly on edge cases with clear recovery instructions

Example:
```bash
# When encountering unnamed/unstarted members:
✗ Error: Cluster has unstarted members

  To fix:
    1. Check: just etcd-list-members
    2. Remove: just etcd-remove-member <ID>
    3. Retry: just sync-etcd
```

Better to provide clear manual steps than try to handle every edge case automatically.

#### 3. DRY Violations Should Be Fixed

We had hardcoded etcd endpoints everywhere:
```bash
ETCD_ENDPOINT="http://hydrogen.i.thepromisedlan.club:2379"
```

User feedback: "Can we get rid of the hardcoded values?"

Solution: Created `etcdctl-auto` wrapper that discovers healthy endpoint from config:

```bash
#!/usr/bin/env bash
# Discovers healthy endpoint from cluster config
nodes=$(nix eval .#opsLib.cluster.nodes --apply 'f: f "etcd"')
for node in $nodes; do
    endpoint="http://${node}.i.thepromisedlan.club:2379"
    if etcdctl --endpoints="$endpoint" endpoint health >/dev/null 2>&1; then
        exec etcdctl --endpoints="$endpoint" "$@"
        break
    fi
done
```

Now every script can just use `etcdctl-auto` instead of managing endpoints.

#### 4. Ordering Matters

**Problem**: Auto-promotion couldn't work because:
1. `sync-etcd` runs (tries to add + promote member)
2. But host hasn't deployed yet, so etcd service isn't running
3. THEN deployment happens
4. Node joins as learner but promotion already failed

**Solution**: Make promotion explicit and manual:
```bash
# After deploying
just etcd-promote-member oxygen
```

Future: Agent running on the node could auto-promote (Phase 2 idea).

#### 5. Declarative + Reconciliation = Powerful

The pattern of "desired state in Nix → reconciliation script → actual state" proved incredibly robust:
- Idempotent (safe to run multiple times)
- Self-correcting (detects and fixes drift)
- Easy to reason about
- Integrates cleanly with deployment

### Edge Cases We Handled

**1. Unstarted/unnamed members:**
- Symptom: Member added but etcd service never started
- Result: Member has ID but no name in `etcdctl member list`
- Solution: Fail explicitly, provide manual cleanup commands

**2. Last voting member protection:**
- Problem: Can't remove last voting member (breaks quorum)
- Solution: Check voting member count before removal

**3. Cluster health checks:**
- Problem: Don't want to modify cluster when it's unhealthy
- Solution: Check health before making any changes

**4. Stale data from failed deployments:**
- Problem: Previous failed deployment left orphaned data
- Solution: Manual cleanup + redeploy (document the process)

### What We Learned About etcd

**etcd member lifecycle:**
```
[Add as learner] → [Start etcd service] → [Promote to voting]
     ↓                    ↓                      ↓
  Has ID only      Has ID + name          Full member
  (unstarted)       (learner)             (voting)
```

**Critical insight**: Operations on members require different approaches:
- Can find **voting/learners** by name
- Can only find **unstarted** by ID (from peer URL)
- Member IDs from API are decimal, but `etcdctl` wants hex

**Conversion needed:**
```bash
member_id_dec=$(etcdctl member list -w json | jq '.members[].ID')
member_id_hex=$(printf "%x" "$member_id_dec")
etcdctl member remove "$member_id_hex"
```

### Testing Process

We tested the complete add/remove cycle with `boron`:

**Test 1: Add boron to cluster**
```bash
# Add to config/clusters.nix
# Run: just push-host boron
# Result: ✓ Added as learner, deployed successfully
```

**Test 2: Remove boron from cluster**
```bash
# Remove from config/clusters.nix
# Run: just push-host boron
# Result: ✓ Removed from etcd, etcd service stopped, user/group removed
```

**Test 3: Edge cases**
- Unstarted members → Explicit failure with clear instructions ✓
- Last member protection → Prevented accidental removal ✓
- Unhealthy cluster → Refused to make changes ✓

### The Aha Moment

After completing the etcd automation, we realized:

**We just built an operator for etcd cluster management.**

And then:

> "What if we built a proper orchestrator for **all** NixOS cluster operations?"

This is where snowglobe was born.

---

## Key Design Decisions

### 1. Separate Repository

**Decision**: Create `snowglobe` as separate repo from day 1, not as part of ops

**Rationale:**
- Clean git history (no private info to scrub)
- Can open source immediately
- Forces good practices (no hardcoded internal IPs)
- Others can contribute from start
- Can still test against ops via `inputs.snowglobe.url = "path:..."`

**Alternative considered**: Develop in `ops/snowglobe/` and extract later
**Why rejected**: Risk of committing private info, harder to share WIP, messy extraction

### 2. Three-Phase Approach

**Decision**: Build in phases, each providing value

**Phase 1: Push-Based Orchestration** (MVP)
- CLI tool (`snow`)
- Reads existing flake.nix (no new config format)
- Push-based deployment (like deploy-rs)
- etcd for state
- Replaces: deploy-rs + sync-etcd + parts of gatus

**Phase 2: Agent-Based** (future)
- Agents on each host (`snow-agent`)
- Pull-based deployment
- Self-healing operations
- Distributed cluster management

**Phase 3: VM Support** (future)
- Test deployments in VMs
- `snow vm create/deploy/test`

**Rationale:**
- Each phase delivers value independently
- Can stop at any phase if effort not worth it
- Validates ideas before over-investing
- Users can adopt incrementally

**Alternative considered**: Build everything at once
**Why rejected**: Too much risk, too much complexity, no early validation

### 3. Rust for Implementation

**Decision**: Use Rust for both CLI and agent

**Rationale:**
- Type safety for reliable tool
- Great CLI ecosystem (clap, indicatif, tabled)
- Good etcd client support
- Fits NixOS ecosystem (well-supported in nixpkgs)
- Can share code between CLI and agent
- Fast compilation with cargo

**Alternatives considered:**
- **Go**: etcd is written in Go, great networking libraries
  - **Why not**: Rust better fits NixOS ecosystem
- **Bash**: What we're using now, fast to iterate
  - **Why not**: Hard to maintain as it grows, poor testing story

### 4. Read-Only Config Consumer (Phase 1)

**Decision**: Don't introduce new config format, read existing flake.nix

**Rationale:**
- Works with any NixOS flake (not snowglobe-specific)
- No migration needed for existing setups
- Lower adoption barrier
- Can add optional snowglobe config later if needed

**How it works:**
```bash
# Get all hosts
nix eval .#nixosConfigurations --apply 'builtins.attrNames'

# Get host config
nix eval .#nixosConfigurations.hydrogen.config.system.build.toplevel
```

**Future enhancement** (optional):
```nix
# Can add snowglobe-specific metadata if desired
{
  snowglobe.hosts.hydrogen = {
    health-checks = [ "etcd" "ssh" "disk-space" ];
  };
}
```

But this is **optional**, not required.

### 5. etcd Schema Design

**Decision**: Hierarchical schema under `/snowglobe/` prefix

```
/snowglobe/
  /version                    → Schema version
  /cluster/                   → Shared cluster state
    /etcd/members             → ["hydrogen", "nitrogen", "oxygen"]
  /hosts/                     → Per-host state
    /hydrogen/
      /status                 → "deployed" | "deploying" | "failed"
      /health                 → "healthy" | "degraded" | "unhealthy"
      /last-deploy            → "2025-11-03T14:23:11Z"
      /generation             → "156"
  /deployments/               → History (optional)
```

**Rationale:**
- Hierarchical = easy to query with `--prefix`
- Namespaced = no conflicts with other etcd users
- Versioned = can migrate schema later
- Minimal = only store what's needed

**Design patterns:**
- Use directories for grouping
- Store timestamps as ISO 8601
- Use enums for status fields
- Separate cluster-wide state from host-specific state

### 6. Fail Explicitly on Edge Cases

**Decision**: Don't try to handle every edge case automatically

**Rationale:**
- Simpler code
- Easier to understand what went wrong
- Safer (less chance of automated tool making it worse)
- Manual intervention for rare cases is acceptable

**Pattern:**
```rust
// Instead of:
if edge_case {
    try_to_fix_automatically() // Complex, fragile
}

// Do:
if edge_case {
    return Err("Clear explanation + manual recovery steps")
}
```

**Example from sync-etcd:**
```bash
✗ Error: Cluster has unstarted members

  This usually means a previous deployment failed.

  To fix:
    1. List members: just etcd-list-members
    2. Remove unstarted: just etcd-remove-member <ID>
    3. Retry: just sync-etcd
```

### 7. CLI Design Principles

**Decisions:**
1. **Human-readable by default**, `--json` for machines
2. **Progressive disclosure** - simple tasks simple, complex tasks possible
3. **Consistent patterns** - similar commands work similarly
4. **Clear errors** - actionable error messages with suggestions
5. **Progress indication** - show progress for long operations

**Examples:**

```bash
# Human-readable
$ snow status
Cluster Status
╭─────────────┬──────────┬─────────╮
│ Host        │ Status   │ Health  │
...

# Machine-readable
$ snow status --json
{"hosts": [{"name": "hydrogen", ...}]}

# Good error
$ snow deploy oxygen
✗ Error: Cannot connect to etcd
  Tried: hydrogen:2379, nitrogen:2379
  Suggestion: Check etcd cluster is running

# Progress
$ snow deploy --all
→ Building configurations... ████████████████████ 3/3
```

---

## Design Philosophy

These principles emerged from building etcd automation and inform snowglobe's design:

### 1. Declarative Configuration + Reconciliation

**Pattern**: Desired state in version control → Reconciliation loop → Actual state

This is the operator pattern, and it's incredibly powerful:
- Source of truth is in git
- Tool reads desired state and makes it reality
- Idempotent (can run multiple times safely)
- Self-correcting (fixes drift)

### 2. Simple First, Then Optimize

Don't build complex solutions for problems you don't have yet.

**Journey of sync-etcd:**
1. **Initial**: Complex state file, auto-promotion, intricate edge case handling
2. **Simplified**: No state file, manual promotion, explicit failures
3. **Result**: More reliable, easier to understand, still solves 95% of cases

**Lesson**: Build for the happy path, handle edge cases as they arise.

### 3. DRY (Don't Repeat Yourself)

When you find yourself copying code, stop and abstract it.

**Example**: Hardcoded etcd endpoints everywhere
**Solution**: `etcdctl-auto` wrapper used by all scripts

**Key insight**: The right abstraction emerges after you've written something 2-3 times. Don't abstract too early!

### 4. Progressive Enhancement

Build features incrementally, each adding value:

**snowglobe phases:**
- Phase 1: Better deploy-rs (push-based, integrated sync)
- Phase 2: Add agents (pull-based)
- Phase 3: Add VMs (testing)

Each phase stands alone. Can stop anywhere and still have value.

### 5. Clear Communication

Tools should communicate clearly what they're doing and why they failed:

**Bad:**
```
Error: operation failed
```

**Good:**
```
✗ Error: Cannot remove last voting member

  etcd requires at least one voting member for quorum.

  To remove hydrogen:
    1. Add another member first
    2. Ensure it's promoted to voting member
    3. Then remove hydrogen
```

### 6. Leverage Existing Tools

Don't rebuild what already works:

**snowglobe will use:**
- etcd for state (not building new distributed store)
- Nix for config (not new config language)
- SSH for communication (Phase 1)
- systemd for agent (Phase 2)
- SOPS/agenix for secrets (not new secret management)

**Only build what's missing**: The orchestration layer that ties it together.

### 7. Open Source From Day 1

Benefits:
- Forces good practices (no secrets, generic examples)
- Can get early feedback
- Others can contribute
- Helps the NixOS community

**How**: Separate repo, public GitHub, clear docs, MIT license

---

## Current State

### What Exists

**snowglobe repository** (`/home/luiz/snowglobe`):

```
snowglobe/
├── README.md              ✓ Written
├── docs/
│   ├── design.md          ✓ Comprehensive design doc
│   └── context.md         ✓ This document
├── snow/                  ○ Directory created, no code yet
│   └── src/
├── snow-agent/            ○ Directory created (Phase 2)
└── nix/                   ○ Directory created
    └── module.nix         (Phase 2)
```

**Git status:**
- Initialized repository
- One commit: `4739cb9` "Initial snowglobe design and project structure"
- Not pushed to GitHub yet

**ops repository** (user's private infra):

The etcd automation that inspired snowglobe:
- `scripts/sync-etcd` - Working etcd membership reconciliation
- `scripts/etcdctl-auto` - Working endpoint discovery
- `.justfile` - Integrated sync into deployment workflow
- `features/linux/etcd.nix` - Working etcd configuration

### What Doesn't Exist Yet

**No code written for snowglobe:**
- No Rust project initialized
- No `snow` CLI implementation
- No etcd client code
- No deployment logic
- No tests

**Documentation complete, implementation pending.**

### Current Decision Points

**Decisions made:**
- ✅ Separate repository
- ✅ Three-phase approach
- ✅ Rust implementation
- ✅ Read existing flake.nix
- ✅ etcd schema designed
- ✅ CLI commands designed
- ✅ Phase 1 scope defined

**Open questions:**
- ❓ License (leaning toward MIT)
- ❓ GitHub org or personal account?
- ❓ Announce on NixOS Discourse now or wait for MVP?
- ❓ CI setup (GitHub Actions configuration)

---

## What We've Built

### In ops Repository (Completed)

**1. scripts/sync-etcd** - etcd membership reconciliation
- Reads desired state from `config/clusters.nix`
- Queries current state from etcd
- Adds missing members (as learners)
- Removes extra members (with safety checks)
- Validates cluster health
- Fails explicitly on edge cases

**2. scripts/etcdctl-auto** - Endpoint discovery wrapper
- Discovers healthy etcd endpoint from config
- Tries each node until finding one that responds
- Passes through all arguments to etcdctl
- Eliminates hardcoded endpoints

**3. .justfile integration**
- `just sync-etcd` - Sync cluster membership
- `just etcd-list-members` - List all members
- `just etcd-remove-member <id>` - Remove member by ID
- `just etcd-promote-member <name>` - Promote learner to voting
- Automatic sync before `just push-host` and `just push-all`

**4. features/linux/etcd.nix fixes**
- tmpfiles rule for `/var/lib/etcd`
- `preStart` hook to ensure directory exists
- Proper permissions (0700, etcd:etcd)

**Commits:**
- `87165d10` - Add automatic etcd cluster membership management
- `bea6f8cd` - Remove hardcoded etcd endpoints via auto-discovery wrapper
- `6349cdd2` - Make learner promotion explicit and manual

**Status**: ✅ Complete, tested, working in production

### In snowglobe Repository (In Progress)

**1. README.md** - Project overview
- Vision and goals
- Command examples (planned)
- Architecture overview
- Development status

**2. docs/design.md** - Comprehensive design document
- Problem statement and motivation
- Existing solutions analysis
- Goals and non-goals (all phases)
- Architecture overview with diagrams
- CLI design with examples
- etcd schema (v1)
- Implementation plan (language, dependencies, modules)
- Integration with NixOS
- Success criteria

**3. docs/context.md** - This document
- Complete project history
- All design decisions with rationale
- Design philosophy
- Technical details

**4. Directory structure**
- Basic repo structure created
- Placeholder directories for code

**Status**: 📝 Design phase complete, ready for implementation

---

## Next Steps

### Immediate Actions

**1. Create GitHub Repository**
```bash
cd /home/luiz/snowglobe
gh repo create snowglobe --public --source=. --remote=origin
git push -u origin main
```

**2. Add License**
```bash
# Add MIT license (or choose another)
curl -o LICENSE https://raw.githubusercontent.com/licenses/license-templates/master/templates/mit.txt
# Edit with your name
git add LICENSE && git commit -m "Add MIT license"
```

**3. Initialize Rust Project**
```bash
cd snow
cargo init --name snow
```

**4. Add Dependencies**
```toml
# snow/Cargo.toml
[dependencies]
clap = { version = "4.0", features = ["derive"] }
etcd-client = "0.12"
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
tabled = "0.14"
indicatif = "0.17"
```

### Phase 1 MVP Implementation Order

**Week 1-2: Foundation**

1. **Set up Rust project structure**
   - Initialize cargo workspace
   - Add dependencies
   - Set up basic CLI arg parsing with clap
   - Create module structure

2. **Implement nix integration** (`src/lib/nix.rs`)
   - Function to discover flake.nix
   - Function to list all nixosConfigurations
   - Function to get host configuration
   - Function to build configuration

3. **Implement etcd client** (`src/lib/etcd.rs`)
   - Connect to etcd cluster
   - Read/write operations
   - Implement schema (as defined in design.md)
   - Helper functions for common operations

**Week 3-4: First Command**

4. **Implement `snow status`** (`src/commands/status.rs`)
   - Read hosts from flake.nix
   - Query etcd for each host's state
   - Format output as table (tabled crate)
   - Handle --json flag

   **Why this first?**
   - Validates all integrations (nix + etcd)
   - Useful immediately (can see cluster state)
   - No destructive operations (safe to test)
   - Fast feedback loop

5. **Test with ops repository**
   ```nix
   # In ops/flake.nix, temporarily add:
   {
     inputs.snowglobe.url = "path:/home/luiz/snowglobe";
   }
   ```

   Run: `nix run .#snowglobe.packages.x86_64-linux.snow -- status`

**Week 5-6: Cluster Operations**

6. **Implement `snow etcd sync`** (`src/commands/etcd.rs`)
   - Port logic from `scripts/sync-etcd`
   - Get desired members from nix eval
   - Get current members from etcd
   - Calculate diff and reconcile
   - Much of this can be ported from bash script

7. **Implement other etcd commands**
   - `snow etcd status` - Show cluster state
   - `snow etcd list-members` - List all members
   - `snow etcd promote <member>` - Promote learner

**Week 7-10: Deployment**

8. **Implement deployment** (`src/lib/deploy.rs`)
   - Build configuration with nix
   - Copy closure to host via SSH
   - Activate configuration
   - Handle errors and rollback

9. **Implement `snow deploy <host>`** (`src/commands/deploy.rs`)
   - Check etcd for current state
   - Run cluster sync if needed
   - Build configuration
   - Deploy to host
   - Update etcd with new state
   - Show progress with indicatif

10. **Implement `snow deploy --all`**
    - Determine which hosts have changes
    - Deploy in parallel (with configurable concurrency)
    - Aggregate results

**Week 11-12: Polish**

11. **Add health checking** (`src/commands/health.rs`)
    - Simple checks (SSH, ping)
    - Service status checks
    - Integrate with `snow status`

12. **Testing and Documentation**
    - Write unit tests
    - Integration tests with ops repo
    - Update README with usage examples
    - Write user guide (docs/usage.md)

13. **Replace deploy-rs in ops repo**
    - Update `.justfile` to use `snow` instead of `deploy`
    - Test thoroughly
    - Keep deploy-rs as backup initially

### Success Criteria for Phase 1

Phase 1 is complete when:

1. ✅ Can run `snow status` and see all hosts
2. ✅ Can run `snow etcd sync` to manage cluster membership
3. ✅ Can run `snow deploy hydrogen` to deploy a single host
4. ✅ Can run `snow deploy --all` to deploy all changed hosts
5. ✅ ops repository uses `snow` instead of deploy-rs + sync-etcd
6. ✅ No regressions vs current deployment workflow
7. ✅ Documentation is complete
8. ✅ Tests are passing
9. ✅ Open source on GitHub with clean commit history

### Beyond Phase 1

**Phase 2 Planning:**
- Agent design document
- NixOS module for snow-agent
- Agent implementation
- Pull-based deployment
- Self-healing capabilities

**Phase 3 Planning:**
- VM backend selection
- Testing workflow design
- Integration with CI

---

## Important Technical Details

### Network Architecture (ops repository)

**Internal domain**: `.i.thepromisedlan.club`

**IPv4 to IPv6 mapping**: `10.x.y.z` → `fd42:dead:beef:x:y::/64`

**Active subnets:**
- `10.0.0.0/24` - WireGuard VPN (hydrogen, nitrogen, oxygen)
- `10.1.1.0/24` - Core network (servers, infrastructure)
- `10.1.2.0/24` - IoT/Services (VLAN 90)
- `10.1.3.0/24` - WiFi clients (VLAN 20)

**Key hosts:**
- `hydrogen`, `nitrogen`, `oxygen` - etcd cluster nodes
- `sodium`, `boron` - Other infrastructure
- `lanthanum`, `cerium`, etc. - Kubernetes nodes

### ops Repository Structure

```
ops/
├── flake.nix              # Main flake, defines all hosts
├── config/
│   └── clusters.nix       # Cluster definitions (etcd, k8s, mysql, etc.)
├── hosts/                 # Per-host configurations
│   ├── x86_64-linux/
│   │   ├── hydrogen/
│   │   ├── nitrogen/
│   │   └── oxygen/
│   └── aarch64-darwin/
├── features/              # Reusable modules
│   └── linux/
│       └── etcd.nix       # etcd configuration
├── scripts/
│   ├── sync-etcd          # etcd reconciliation
│   └── etcdctl-auto       # Endpoint discovery
└── .justfile              # Task runner with deployment commands
```

### etcd Configuration Pattern

```nix
# config/clusters.nix
{
  etcd = {
    # Cluster membership is automatically synced by `just sync-etcd`
    nodes = [ "hydrogen" "nitrogen" "oxygen" ];
  };
}

# Access in Nix:
nix eval .#opsLib.cluster.nodes --apply 'f: f "etcd"'
# => ["hydrogen" "nitrogen" "oxygen"]
```

### etcdctl Command Patterns

**Member list:**
```bash
etcdctl member list -w json
# Returns: {members: [{ID: 123, name: "hydrogen", ...}]}
```

**Add member:**
```bash
etcdctl member add oxygen \
  --peer-urls="http://oxygen.i.thepromisedlan.club:2380" \
  --learner
```

**Promote member:**
```bash
# Need to convert ID from decimal to hex
member_id_dec=$(etcdctl member list -w json | jq -r '.members[] | select(.name == "oxygen") | .ID')
member_id_hex=$(printf "%x" "$member_id_dec")
etcdctl member promote "$member_id_hex"
```

**Health check:**
```bash
etcdctl endpoint health
# Returns: endpoint is healthy/unhealthy
```

### NixOS Deployment Pattern

**Build:**
```bash
nix build .#nixosConfigurations.hydrogen.config.system.build.toplevel
# Result: /nix/store/abc123-nixos-system-hydrogen
```

**Deploy via SSH:**
```bash
# Copy closure
nix copy --to ssh://hydrogen.i.thepromisedlan.club \
  /nix/store/abc123-nixos-system-hydrogen

# Activate
ssh hydrogen.i.thepromisedlan.club \
  "sudo /nix/store/abc123-nixos-system-hydrogen/bin/switch-to-configuration switch"
```

**Get current generation:**
```bash
ssh hydrogen.i.thepromisedlan.club \
  "readlink /run/current-system | cut -d- -f2"
```

### Useful Nix Eval Patterns

**List all nixosConfigurations:**
```bash
nix eval .#nixosConfigurations --apply 'builtins.attrNames' --json
```

**Get host attribute:**
```bash
# Hostname
nix eval .#nixosConfigurations.hydrogen.config.networking.hostName --raw

# Architecture
nix eval .#nixosConfigurations.hydrogen.config.nixpkgs.system --raw

# Any config value
nix eval .#nixosConfigurations.hydrogen.config.PATH.TO.OPTION
```

**Check if host has feature:**
```bash
nix eval .#nixosConfigurations.hydrogen.config.features.etcd.enable
```

### Testing Pattern

**Safe testing approach:**

1. Test command against ops repo
2. Use `--dry-run` where applicable
3. Test on non-critical host first
4. Keep old tools available as backup
5. Incremental adoption

**Example workflow:**
```bash
# Terminal 1: Development
cd ~/snowglobe
cargo build

# Terminal 2: Testing
cd ~/ops
nix run ~/snowglobe#snow -- status
nix run ~/snowglobe#snow -- deploy boron  # Non-critical test host
```

### Error Handling Patterns

From sync-etcd, patterns that work well:

**Health checks before operations:**
```bash
if ! etcdctl endpoint health >/dev/null 2>&1; then
    echo "✗ etcd cluster is not healthy"
    exit 1
fi
```

**Validation before destructive operations:**
```bash
# Check if last voting member
voting_count=$(etcdctl member list -w json \
    | jq '[.members[] | select(.isLearner != true)] | length')
if [[ "$voting_count" -le 1 ]]; then
    echo "✗ Cannot remove last voting member"
    exit 1
fi
```

**Clear output with colors:**
```bash
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}✓${NC} Success message"
echo -e "${RED}✗${NC} Error message"
echo -e "${YELLOW}⚠${NC} Warning message"
```

### Performance Considerations

**etcd query optimization:**
- Batch queries where possible
- Use prefix queries for related keys
- Cache cluster membership (changes rarely)

**Nix evaluation optimization:**
- Evaluate once, use results multiple times
- Use `--apply` to extract just what's needed
- Consider caching eval results for `--all` operations

**Deployment parallelization:**
- Build all configurations in parallel first
- Deploy to independent hosts in parallel
- Respect dependencies (etcd leader last, etc.)

---

## Communication Guidelines

### User's Preferences

**From observing interactions:**

1. **Honest takes**: User values honest assessment over agreement
   - Example: "Is this really the way to go? Give me your honest take."
   - Response should be genuine analysis of trade-offs

2. **Simplicity**: Prefers simple solutions over complex ones
   - User chose Option A (fail explicitly) over complex edge case handling
   - "Yeah I like option A" - after being presented with simple vs complex

3. **Avoiding repetition**: Points out when code is being repeated
   - "We're repeating ourselves quite a bit. Can you abstract this away?"
   - Values DRY principle

4. **Understanding decisions**: Wants to understand "why"
   - "Even if we retried, it wouldn't work. Right? Because..."
   - Thinks through implications before committing

5. **Pragmatic**: Balances ideal vs practical
   - Chose manual promotion (simple) over auto-promotion (complex)
   - Plans future improvements but doesn't over-engineer now

### Communication Style That Works

**Good:**
- Present options with clear trade-offs
- Be honest about limitations
- Explain reasoning behind suggestions
- Ask clarifying questions when ambiguous
- Acknowledge when something could be done better

**Avoid:**
- Over-complicating simple problems
- Assuming you know best without discussion
- Building features not requested
- Glossing over edge cases or limitations

### Example Interaction Pattern

```
User: [Asks question about approach]

Claude: [Brief direct answer]
        [2-3 options with clear trade-offs]
        [Honest recommendation with reasoning]
        [Ask which approach user prefers]

User: [Chooses option, possibly with follow-up]

Claude: [Implement chosen approach]
        [Note any implications or future considerations]
```

---

## Appendix: Code Patterns to Reuse

### From sync-etcd (bash → Rust)

**Pattern: Discover etcd endpoint**

Bash version:
```bash
for node in $nodes; do
    endpoint="http://${node}.i.thepromisedlan.club:2379"
    if etcdctl --endpoints="$endpoint" endpoint health >/dev/null 2>&1; then
        echo "$endpoint"
        return 0
    fi
done
```

Rust version (sketch):
```rust
async fn discover_etcd_endpoint(nodes: &[String]) -> Result<String> {
    for node in nodes {
        let endpoint = format!("http://{}.i.thepromisedlan.club:2379", node);
        if check_etcd_health(&endpoint).await.is_ok() {
            return Ok(endpoint);
        }
    }
    bail!("Could not find healthy etcd endpoint")
}
```

**Pattern: Get desired cluster members**

Bash version:
```bash
nix eval --raw "$REPO_ROOT#opsLib.cluster.nodes" \
    --apply 'f: builtins.toJSON (f "etcd")' 2>/dev/null \
    | jq -r '.[]'
```

Rust version (sketch):
```rust
fn get_desired_members(cluster_type: &str) -> Result<Vec<String>> {
    let output = Command::new("nix")
        .args(&["eval", "--raw", ".#opsLib.cluster.nodes"])
        .arg("--apply")
        .arg(format!("f: builtins.toJSON (f \"{}\")", cluster_type))
        .output()?;

    let members: Vec<String> = serde_json::from_slice(&output.stdout)?;
    Ok(members)
}
```

**Pattern: Reconciliation**

Bash version:
```bash
to_add=$(comm -13 <(echo "$current") <(echo "$desired"))
to_remove=$(comm -23 <(echo "$current") <(echo "$desired"))
```

Rust version (sketch):
```rust
fn calculate_diff(current: &HashSet<String>, desired: &HashSet<String>)
    -> (Vec<String>, Vec<String>)
{
    let to_add: Vec<_> = desired.difference(current).cloned().collect();
    let to_remove: Vec<_> = current.difference(desired).cloned().collect();
    (to_add, to_remove)
}
```

### Table Formatting (with tabled)

```rust
use tabled::{Table, Tabled};

#[derive(Tabled)]
struct HostStatus {
    #[tabled(rename = "Host")]
    name: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Health")]
    health: String,
    #[tabled(rename = "Last Deploy")]
    last_deploy: String,
    #[tabled(rename = "Generation")]
    generation: u64,
}

let statuses = vec![/* ... */];
let table = Table::new(statuses).to_string();
println!("{}", table);
```

### Progress Bars (with indicatif)

```rust
use indicatif::{ProgressBar, ProgressStyle};

let pb = ProgressBar::new(total);
pb.set_style(
    ProgressStyle::default_bar()
        .template("{msg} [{bar:40}] {pos}/{len}")
        .unwrap()
);

for item in items {
    pb.set_message(format!("Processing {}", item));
    // Do work
    pb.inc(1);
}

pb.finish_with_message("Complete!");
```

---

## Final Notes

### What Makes snowglobe Different

This isn't just "yet another deployment tool". It's:

1. **Cluster-aware**: Understands distributed services (etcd, k8s, etc.)
2. **State-backed**: Uses etcd as single source of truth
3. **Integrated**: Deployment + cluster management + health in one tool
4. **Agent-capable**: Can evolve from push to pull architecture
5. **NixOS-native**: Built specifically for NixOS, not adapted from other platforms

### Why This Might Succeed

1. **Real need**: Born from actual pain points, not theoretical problems
2. **Proven patterns**: Operator pattern works (see: Kubernetes ecosystem)
3. **Incremental value**: Each phase provides value independently
4. **Right scope**: Not trying to be Kubernetes, just better NixOS clustering
5. **Timing**: NixOS growing in popularity, infrastructure-as-code mainstream

### Why This Might Fail

1. **Niche market**: Only useful for multi-host NixOS deployments
2. **Maintenance burden**: Maintaining distributed systems tools is hard
3. **Competition**: deploy-rs is good enough for many users
4. **Complexity**: Might end up being "too clever" despite best intentions

### How to Avoid Failure

1. **Start small**: Phase 1 should be genuinely simpler than current tools
2. **Real users first**: Must work for ops repo before generalizing
3. **Stay focused**: Resist feature creep, stick to core mission
4. **Community**: Get feedback early, incorporate others' needs
5. **Know when to stop**: If Phase 1 doesn't provide value, don't do Phase 2

---

## Questions for Next Session

When continuing this work, consider:

1. **Scope questions:**
   - Should Phase 1 support darwin (nix-darwin) or focus on NixOS only?
   - How much deploy-rs compatibility do we need?
   - What's minimum viable health checking?

2. **Implementation questions:**
   - Should we use tokio for async or keep it sync (simpler)?
   - How to structure error types (anyhow vs thiserror)?
   - Testing strategy (unit tests, integration tests, manual tests)?

3. **UX questions:**
   - Default behavior for `snow deploy` (deploy all changed? require args?)?
   - How verbose should output be by default?
   - Configuration file or environment variables for settings?

4. **Community questions:**
   - When to announce on NixOS Discourse?
   - How to get early feedback?
   - Should we seek co-maintainers early?

---

**End of Context Document**

This document should provide comprehensive context for continuing snowglobe development. All key decisions, rationale, technical details, and lessons learned are captured here.

For implementation details, see `docs/design.md`.
For project overview, see `README.md`.

Good luck! 🚀
