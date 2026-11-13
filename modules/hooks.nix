{ config, lib, localPkgs, ... }:

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
          - "local": Execute on the machine running `snow deploy`
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
          - SNOW_HOSTNAME: the host's name in the flake (its attribute under
            `nixosConfigurations` or `darwinConfigurations`), not the address
            snow connects to; interpolate `config.networking.hostName` for that
          - SNOW_PHASE: the host's `snowglobe.deploy.phase`; unset when it has none
          - SNOW_STORE_PATH: the new system's store path; unset in on-failure
            hooks of a host whose build failed

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
          Files of KEY=VALUE lines whose variables are added to the hook's
          environment, for secrets kept out of the Nix store.

          snow reads them on the machine running snow, for local and
          remote hooks alike, so they must exist there, not on the host
          being deployed. They are parsed, not sourced: blank lines and
          lines starting with `#` are skipped, each line is split at the
          first `=`, and keys and values are only trimmed: quotes are kept
          and variables are not expanded.

          Later files override earlier ones, and `environment` overrides
          them all. Give paths as strings: a Nix path such as `./deploy.env`
          is copied into the world-readable Nix store.

          Local hooks get these variables on top of snow's own environment.
          Remote hooks don't inherit snow's environment: they get the
          `SNOW_*` variables and yours on top of what the host gives a
          non-interactive SSH session.

          If a file can't be read, the host fails at that hook's stage
          without running it. With `continueOnFailure = true` the hook runs
          anyway without that file's variables, so a hook needing a secret
          from it should check the variable is set.
        '';
        example = literalExpression ''
          [
            "/home/deployer/.config/snowglobe/slack-webhook.env"
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

          Evaluation fails for a timeout outside 1 to 7200 seconds.
        '';
      };

      continueOnFailure = mkOption {
        type = types.bool;
        default = false;
        description = ''
          Whether the host carries on when this hook still fails after its
          last attempt.

          - false: the host fails at this hook's stage, and its on-failure
            hooks run
          - true: the failure is logged as a warning, and the host carries on

          It only decides what a failure of this hook does; it doesn't make
          the hook run when an earlier step failed. Use it for notifications,
          and for on-failure hooks so one failing cleanup doesn't stop the rest.
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

          Useful for hooks that depend on external services that may be
          temporarily unavailable (e.g., Kubernetes API after node reboot).

          Evaluation fails for more than 20 retries.
        '';
        example = 5;
      };

      retryDelay = mkOption {
        type = types.ints.positive;
        default = 5;
        description = ''
          Seconds to wait between retry attempts.

          Only used when retries > 0. Evaluation fails for a delay outside
          1 to 300 seconds.
        '';
        example = 30;
      };
    };
  });

  removedStages = {
    pre-activate = "Use snowglobe.hooks.pre-deploy instead; it only runs for hosts that change.";
    post-activate = "Use snowglobe.hooks.post-deploy instead; it runs after any reboot, once the host runs the new system.";
  };

  # Removed stages stay declared (so defining them fails with a pointer to the
  # replacement) but throw when read, so validation must skip them
  currentHooks = removeAttrs config.snowglobe.hooks (attrNames removedStages);

  # Local hooks keep their string context so building the hooks file on the
  # deployer realizes what they reference. Remote hooks run on the host and
  # reach it through the system closure, so building their dependencies here
  # would mean building for the host's platform on the deployer.
  discard = builtins.unsafeDiscardStringContext;

  hooksForPush = mapAttrs
    (stage: mapAttrs (name: hook:
      if hook.location == "remote"
      then hook // {
        command = discard hook.command;
        environment = mapAttrs (_: discard) hook.environment;
      }
      else hook))
    currentHooks;

  remoteHookLines = concatLists (mapAttrsToList
    (stage: hooks: concatLists (mapAttrsToList
      (name: hook:
        [ "${stage}/${name}: ${hook.command}" ]
        ++ mapAttrsToList (var: value: "${stage}/${name}: ${var}=${value}") hook.environment)
      (filterAttrs (_: hook: hook.location == "remote") hooks)))
    currentHooks);
in
{
  imports = [ ./local-pkgs.nix ] ++ mapAttrsToList
    (stage: instructions: mkRemovedOptionModule [ "snowglobe" "hooks" stage ] instructions)
    removedStages;

  options.snowglobe.hooksFile = mkOption {
    type = types.package;
    internal = true;
    readOnly = true;
    description = ''
      JSON file with all hooks, built for `snowglobe.localSystem`. snow builds
      it before deploying, which also fetches or builds what local hooks
      reference.
    '';
  };

  config.snowglobe.hooksFile = localPkgs.writeText "snowglobe-hooks.json" (builtins.toJSON hooksForPush);

  # Puts what remote hooks reference into the system closure, so it reaches
  # the host with the normal closure copy. An /etc file is the mechanism that
  # works the same on NixOS and nix-darwin.
  config.environment.etc."snowglobe/remote-hooks" = mkIf (remoteHookLines != [ ]) {
    text = concatStringsSep "\n" remoteHookLines + "\n";
  };

  options.snowglobe.hooks = {
    pre-deploy = mkOption {
      type = types.attrsOf hookSubmodule;
      default = {};
      description = ''
        Hooks to run after copying the new system but before activation.
        They only run for hosts that change: a host already running and
        booting the built system is skipped entirely.

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
            command = "kubectl drain --ignore-daemonsets ''${config.networking.hostName}";
            timeout = 360;
            continueOnFailure = false;
            priority = 50; # Run before other hooks
          };
        }
      '';
    };

    post-deploy = mkOption {
      type = types.attrsOf hookSubmodule;
      default = {};
      description = ''
        Hooks to run last, once the host runs the new system: after
        activation, after any reboot, and after post-reboot hooks.
        They run whether or not a reboot happened, but only if the
        deploy (including any reboot) succeeded.

        Common use cases:
        - Uncordon cluster nodes
        - Add back to load balancer pool
        - Gate on cluster health before the next host proceeds
        - Update service discovery
      '';
      example = literalExpression ''
        {
          uncordon-node = {
            location = "local";
            command = "kubectl uncordon ''${config.networking.hostName}";
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
            command = "kubectl uncordon ''${config.networking.hostName}";
            timeout = 60;
            continueOnFailure = true;
          };
          send-notification = {
            location = "local";
            command = "notify-send 'Deployment failed for ''${config.networking.hostName}'";
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
        map (stage: validateHookNames stage currentHooks.${stage})
            (attrNames currentHooks)
      );

      commandAssertions = lib.flatten (
        map (stage: validateHookCommands stage currentHooks.${stage})
            (attrNames currentHooks)
      );

      timeoutAssertions = lib.flatten (
        map (stage: validateHookTimeouts stage currentHooks.${stage})
            (attrNames currentHooks)
      );

      retryAssertions = lib.flatten (
        map (stage: validateHookRetries stage currentHooks.${stage})
            (attrNames currentHooks)
      );
    in
      nameAssertions ++ commandAssertions ++ timeoutAssertions ++ retryAssertions;
}
