{ config, lib, ... }:

# Platform-agnostic hook configuration module
# This module is imported by both nixos.nix and darwin.nix

with lib;

let
  hookSubmodule = types.submodule {
    options = {
      location = mkOption {
        type = types.enum [ "local" "remote" ];
        description = ''
          Where to execute the hook:
          - "local": Execute on the machine running `snow push`
          - "remote": Execute on the target host via SSH
        '';
        example = "local";
      };

      command = mkOption {
        type = types.str;
        description = ''
          Command to execute. Can reference NixOS/nix-darwin config values
          using standard Nix string interpolation.

          Environment variables available during execution:
          - SNOW_HOSTNAME: Target hostname
          - SNOW_PHASE: Deployment phase (if configured)
          - SNOW_STORE_PATH: Path to the new system in /nix/store
        '';
        example = "kubectl drain \${config.networking.hostName}";
      };

      timeout = mkOption {
        type = types.ints.positive;
        default = 300;
        description = ''
          Maximum time (in seconds) to wait for the hook to complete.
          If the hook exceeds this timeout, it will be terminated and
          treated as a failure (unless continueOnFailure is true).
        '';
      };

      continueOnFailure = mkOption {
        type = types.bool;
        default = false;
        description = ''
          Whether to continue the deployment if this hook fails.

          - false (default): Hook failure aborts the deployment for this host
          - true: Hook failure is logged but deployment continues

          Useful for cleanup hooks that should always run (e.g., uncordoning
          nodes even if activation failed).
        '';
      };
    };
  };
in
{
  options.snowglobe.hooks = {
    pre-activate = mkOption {
      type = types.listOf hookSubmodule;
      default = [];
      description = ''
        Hooks to run after copying the new system but before activation.

        Common use cases:
        - Drain cluster nodes (Kubernetes, Nomad)
        - Remove from load balancer pool
        - Stop accepting new connections
        - Gracefully leave cluster (Consul, etcd)

        These hooks run before the system is activated, so the old
        configuration is still running.
      '';
      example = literalExpression ''
        [
          {
            location = "local";
            command = "kubectl drain --ignore-daemonsets \${config.networking.hostName}";
            timeout = 360;
            continueOnFailure = false;
          }
        ]
      '';
    };

    post-activate = mkOption {
      type = types.listOf hookSubmodule;
      default = [];
      description = ''
        Hooks to run after activation completes, before any reboot.

        Common use cases:
        - Uncordon cluster nodes
        - Add back to load balancer pool
        - Verify deployment success
        - Update service discovery

        These hooks run after the new configuration is activated but
        before any required reboot.
      '';
      example = literalExpression ''
        [
          {
            location = "local";
            command = "kubectl uncordon \${config.networking.hostName}";
            timeout = 60;
            continueOnFailure = true;
          }
        ]
      '';
    };

    post-reboot = mkOption {
      type = types.listOf hookSubmodule;
      default = [];
      description = ''
        Hooks to run after the host reboots (only if a reboot occurred).

        Common use cases:
        - Verify services started correctly
        - Run post-boot health checks
        - Re-enable monitoring
        - Update external state

        These hooks only run if the deployment triggered a reboot.
        They run after the host comes back online.
      '';
      example = literalExpression ''
        [
          {
            location = "remote";
            command = "systemctl is-system-running --wait";
            timeout = 120;
            continueOnFailure = false;
          }
        ]
      '';
    };

    on-failure = mkOption {
      type = types.listOf hookSubmodule;
      default = [];
      description = ''
        Hooks to run if the deployment fails at any stage.

        Common use cases:
        - Rollback external state changes
        - Send failure notifications
        - Restore cluster membership
        - Emergency uncordon/drain cleanup

        These hooks run if any deployment step fails (build, copy,
        activation, or other hooks). Useful for cleanup and rollback.
      '';
      example = literalExpression ''
        [
          {
            location = "local";
            command = "kubectl uncordon \${config.networking.hostName}";
            timeout = 60;
            continueOnFailure = true;
          }
          {
            location = "local";
            command = "notify-send 'Deployment failed for \${config.networking.hostName}'";
            timeout = 10;
            continueOnFailure = true;
          }
        ]
      '';
    };
  };
}
