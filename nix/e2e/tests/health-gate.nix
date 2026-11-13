let
  failureMarker = "/tmp/rolled-back";
in
{
  name = "health-gate";

  nodes = {
    web1 = { pkgs, ... }: {
      snowglobe.deploy.phase = "p1";
      snowglobe.hooks = {
        # Nothing listens on 8080, so the canary never turns healthy
        post-deploy.healthz = {
          location = "remote";
          command = "${pkgs.curl}/bin/curl -fsS --max-time 5 http://localhost:8080/healthz";
          timeout = 30;
        };
        on-failure.roll-back = {
          location = "local";
          command = ''echo "$SNOW_HOSTNAME" >> ${failureMarker}'';
          continueOnFailure = true;
        };
      };
    };
    db1.snowglobe.deploy.phase = "p2";
  };

  testScript = ''
    # Without a terminal, snow logs to stderr, next to its report
    status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    output = plain(output)
    assert status != 0, output
    contains(output, (
        "web1: failed at post-deploy", "✗ web1 at post-deploy",
        "⊘ Phase p2 (1 host skipped)", "⊘ db1",
    ))
    assert "db1: build started" not in output, output
    assert deployer.succeed("cat ${failureMarker}").split() == ["web1"]
    runs("web1", deployed("web1"))
    runs("db1", base("db1"))
  '';
}
