# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

snowglobe is a NixOS-native cluster orchestration tool providing push-based deployments with integrated cluster state management. It's currently in Phase 1 (push-based deployments), with future phases planned for agent-based (pull) architecture and VM testing support.

**⚠️ Experimental Project**: This is an early-stage experimental project. Breaking changes to the CLI, library API, and configuration format are expected and acceptable as the design evolves. Don't hesitate to make breaking changes if they improve the architecture or user experience.

**Key characteristics:**
- Rust workspace with CLI (`snow`) and library (`snowlib`)
- SSH-based deployment similar to deploy-rs but with cluster awareness
- Sophisticated reboot detection and handling
- Designed for production homelab use

## Development Commands

### Building and Testing

```bash
# Build the project
cargo build

# Build release version
cargo build --release

# Run tests
cargo test

# Run the CLI (from project root)
cargo run --bin snow -- <command>

# Run from target directory
./target/debug/snow --help
./target/release/snow --help
```

### CLI Commands

```bash
# List all NixOS configurations from a flake
snow list

# Show cluster status
snow status

# Deploy to specific hosts
snow push hydrogen oxygen

# Deploy to all hosts
snow push

# Deploy with options
snow push --dry-run           # Preview changes
snow push --switch            # Force immediate activation
snow push --boot              # Set for next boot only
snow push --no-reboot         # Don't auto-reboot
snow push --quiet             # Suppress subprocess output
snow push --concurrency 5     # Control parallelism
```

### Working with Flakes

The tool expects to work with NixOS flakes. By default, it searches for `flake.nix` in the current directory and parent directories. You can override this with:

```bash
# Via CLI flag
snow -f /path/to/flake push hydrogen

# Via environment variable
export FLAKE_PATH=/path/to/flake
snow push hydrogen
```

## Code Architecture

### Workspace Structure

This is a Cargo workspace with two main crates:

1. **`snow/`** - CLI frontend
   - Argument parsing with clap
   - User-facing commands
   - Terminal output formatting
   - Thin wrapper around snowlib

2. **`snowlib/`** - Core library
   - All business logic
   - No terminal I/O (designed for reuse in agents/TUI)
   - Backend trait system for testability

### Key Architectural Patterns

#### Backend Trait System

The codebase uses dependency injection through backend traits to enable testing and future extensibility:

- **`NixBackend`** - Nix operations (eval, build)
  - `SystemNixBackend` - Real implementation using Nix commands
  - `MockNixBackend` - Test fixture

- **`DeployBackend`** - SSH deployment operations
  - `SshDeployBackend` - Real SSH-based deployment
  - `MockDeployBackend` - Test fixture

- **`RebootBackend`** - Reboot operations
  - `SystemRebootBackend` - Real systemctl reboot via SSH
  - Mock implementation in test fixtures

This pattern allows comprehensive testing without actual Nix evaluation or SSH connections.

#### Push Context (PushContext)

The `PushContext` struct consolidates all deployment state:
- Flake reference
- Deployment configuration
- Backend instances (nix, deploy, reboot)
- Observer for progress reporting

This replaced an earlier scattered approach and centralizes deployment orchestration.

#### Parallel-Per-Host Execution

The push operation uses a parallel-per-host execution model where each host runs through all deployment steps independently:

1. **Build** - Build NixOS configuration for the host
2. **Copy** - Copy closure to target host
3. **Planning** - Create deployment plan with reboot detection
4. **Activate** - Run activation script on host
5. **Reboot** - Reboot host if required

This is implemented in `execute_phase()` which uses Rayon to run `execute_host_pipeline()` concurrently for each host. Dynamic step spans (managed by the observer) provide visibility into which steps hosts are currently in.

The advantage of this approach is that fast hosts don't wait for slow hosts to complete each step - each host progresses through the pipeline at its own pace.

### Core Concepts

#### Flake

The `Flake` struct represents a NixOS flake and provides methods to:
- Discover flake location (search upward from cwd)
- List all `nixosConfigurations`
- Evaluate host configurations
- Build system closures

#### Reboot Detection

Sophisticated two-phase detection system:

**Phase 1: Fast Symlink Check**
- Compares `/run/booted-system/{kernel,initrd,init}` with new system
- Catches most reboot-requiring changes

**Phase 2: Detailed Service Check**
- Runs `dry-activate` to see what would change
- Checks if critical services would restart (audit, udev, etc.)
- Uses machine-readable output from `/run/nixos/dry-activation-restart-list`

See `docs/push.md` for full details on reboot policies and activation modes.

#### Reboot Policies

Hosts can configure per-host reboot policies in NixOS config:

```nix
config.core.rebootPolicy = "auto";  # or "never" or "always"
```

- **auto** - Detect if reboot needed, reboot only when necessary (default)
- **never** - Never auto-reboot, always use boot mode
- **always** - Always reboot after deployment

Policies override command-line flags for safety.

#### Observer Pattern

The `PushObserver` trait allows progress reporting to be decoupled from deployment logic:
- CLI uses it for real-time progress display
- Tests use it to verify correct behavior
- Future TUI/web interfaces can implement custom observers

### Important Naming Conventions

Recent refactorings established these conventions:

- `flake` - Instance of `Flake` struct (not `ctx` or `snow_ctx`)
- `_backend` - Backend trait instances (not `_ops`)
- `PushContext` - Consolidated deployment state (replaces scattered parameters)

### Module Organization

```
snowlib/src/
├── lib.rs              # Public API exports
├── error.rs            # Error types using thiserror
├── observer.rs         # Observer trait for progress reporting
├── nix/                # Nix integration
│   ├── flake.rs        # Flake discovery and operations
│   ├── types.rs        # Domain types (StorePath, etc.)
│   └── command.rs      # Nix command wrapper
├── backends/           # Backend traits and implementations
│   ├── nix.rs          # NixBackend trait
│   ├── deploy/         # DeployBackend trait and SSH impl
│   ├── reboot.rs       # RebootBackend trait
│   └── mock.rs         # Test fixtures
└── push/               # Push command implementation
    ├── mod.rs          # Main push orchestration (includes deployment planning)
    ├── config.rs       # Configuration types
    ├── context.rs      # PushContext
    └── results.rs      # Result types
```

## Design Philosophy & Patterns

### From docs/context.md

The project emerged from real homelab pain points and embodies these principles:

1. **Declarative + Reconciliation** - Desired state in config → tool makes it reality
2. **Simple First** - Build for the happy path, handle edge cases explicitly
3. **Fail Explicitly** - Clear errors with recovery steps over automatic edge case handling
4. **Progressive Enhancement** - Phase 1 (push) → Phase 2 (agents) → Phase 3 (VMs)
5. **Smart Defaults** - `snow push` should "do the right thing" automatically

### Error Handling

- Use `miette` for user-facing errors with context and suggestions
- Use `thiserror` for library error types
- Provide actionable error messages with recovery steps
- Fail explicitly on edge cases rather than trying to handle everything

### Testing Strategy

- Unit tests for individual functions
- Integration tests using mock backends
- CLI tests using `assert_cmd` and `predicates`
- Real-world testing against actual NixOS flake (user's ops repo)

## Context Files

For deep understanding of design decisions and project evolution:

- **docs/design.md** - Original comprehensive design document (may be outdated)
- **docs/context.md** - Complete project history, all decisions and rationale
- **docs/push.md** - Current push command documentation (up-to-date)
- **README.md** - Project overview and vision

Note: docs/design.md was written before implementation began and may not reflect current architecture. docs/context.md and docs/push.md are more reliable for understanding current state.

## Git Workflow

Recent commits show active refactoring focused on:
- Consolidating state into PushContext
- Renaming for consistency (ctx → flake, _ops → _backend)
- Minimizing public API surface
- Extracting domain types and modules

This is a small, focused project with clean commit history. Keep commits atomic and well-described.

## NixOS Integration

The tool works by:

1. **Discovery**: Finding flake.nix in current directory or parents
2. **Evaluation**: Using `nix eval` to inspect configurations
3. **Building**: Using `nix build` to create system closures
4. **Deployment**: Using `nix copy` to send closures over SSH
5. **Activation**: Running `/nix/store/.../bin/switch-to-configuration` on target

### Common Nix Eval Patterns

```bash
# List all nixosConfigurations
nix eval .#nixosConfigurations --apply 'builtins.attrNames' --json

# Get system path for a host
nix eval .#nixosConfigurations.hostname.config.system.build.toplevel --raw

# Build a configuration
nix build .#nixosConfigurations.hostname.config.system.build.toplevel
```

## Future Phases

### Phase 2: Agent-Based (Not Yet Implemented)

- `snow-agent` binary running on each host
- Pull-based deployment (hosts poll etcd for desired state)
- Self-healing cluster operations
- NixOS module for agent configuration

### Phase 3: VM Testing (Not Yet Implemented)

- Test deployments in VMs before production
- `snow vm create/deploy/test` commands
- Safe testing of configuration changes

## Common Patterns

### Creating a New Command

1. Add module in `snow/src/commands/`
2. Define args struct with clap derives
3. Implement `run(&self, flake: &Flake) -> Result<()>`
4. Add to `Commands` enum in `snow/src/commands/mod.rs`
5. Add tests in `snow/tests/cli_tests.rs`

### Adding a Backend Operation

1. Add method to appropriate backend trait in `snowlib/src/backends/`
2. Implement for real backend (e.g., `SshDeployBackend`)
3. Implement for mock backend with configurable fixture
4. Use in deployment steps via PushContext

### Working with the Observer

When adding new deployment operations or modifying the pipeline:

1. Call observer methods at appropriate points:
   - `on_phase_start()` / `on_phase_complete()` for phase lifecycle
   - `on_build_start()` / `on_build_complete()` for build operations
   - `on_copy_start()` / `on_copy_complete()` for copy operations
   - `on_deploy_start()` / `on_deploy_complete()` for activation
   - `on_reboot_start()` / `on_reboot_complete()` for reboot operations
2. Add tracing events for progress visibility (`tracing::info!`, `tracing::error!`)
3. Mock observer in tests to verify callbacks
4. CLI observer manages dynamic step spans automatically
