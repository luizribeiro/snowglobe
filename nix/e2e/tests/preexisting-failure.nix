{
  name = "preexisting-failure";

  # Booted as well as deployed, so the deployment leaves these units as they are:
  # web1's failed once at boot, web2's keeps crashing like a broken app
  base = {
    web1 = { pkgs, ... }: {
      systemd.services.stale-job = {
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${pkgs.coreutils}/bin/false";
        };
      };
    };
    web2 = { pkgs, ... }: {
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
    web1.snowglobe.deploy.phase = "p1";
    web2.snowglobe.deploy = {
      phase = "p1";
      ignoredFailedUnits = [ "crash-loop.service" ];
    };
    web3.snowglobe.deploy.phase = "p2";
  };

  testScript = ''
    hosts["web1"].wait_until_succeeds("systemctl is-failed stale-job.service")
    hosts["web2"].wait_until_succeeds(
        "test $(systemctl show -P SubState crash-loop.service) = auto-restart"
    )
    status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    assert status != 0, output
    contains(output, (
        "✗ web1 at switch: activated, but stale-job.service is failing"
        " (it was already failing before this deployment)",
        "✓ web2 (switch, ignored failing crash-loop.service)",
        "⊘ Phase p2",
    ))
    assert "✓ web1" not in output, output
    # Both switched, failing or not
    runs("web1", deployed("web1"))
    runs("web2", deployed("web2"))
    runs("web3", base("web3"))
  '';
}
