{ pkgs }:

let
  # Mock assertions module for standalone testing
  # (assertions option is normally provided by NixOS/darwin module system)
  assertionsModule = {
    options.assertions = pkgs.lib.mkOption {
      type = pkgs.lib.types.listOf pkgs.lib.types.unspecified;
      default = [];
    };
  };

  evalConfig = config: (pkgs.lib.evalModules {
    modules = [
      assertionsModule
      ./hooks.nix
      { _module.args.pkgs = pkgs; }
      config
    ];
  }).config;

  evalHooks = config: (evalConfig config).snowglobe.hooks;

  aarch64Pkgs = import pkgs.path { localSystem = { system = "aarch64-linux"; }; };

  # An aarch64 host (e.g. a Raspberry Pi) pushed from an x86_64-linux deployer
  piWithHooks = (pkgs.lib.evalModules {
    modules = [
      assertionsModule
      ./hooks.nix
      { _module.args.pkgs = aarch64Pkgs; }
      ({ pkgs, localPkgs, ... }: {
        snowglobe.localSystem = "x86_64-linux";
        snowglobe.hooks.pre-deploy.health-gate = {
          location = "local";
          command = "${localPkgs.hello}/bin/hello --gate";
        };
        snowglobe.hooks.post-deploy.verify = {
          location = "remote";
          command = "${pkgs.hello}/bin/hello --verify";
        };
      })
    ];
  }).config;

  piHooksFile = piWithHooks.snowglobe.hooksFile;
  piHooksFileInputs = builtins.attrNames (builtins.getContext piHooksFile.text);
  piHooksJSON = builtins.fromJSON (builtins.unsafeDiscardStringContext piHooksFile.text);

  failedAssertionMessages = config:
    map (a: a.message) (builtins.filter (a: !a.assertion) (evalConfig config).assertions);
in
{
  "test hooks module evaluation: evaluates a minimal pre-deploy hook" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test-hook = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-deploy.test-hook.command == "echo test";
    expected = true;
  };

  "test hooks module evaluation: evaluates a minimal post-deploy hook" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.post-deploy.test-hook = {
            location = "remote";
            command = "systemctl status";
          };
        };
      in
      hooks.post-deploy.test-hook.location == "remote";
    expected = true;
  };

  "test hooks module evaluation: evaluates post-reboot hooks" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.post-reboot.verify = {
            location = "remote";
            command = "systemctl is-system-running --wait";
          };
        };
      in
      hooks.post-reboot.verify.command == "systemctl is-system-running --wait";
    expected = true;
  };

  "test hooks module evaluation: evaluates on-failure hooks" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.on-failure.cleanup = {
            location = "local";
            command = "kubectl uncordon myhost";
            continueOnFailure = true;
          };
        };
      in
      hooks.on-failure.cleanup.continueOnFailure == true;
    expected = true;
  };

  "test hooks default values: sets default timeout to 300" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-deploy.test.timeout == 300;
    expected = true;
  };

  "test hooks default values: sets default continueOnFailure to false" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-deploy.test.continueOnFailure == false;
    expected = true;
  };

  "test hooks default values: sets default priority to 100" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-deploy.test.priority == 100;
    expected = true;
  };

  "test hooks default values: sets default environment to empty" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-deploy.test.environment == {};
    expected = true;
  };

  "test hooks default values: sets default environmentFiles to empty" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-deploy.test.environmentFiles == [];
    expected = true;
  };

  "test hooks custom values: allows custom timeout" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
            timeout = 600;
          };
        };
      in
      hooks.pre-deploy.test.timeout == 600;
    expected = true;
  };

  "test hooks custom values: allows custom priority" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
            priority = -10;
          };
        };
      in
      hooks.pre-deploy.test.priority == -10;
    expected = true;
  };

  "test hooks custom values: allows custom environment variables" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
            environment = {
              KUBECONFIG = "/home/user/.kube/config";
              LOG_LEVEL = "debug";
            };
          };
        };
      in
      hooks.pre-deploy.test.environment.KUBECONFIG == "/home/user/.kube/config" &&
      hooks.pre-deploy.test.environment.LOG_LEVEL == "debug";
    expected = true;
  };

  "test multiple hooks in same stage: allows multiple hooks in pre-deploy" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy = {
            first = { location = "local"; command = "echo first"; };
            second = { location = "local"; command = "echo second"; };
          };
        };
      in
      hooks.pre-deploy.first.command == "echo first" &&
      hooks.pre-deploy.second.command == "echo second";
    expected = true;
  };

  "test multiple hooks in same stage: allows multiple hooks in post-deploy" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.post-deploy = {
            a = { location = "local"; command = "cmd-a"; };
            b = { location = "remote"; command = "cmd-b"; };
            c = { location = "local"; command = "cmd-c"; };
          };
        };
      in
      hooks.post-deploy.a.command == "cmd-a" &&
      hooks.post-deploy.b.location == "remote" &&
      hooks.post-deploy.c.command == "cmd-c";
    expected = true;
  };

  "test hooks across multiple stages: allows hooks in all stages simultaneously" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks = {
            pre-deploy.drain = { location = "local"; command = "kubectl drain"; };
            post-deploy.uncordon = { location = "local"; command = "kubectl uncordon"; };
            post-reboot.verify = { location = "remote"; command = "systemctl is-system-running"; };
            on-failure.cleanup = { location = "local"; command = "kubectl uncordon"; };
          };
        };
      in
      hooks.pre-deploy.drain.command == "kubectl drain" &&
      hooks.post-deploy.uncordon.command == "kubectl uncordon" &&
      hooks.post-reboot.verify.location == "remote" &&
      hooks.on-failure.cleanup.command == "kubectl uncordon";
    expected = true;
  };

  "test location validation: accepts local location" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-deploy.test.location == "local";
    expected = true;
  };

  "test location validation: accepts remote location" = {
    expr = let
        hooks = evalHooks {
          snowglobe.hooks.pre-deploy.test = {
            location = "remote";
            command = "echo test";
          };
        };
      in
      hooks.pre-deploy.test.location == "remote";
    expected = true;
  };

  "test empty stages: allows empty pre-deploy" = {
    expr = let
        hooks = evalHooks {};
      in
      hooks.pre-deploy == {};
    expected = true;
  };

  "test empty stages: allows empty post-deploy" = {
    expr = let
        hooks = evalHooks {};
      in
      hooks.post-deploy == {};
    expected = true;
  };

  "test empty stages: allows empty post-reboot" = {
    expr = let
        hooks = evalHooks {};
      in
      hooks.post-reboot == {};
    expected = true;
  };

  "test empty stages: allows empty on-failure" = {
    expr = let
        hooks = evalHooks {};
      in
      hooks.on-failure == {};
    expected = true;
  };

  "test removed hook stages: rejects pre-activate with a pointer to pre-deploy" = {
    expr = let
        messages = failedAssertionMessages {
          snowglobe.hooks.pre-activate.drain-node = {
            location = "local";
            command = "kubectl drain hydrogen";
          };
        };
      in
      builtins.any (m: pkgs.lib.hasInfix "snowglobe.hooks.pre-deploy" m) messages;
    expected = true;
  };

  "test removed hook stages: rejects post-activate with a pointer to post-deploy" = {
    expr = builtins.any (m: pkgs.lib.hasInfix "snowglobe.hooks.post-deploy" m) (failedAssertionMessages {
      snowglobe.hooks.post-activate.uncordon-node = {
        location = "local";
        command = "kubectl uncordon hydrogen";
      };
    });
    expected = true;
  };

  "test removed hook stages: has no failed assertions when only current stages are used" = {
    expr = failedAssertionMessages {
        snowglobe.hooks.pre-deploy.drain-node = {
          location = "local";
          command = "kubectl drain hydrogen";
        };
      } == [];
    expected = true;
  };

  "test hooks file: is built for the deployer's platform" = {
    expr = piHooksFile.system;
    expected = "x86_64-linux";
  };

  "test hooks file: depends on what local hooks reference" = {
    expr = builtins.elem pkgs.hello.drvPath piHooksFileInputs;
    expected = true;
  };

  "test hooks file: needs nothing built for the host's platform" = {
    expr = builtins.elem aarch64Pkgs.hello.drvPath piHooksFileInputs;
    expected = false;
  };

  "test hooks file: keeps remote commands verbatim" = {
    expr = piHooksJSON.post-deploy.verify.command;
    expected = builtins.unsafeDiscardStringContext "${aarch64Pkgs.hello}/bin/hello --verify";
  };

  "test hooks file: lists current stages only" = {
    expr = builtins.attrNames piHooksJSON;
    expected = [ "on-failure" "post-deploy" "post-reboot" "pre-deploy" ];
  };
}
