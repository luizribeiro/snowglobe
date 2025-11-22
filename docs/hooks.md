# Lifecycle Hooks

Lifecycle hooks enable you to execute custom commands at specific points in the deployment pipeline, allowing for sophisticated orchestration like draining Kubernetes nodes, coordinating with service discovery systems, or performing custom validation.

## Overview

Hooks are configured in your NixOS configuration and execute automatically during deployment:

```
Build → Copy → [pre-activate hooks] → Activate → [post-activate hooks] → Reboot → [post-reboot hooks]
                                                ↓ (on any failure)
                                           [on-failure hooks]
```

### Key Features

- **Declarative Configuration**: Define hooks in your NixOS config alongside your system definition
- **Per-Host Execution**: Hooks run individually for each host during parallel deployments
- **Local and Remote**: Execute commands on the machine running `snow` or on the target host via SSH
- **Reliable Error Handling**: Built-in timeout, retry logic, and failure modes
- **Environment Variables**: Access deployment metadata (hostname, phase, store path)

## Configuration

Hooks are configured under `config.snowglobe.hooks` in your NixOS configuration:

```nix
{
  config.snowglobe.hooks = {
    pre-activate = {
      drain-node = {
        location = "local";  # or "remote"
        command = "kubectl drain ${config.networking.hostName}";
        timeout = 300;  # seconds (default: 300)
        continueOnFailure = false;  # default: false
      };
    };
    post-activate = { /* ... */ };
    post-reboot = { /* ... */ };
    on-failure = { /* ... */ };
  };
}
```

### Hook Stages

#### `pre-activate`

Runs **after copying** the new system but **before activation**.

**Common use cases:**
- Drain cluster nodes (Kubernetes, Nomad)
- Remove from load balancer pool
- Stop accepting new connections
- Gracefully leave cluster (Consul, etcd)

**Example:**
```nix
pre-activate = {
  drain-node = {
    location = "local";
    command = "kubectl drain --delete-emptydir-data --ignore-daemonsets --timeout=5m ${config.networking.hostName}";
    timeout = 360;
    continueOnFailure = false;
  };
};
```

#### `post-activate`

Runs **after activation completes**, before any reboot.

**Common use cases:**
- Uncordon cluster nodes
- Add back to load balancer pool
- Verify deployment success
- Update service discovery

**Example:**
```nix
post-activate = {
  uncordon-node = {
    location = "local";
    command = "kubectl uncordon ${config.networking.hostName}";
    timeout = 60;
    continueOnFailure = true;  # Always uncordon, even if activation had issues
  };
};
```

#### `post-reboot`

Runs **after the host reboots** and comes back online (only if a reboot actually occurred).

**Common use cases:**
- Wait for services to be ready
- Verify node is healthy
- Rejoin cluster after reboot
- Run post-boot validation

**Example:**
```nix
post-reboot = {
  wait-node-ready = {
    location = "local";
    command = "kubectl wait --for=condition=Ready node/${config.networking.hostName} --timeout=5m";
    timeout = 360;
    continueOnFailure = false;
  };
};
```

#### `on-failure`

Runs **when deployment or reboot fails** at any stage.

**Common use cases:**
- Cleanup after failed deployment
- Send alerts or notifications
- Rollback operations
- Log failure details

**Example:**
```nix
on-failure = {
  send-alert = {
    location = "local";
    command = "curl -X POST https://alerts.example.com/webhook -d '{\"host\": \"${config.networking.hostName}\", \"status\": \"failed\"}'";
    timeout = 30;
    continueOnFailure = true;  # Don't fail the failure handler
  };
};
```

## Execution Location

### Local Execution (`location = "local"`)

Executes on the **machine running `snow`** (your workstation, CI/CD runner, etc.).

**Use for:**
- Cluster orchestration commands (`kubectl`, `consul`, `nomad`)
- API calls to external services
- Commands that need credentials not on the target host

**Example:**
```nix
{
  location = "local";
  command = "kubectl drain ${config.networking.hostName}";
}
```

### Remote Execution (`location = "remote"`)

Executes on the **target host via SSH**.

**Use for:**
- Host-specific validation
- Local service checks
- Commands that must run on the target system

**Example:**
```nix
{
  location = "remote";
  command = "systemctl is-active docker";
}
```

## Error Handling

### `continueOnFailure`

Controls what happens when a hook fails (non-zero exit code or timeout).

**`continueOnFailure = false` (default):**
- Hook failure **aborts the deployment** for that host
- Other hosts in the phase continue deploying
- `on-failure` hooks are executed

**`continueOnFailure = true`:**
- Hook failure is **logged as a warning**
- Deployment continues normally
- `on-failure` hooks are **not** executed

**When to use `continueOnFailure = true`:**
- Cleanup operations (like uncordoning nodes)
- Best-effort operations (like sending notifications)
- Non-critical validation

**Example:**
```nix
post-activate = {
  uncordon-node = {
    location = "local";
    command = "kubectl uncordon ${config.networking.hostName}";
    continueOnFailure = true;  # Always uncordon, even if activation failed
  };
};
```

### Timeout

Each hook has a timeout (default: 300 seconds). If the command doesn't complete within the timeout, it's terminated and treated as a failure.

**Timeout behavior:**
1. When a hook exceeds its timeout, `snow` sends `SIGTERM` to gracefully terminate the process
2. If the process doesn't exit within 5 seconds, `SIGKILL` is sent to force termination
3. `snow` waits up to 2 seconds for the process to be reaped by the kernel
4. The hook is marked as failed and any output captured up to the timeout is included in error messages

```nix
{
  command = "kubectl drain ${config.networking.hostName}";
  timeout = 600;  # 10 minutes for slow drain
}
```

**Notes:**
- For hooks that perform cleanup or have side effects, ensure they handle `SIGTERM` gracefully to avoid leaving resources in an inconsistent state
- In rare cases (zombie processes or uninterruptible sleep states), a process may not be killable; `snow` will log an error but continue with the deployment

## Environment Variables

### Default Environment

Hooks receive these environment variables automatically:

| Variable | Description | Example |
|----------|-------------|---------|
| `SNOW_HOSTNAME` | Target hostname | `web01` |
| `SNOW_PHASE` | Deployment phase | `production` |
| `SNOW_STORE_PATH` | Nix store path being deployed | `/nix/store/...` |

Plus any environment variables from your shell or CI/CD workflow (e.g., `KUBECONFIG`).

### Custom Environment Variables

You can set additional environment variables for hooks using `environment` and `environmentFiles`:

```nix
{
  notify-deploy = {
    location = "local";
    command = ./scripts/notify-slack.sh;
    environment = {
      SLACK_CHANNEL = "#deploys";
    };
    environmentFiles = [
      config.age.secrets.slack-webhook.path
    ];
  };
}
```

**`environment`**: Attrset of environment variables to set:
```nix
environment = {
  KUBECONFIG = "/home/user/.kube/config";
  LOG_LEVEL = "debug";
};
```

**`environmentFiles`**: List of files containing environment variables (KEY=VALUE format, one per line). Useful for secrets:
```nix
environmentFiles = [
  config.age.secrets.deploy-tokens.path
  /run/secrets/api-keys
];
```

Environment files are loaded first, then `environment` variables are applied (allowing explicit variables to override file contents).

## Variable Substitution

Hooks support Nix string interpolation for host-specific values:

```nix
{
  command = "kubectl drain ${config.networking.hostName}";
  #                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^
  #                        Substituted during Nix evaluation
}
```

This is evaluated when building the configuration, so the command stored in metadata contains the actual hostname.

## Complete Examples

### Kubernetes Cluster

Zero-downtime rolling updates for Kubernetes nodes:

```nix
# In features/linux/kubernetes.nix
{ config, lib, ... }:

{
  config = lib.mkIf config.features.kubernetes.enable {
    snowglobe.hooks = {
      pre-activate = {
        drain-node = {
          location = "local";
          command = "kubectl drain --delete-emptydir-data --ignore-daemonsets --timeout=5m ${config.networking.hostName}";
          timeout = 360;
          continueOnFailure = false;
        };
      };

      post-activate = {
        uncordon-node = {
          location = "local";
          command = "kubectl uncordon ${config.networking.hostName}";
          timeout = 60;
          continueOnFailure = true;  # Always uncordon
        };
      };

      post-reboot = {
        wait-node-ready = {
          location = "local";
          command = "kubectl wait --for=condition=Ready node/${config.networking.hostName} --timeout=5m";
          timeout = 360;
          continueOnFailure = false;
        };
      };

      on-failure = {
        uncordon-node = {
          location = "local";
          command = "kubectl uncordon ${config.networking.hostName}";
          timeout = 60;
          continueOnFailure = true;  # Cleanup: uncordon even on failure
        };
      };
    };
  };
}
```

### Consul Cluster

Gracefully leave and rejoin Consul cluster:

```nix
{
  snowglobe.hooks = {
    pre-activate = {
      leave-cluster = {
        location = "remote";
        command = "consul leave";
        timeout = 30;
        continueOnFailure = true;  # Leave is best-effort
      };
    };

    post-reboot = {
      wait-cluster-rejoin = {
        location = "remote";
        command = "until consul members | grep ${config.networking.hostName}; do sleep 2; done";
        timeout = 120;
        continueOnFailure = false;
      };
    };
  };
}
```

### Load Balancer Integration

Remove from and add back to load balancer:

```nix
{
  snowglobe.hooks = {
    pre-activate = {
      remove-from-pool = {
        location = "local";
        command = "curl -X POST https://lb.example.com/api/pool/remove -d '{\"host\": \"${config.networking.hostName}\"}'";
        timeout = 30;
        continueOnFailure = false;
      };
      drain-connections = {
        location = "local";
        command = "sleep 10";  # Wait for connections to drain
        timeout = 15;
        continueOnFailure = false;
      };
    };

    post-activate = {
      add-to-pool = {
        location = "local";
        command = "curl -X POST https://lb.example.com/api/pool/add -d '{\"host\": \"${config.networking.hostName}\"}'";
        timeout = 30;
        continueOnFailure = false;
      };
    };

    on-failure = {
      add-to-pool = {
        location = "local";
        command = "curl -X POST https://lb.example.com/api/pool/add -d '{\"host\": \"${config.networking.hostName}\"}'";
        timeout = 30;
        continueOnFailure = true;  # Always re-add to pool
      };
    };
  };
}
```

### Custom Validation

Validate deployment before considering it successful:

```nix
{
  snowglobe.hooks = {
    post-activate = {
      check-services = {
        location = "remote";
        command = "systemctl is-active nginx postgresql";
        timeout = 30;
        continueOnFailure = false;  # Fail deployment if services aren't running
      };
    };

    post-reboot = {
      check-health = {
        location = "remote";
        command = "curl --fail http://localhost:8080/health";
        timeout = 60;
        continueOnFailure = false;  # Fail if health check fails
      };
    };
  };
}
```

### Notifications and Alerts

Send notifications during deployment using environment files for secrets:

```nix
{ pkgs, config, ... }:

let
  notifySlack = pkgs.writeShellScript "notify-slack.sh" ''
    #!/usr/bin/env bash
    set -euo pipefail

    MESSAGE="$1"

    curl -X POST https://slack.com/api/chat.postMessage \
      -H "Authorization: Bearer $SLACK_WEBHOOK_TOKEN" \
      -H "Content-Type: application/json" \
      -d "{
        \"channel\": \"$SLACK_CHANNEL\",
        \"text\": \"$MESSAGE\"
      }"
  '';
in
{
  snowglobe.hooks = {
    pre-activate = {
      notify-start = {
        location = "local";
        command = "${notifySlack} 'Starting deployment to ${config.networking.hostName}'";
        environment = {
          SLACK_CHANNEL = "#deploys";
        };
        environmentFiles = [
          config.age.secrets.slack-webhook.path  # Contains SLACK_WEBHOOK_TOKEN=...
        ];
        timeout = 10;
        continueOnFailure = true;  # Don't fail deployment if notification fails
      };
    };

    on-failure = {
      notify-failure = {
        location = "local";
        command = "${notifySlack} 'Deployment failed for ${config.networking.hostName}'";
        environment = {
          SLACK_CHANNEL = "#alerts";
        };
        environmentFiles = [
          config.age.secrets.slack-webhook.path
        ];
        timeout = 30;
        continueOnFailure = true;
      };
    };
  };
}
```

## Best Practices

### 1. Use Appropriate `continueOnFailure` Settings

```nix
# Critical pre-deployment checks: fail fast
pre-activate = {
  check-health = {
    command = "check-cluster-health";
    continueOnFailure = false;
  };
};

# Cleanup operations: always run
post-activate = {
  uncordon-node = {
    command = "kubectl uncordon ${config.networking.hostName}";
    continueOnFailure = true;
  };
};
```

### 2. Set Realistic Timeouts

```nix
# Quick health checks
{ command = "curl localhost:8080/health"; timeout = 10; }

# Slow operations like draining
{ command = "kubectl drain ..."; timeout = 600; }
```

### 3. Use on-failure for Cleanup

Always uncordon nodes or re-add to pools even if deployment fails:

```nix
on-failure = {
  uncordon-node = {
    location = "local";
    command = "kubectl uncordon ${config.networking.hostName}";
    continueOnFailure = true;
  };
};
```

### 4. Organize Hooks in Feature Modules

Define hooks alongside the feature they support:

```nix
# features/linux/kubernetes.nix
{ config, lib, ... }:
{
  config = lib.mkIf config.features.kubernetes.enable {
    snowglobe.hooks = {
      pre-activate = {
        drain-node = { /* kubectl drain */ };
      };
      post-activate = {
        uncordon-node = { /* kubectl uncordon */ };
      };
    };
  };
}
```

### 5. Test Hooks Before Production

Test hooks on a single host before rolling out to the cluster:

```bash
# Deploy to a single test host first
snow push test-host

# Then deploy to the full phase
snow push --phase production
```

### 6. Use Remote Hooks Sparingly

Prefer local execution for cluster orchestration. Use remote hooks only for host-specific operations:

```nix
# Good: Cluster orchestration from local
{ location = "local"; command = "kubectl drain ..."; }

# Good: Host-specific check on remote
{ location = "remote"; command = "systemctl is-active docker"; }
```

### 7. Make Hooks Idempotent

Hooks may be retried, so ensure they can run multiple times safely:

```bash
# Bad: Fails if already drained
kubectl drain node

# Good: Succeeds even if already drained
kubectl drain node || kubectl get node | grep SchedulingDisabled
```

## Troubleshooting

### Hook Failures

When a hook fails, you'll see:

```
ERROR Hook failed for web01 (exit code: 1): error message from stderr
```

**Debug steps:**
1. Check the hook command runs successfully manually
2. Verify environment variables are available (`KUBECONFIG`, etc.)
3. Increase timeout if the command is slow
4. Check SSH connectivity for remote hooks

### Hooks Not Executing

**Verify hooks are configured:**
```bash
# Check host metadata includes hooks
snow list -e 'c: c.snowglobe.hooks or {}'
```

**Common issues:**
- Hooks configured in wrong location (must be under `config.snowglobe.hooks`)
- Typo in stage name (must be `pre-activate`, not `preActivate`)
- Missing `continueOnFailure` field (defaults to `false`)

### SSH Issues with Remote Hooks

Remote hooks use SSH to connect to the target host. Ensure:
- SSH key authentication is configured
- The user has permission to run the command
- The command is in the `PATH` on the remote system

## Migration from External Wrappers

If you currently use wrapper scripts around `snow push`:

**Before (justfile wrapper):**
```bash
push-k8s:
  kubectl drain node1 node2 node3
  snow push --hosts node1,node2,node3
  kubectl uncordon node1 node2 node3
```

**After (hooks in NixOS config):**
```nix
# features/linux/kubernetes.nix
config.snowglobe.hooks = {
  pre-activate = {
    drain-node = { location = "local"; command = "kubectl drain ${config.networking.hostName}"; };
  };
  post-activate = {
    uncordon-node = { location = "local"; command = "kubectl uncordon ${config.networking.hostName}"; };
  };
};
```

```bash
# Just deploy directly
snow push --hosts node1,node2,node3
```

**Benefits:**
- Proper per-host orchestration (rolling updates)
- Better error handling
- Integrated logging
- No external wrapper scripts needed

## Migration Guide

### Breaking Change: Named Hooks (Attrsets)

Hooks were changed from lists to named attrsets to improve debugging and clarity. Each hook now requires a unique name.

**Before (lists):**
```nix
snowglobe.hooks = {
  pre-activate = [
    {
      location = "local";
      command = "kubectl drain ${config.networking.hostName}";
      timeout = 360;
    }
  ];
};
```

**After (named attrsets):**
```nix
snowglobe.hooks = {
  pre-activate = {
    drain-node = {  # ← Hook now has a name
      location = "local";
      command = "kubectl drain ${config.networking.hostName}";
      timeout = 360;
    };
  };
};
```

**Why this change?**
- Hook names appear in logs, making debugging easier
- Better organization when you have multiple hooks per stage
- Clearer configuration structure

**Migration steps:**
1. Wrap each hook list in an attrset
2. Give each hook a descriptive kebab-case name
3. Use names that describe what the hook does (e.g., `drain-node`, `check-health`, `notify-deploy`)

Hook names appear in all log messages:
```
INFO  Executing PreActivate hook 'drain-node' for web01: kubectl drain...
INFO  Hook 'drain-node' completed successfully for web01 in 12.3s
ERROR Hook 'uncordon-node' failed for web01 (exit code: 1): ...
```

## Logging and Observability

Hooks integrate with snowglobe's logging via `tracing`:

```
INFO  hooks: Executing PreActivate hook for web01: kubectl drain web01
INFO  hooks: Hook completed successfully for web01 in 12.3s
```

**Hook lifecycle logging:**
- Hook start with command
- Hook completion with duration
- Hook failure with exit code and stderr
- Warnings for hooks with `continueOnFailure = true`

Set `RUST_LOG=hooks=debug` for verbose hook debugging.

## FAQ

**Q: Can I use hooks to modify the system before building?**
A: No, hooks run during deployment, not during build. Use NixOS configuration for system modifications.

**Q: Do hooks run in parallel?**
A: Hooks for different hosts run in parallel (same as deployment). Hooks for the same host run sequentially in the order defined.

**Q: Can hooks access secrets?**
A: Yes, via environment variables or files on the local machine (for local hooks) or target host (for remote hooks).

**Q: What happens if a hook times out?**
A: The hook is killed and treated as a failure (unless `continueOnFailure = true`).

**Q: Can I skip hooks for a specific deployment?**
A: Not currently. Hooks are part of the host configuration and always execute.

**Q: Do hooks work with `--dry-run`?**
A: No, hooks do not execute during dry-run mode.
