{ config, lib, ... }:

# Platform-agnostic hook configuration module
# This module is imported by both nixos.nix and darwin.nix

with lib;

let
  hookSubmodule = types.submodule ({ name, ... }: {
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

          Additional environment variables can be set via the `environment`
          and `environmentFiles` options.
        '';
        example = "kubectl drain \${config.networking.hostName}";
      };

      environment = mkOption {
        type = types.attrsOf types.str;
        default = {};
        description = ''
          Environment variables to set when executing the hook.

          These are merged with the default Snow environment variables
          (SNOW_HOSTNAME, SNOW_PHASE, SNOW_STORE_PATH).
        '';
        example = literalExpression ''
          {
            KUBECONFIG = "/home/user/.kube/config";
            LOG_LEVEL = "debug";
          }
        '';
      };

      environmentFiles = mkOption {
        type = types.listOf types.path;
        default = [];
        description = ''
          List of files containing environment variables (one per line,
          KEY=VALUE format). Useful for secrets management.

          Files are sourced in order before executing the hook command.
          Later files override earlier ones.
        '';
        example = literalExpression ''
          [
            config.age.secrets.slack-webhook.path
            /run/secrets/deploy-tokens
          ]
        '';
      };

      timeout = mkOption {
        type = types.ints.positive;
        default = 300;
        description = ''
          Maximum time (in seconds) to wait for the hook to complete.
          If the hook exceeds this timeout, it will be terminated and
          treated as a failure (unless continueOnFailure is true).

          Default: 300 seconds (5 minutes)
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

      priority = mkOption {
        type = types.int;
        default = 100;
        description = ''
          Execution priority for hooks within the same stage.

          Hooks are executed in order of priority (lower values first).
          Hooks with the same priority are executed in alphabetical order
          by name.

          Default: 100

          Common patterns:
          - Use negative values (-10, -5) for hooks that must run first
          - Use 0-50 for early hooks (e.g., pre-checks, validations)
          - Use 50-150 for normal hooks (default range)
          - Use 150-200 for late hooks (e.g., cleanup, notifications)
        '';
        example = 50;
      };

      retries = mkOption {
        type = types.ints.unsigned;
        default = 0;
        description = ''
          Number of times to retry the hook if it fails.

          The total number of attempts is retries + 1 (initial attempt plus retries).
          Set to 0 (default) for no retries.

          Useful for hooks that depend on external services that may be
          temporarily unavailable (e.g., Kubernetes API after node reboot).
        '';
        example = 5;
      };

      retryDelay = mkOption {
        type = types.ints.positive;
        default = 5;
        description = ''
          Seconds to wait between retry attempts.

          Only used when retries > 0.
        '';
        example = 30;
      };
    };
  });
in
{
  options.snowglobe.hooks = {
    pre-activate = mkOption {
      type = types.attrsOf hookSubmodule;
      default = {};
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
        {
          drain-node = {
            location = "local";
            command = "kubectl drain --ignore-daemonsets \${config.networking.hostName}";
            timeout = 360;
            continueOnFailure = false;
            priority = 50; # Run before other hooks
          };
        }
      '';
    };

    post-activate = mkOption {
      type = types.attrsOf hookSubmodule;
      default = {};
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
        {
          uncordon-node = {
            location = "local";
            command = "kubectl uncordon \${config.networking.hostName}";
            timeout = 60;
            continueOnFailure = true;
          };
        }
      '';
    };

    post-reboot = mkOption {
      type = types.attrsOf hookSubmodule;
      default = {};
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
        {
          wait-system-running = {
            location = "remote";
            command = "systemctl is-system-running --wait";
            timeout = 120;
            continueOnFailure = false;
          };
        }
      '';
    };

    on-failure = mkOption {
      type = types.attrsOf hookSubmodule;
      default = {};
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
        {
          uncordon-on-failure = {
            location = "local";
            command = "kubectl uncordon \${config.networking.hostName}";
            timeout = 60;
            continueOnFailure = true;
          };
          send-notification = {
            location = "local";
            command = "notify-send 'Deployment failed for \${config.networking.hostName}'";
            timeout = 10;
            continueOnFailure = true;
          };
        }
      '';
    };
  };

  config.assertions =
    let
      # Hook name validation - ensures hook names are safe for use in logs and scripts
      validateHookNames = stageName: hooks:
        map (hookName: {
          assertion =
            hookName != "" &&
            builtins.match "[a-zA-Z0-9_-]+" hookName != null;
          message = ''
            Hook name "${hookName}" in ${stageName} is invalid.
            Hook names must contain only alphanumeric characters (a-z, A-Z, 0-9),
            hyphens (-), and underscores (_).

            Valid examples: "drain-node", "check_health", "notify-deploy"
            Invalid: "${hookName}"
          '';
        }) (attrNames hooks);

      # Hook command validation - ensures commands are non-empty
      validateHookCommands = stageName: hooks:
        lib.flatten (map (hookName:
          let
            hook = hooks.${hookName};
          in {
            assertion = hook.command != "";
            message = ''
              Hook "${hookName}" in ${stageName} has an empty command.
              Hooks must specify a non-empty command to execute.
            '';
          }
        ) (attrNames hooks));

      # Hook timeout validation - ensures timeouts are reasonable
      validateHookTimeouts = stageName: hooks:
        lib.flatten (map (hookName:
          let
            hook = hooks.${hookName};
          in {
            assertion = hook.timeout >= 1 && hook.timeout <= 7200;
            message = ''
              Hook "${hookName}" in ${stageName} has an invalid timeout of ${toString hook.timeout} seconds.
              Timeouts must be between 1 and 7200 seconds (2 hours).

              If you need longer timeouts, consider breaking the operation into smaller steps
              or running it asynchronously outside of the deployment pipeline.
            '';
          }
        ) (attrNames hooks));

      # Hook retry validation - ensures retry configuration is reasonable
      validateHookRetries = stageName: hooks:
        lib.flatten (map (hookName:
          let
            hook = hooks.${hookName};
          in [
            {
              assertion = hook.retries <= 20;
              message = ''
                Hook "${hookName}" in ${stageName} has too many retries (${toString hook.retries}).
                Maximum allowed is 20 retries. For longer retry scenarios, consider
                implementing exponential backoff in the hook command itself.
              '';
            }
            {
              assertion = hook.retryDelay >= 1 && hook.retryDelay <= 300;
              message = ''
                Hook "${hookName}" in ${stageName} has an invalid retryDelay of ${toString hook.retryDelay} seconds.
                Retry delay must be between 1 and 300 seconds (5 minutes).
              '';
            }
          ]
        ) (attrNames hooks));

      # Collect all validations across all stages
      nameAssertions = lib.flatten (
        map (stage: validateHookNames stage config.snowglobe.hooks.${stage})
            (attrNames config.snowglobe.hooks)
      );

      commandAssertions = lib.flatten (
        map (stage: validateHookCommands stage config.snowglobe.hooks.${stage})
            (attrNames config.snowglobe.hooks)
      );

      timeoutAssertions = lib.flatten (
        map (stage: validateHookTimeouts stage config.snowglobe.hooks.${stage})
            (attrNames config.snowglobe.hooks)
      );

      retryAssertions = lib.flatten (
        map (stage: validateHookRetries stage config.snowglobe.hooks.${stage})
            (attrNames config.snowglobe.hooks)
      );
    in
      nameAssertions ++ commandAssertions ++ timeoutAssertions ++ retryAssertions;
}
