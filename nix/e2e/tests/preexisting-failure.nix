{
  name = "preexisting-failure";

  # Booted as well as pushed, so the push leaves these units as they are:
  # boron's failed once at boot, cerium's keeps crashing like a broken app
  base = {
    boron = { pkgs, ... }: {
      systemd.services.stale-job = {
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${pkgs.coreutils}/bin/false";
        };
      };
    };
    cerium = { pkgs, ... }: {
      systemd.services.crash-loop = {
        wantedBy = [ "multi-user.target" ];
        startLimitIntervalSec = 0;
        serviceConfig = {
          ExecStart = "${pkgs.coreutils}/bin/false";
          Restart = "always";
          RestartSec = 30;
        };
      };
    };
  };

  nodes = {
    boron.snowglobe.deploy.phase = "p1";
    cerium.snowglobe.deploy = {
      phase = "p1";
      ignoredFailedUnits = [ "crash-loop.service" ];
    };
    radon.snowglobe.deploy.phase = "p2";
  };

  testScript = ''
    hosts["boron"].wait_until_succeeds("systemctl is-failed stale-job.service")
    hosts["cerium"].wait_until_succeeds(
        "test $(systemctl show -P SubState crash-loop.service) = auto-restart"
    )
    status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    assert status != 0, output
    contains(output, (
        "✗ boron at activate: activated, but stale-job.service is failing"
        " (it was already failing before this deployment)",
        "✓ cerium (switch, ignored failing crash-loop.service)",
        "⊘ Phase p2",
    ))
    assert "boron (switch" not in output, output
    # Both switched, failing or not
    runs("boron", pushed("boron"))
    runs("cerium", pushed("cerium"))
    runs("radon", base("radon"))
  '';
}
