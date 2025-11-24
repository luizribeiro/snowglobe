{ nixt, pkgs }:

let
  inherit (nixt.lib) block describe it;

  # Mock assertions module for standalone testing
  # (assertions option is normally provided by NixOS/darwin module system)
  assertionsModule = {
    options.assertions = pkgs.lib.mkOption {
      type = pkgs.lib.types.listOf pkgs.lib.types.unspecified;
      default = [];
    };
  };

  evalHooks = config: (pkgs.lib.evalModules {
    modules = [
      assertionsModule
      ./hooks.nix
      config
    ];
  }).config.snowglobe.hooks;
in
block ./hooks.test.nix [
  (describe "hooks module evaluation" [
    (it "evaluates a minimal pre-activate hook" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test-hook = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-activate.test-hook.command == "echo test"
    ))

    (it "evaluates a minimal post-activate hook" (
      let
        hooks = evalHooks {
          snowglobe.hooks.post-activate.test-hook = {
            location = "remote";
            command = "systemctl status";
          };
        };
      in
      hooks.post-activate.test-hook.location == "remote"
    ))

    (it "evaluates post-reboot hooks" (
      let
        hooks = evalHooks {
          snowglobe.hooks.post-reboot.verify = {
            location = "remote";
            command = "systemctl is-system-running --wait";
          };
        };
      in
      hooks.post-reboot.verify.command == "systemctl is-system-running --wait"
    ))

    (it "evaluates on-failure hooks" (
      let
        hooks = evalHooks {
          snowglobe.hooks.on-failure.cleanup = {
            location = "local";
            command = "kubectl uncordon myhost";
            continueOnFailure = true;
          };
        };
      in
      hooks.on-failure.cleanup.continueOnFailure == true
    ))
  ])

  (describe "hooks default values" [
    (it "sets default timeout to 300" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-activate.test.timeout == 300
    ))

    (it "sets default continueOnFailure to false" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-activate.test.continueOnFailure == false
    ))

    (it "sets default priority to 100" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-activate.test.priority == 100
    ))

    (it "sets default environment to empty" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-activate.test.environment == {}
    ))

    (it "sets default environmentFiles to empty" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-activate.test.environmentFiles == []
    ))
  ])

  (describe "hooks custom values" [
    (it "allows custom timeout" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
            timeout = 600;
          };
        };
      in
      hooks.pre-activate.test.timeout == 600
    ))

    (it "allows custom priority" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
            priority = -10;
          };
        };
      in
      hooks.pre-activate.test.priority == -10
    ))

    (it "allows custom environment variables" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
            environment = {
              KUBECONFIG = "/home/user/.kube/config";
              LOG_LEVEL = "debug";
            };
          };
        };
      in
      hooks.pre-activate.test.environment.KUBECONFIG == "/home/user/.kube/config" &&
      hooks.pre-activate.test.environment.LOG_LEVEL == "debug"
    ))
  ])

  (describe "multiple hooks in same stage" [
    (it "allows multiple hooks in pre-activate" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate = {
            first = { location = "local"; command = "echo first"; };
            second = { location = "local"; command = "echo second"; };
          };
        };
      in
      hooks.pre-activate.first.command == "echo first" &&
      hooks.pre-activate.second.command == "echo second"
    ))

    (it "allows multiple hooks in post-activate" (
      let
        hooks = evalHooks {
          snowglobe.hooks.post-activate = {
            a = { location = "local"; command = "cmd-a"; };
            b = { location = "remote"; command = "cmd-b"; };
            c = { location = "local"; command = "cmd-c"; };
          };
        };
      in
      hooks.post-activate.a.command == "cmd-a" &&
      hooks.post-activate.b.location == "remote" &&
      hooks.post-activate.c.command == "cmd-c"
    ))
  ])

  (describe "hooks across multiple stages" [
    (it "allows hooks in all stages simultaneously" (
      let
        hooks = evalHooks {
          snowglobe.hooks = {
            pre-activate.drain = { location = "local"; command = "kubectl drain"; };
            post-activate.uncordon = { location = "local"; command = "kubectl uncordon"; };
            post-reboot.verify = { location = "remote"; command = "systemctl is-system-running"; };
            on-failure.cleanup = { location = "local"; command = "kubectl uncordon"; };
          };
        };
      in
      hooks.pre-activate.drain.command == "kubectl drain" &&
      hooks.post-activate.uncordon.command == "kubectl uncordon" &&
      hooks.post-reboot.verify.location == "remote" &&
      hooks.on-failure.cleanup.command == "kubectl uncordon"
    ))
  ])

  (describe "location validation" [
    (it "accepts local location" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "local";
            command = "echo test";
          };
        };
      in
      hooks.pre-activate.test.location == "local"
    ))

    (it "accepts remote location" (
      let
        hooks = evalHooks {
          snowglobe.hooks.pre-activate.test = {
            location = "remote";
            command = "echo test";
          };
        };
      in
      hooks.pre-activate.test.location == "remote"
    ))
  ])

  (describe "empty stages" [
    (it "allows empty pre-activate" (
      let
        hooks = evalHooks {};
      in
      hooks.pre-activate == {}
    ))

    (it "allows empty post-activate" (
      let
        hooks = evalHooks {};
      in
      hooks.post-activate == {}
    ))

    (it "allows empty post-reboot" (
      let
        hooks = evalHooks {};
      in
      hooks.post-reboot == {}
    ))

    (it "allows empty on-failure" (
      let
        hooks = evalHooks {};
      in
      hooks.on-failure == {}
    ))
  ])
]
