{
  name = "activation-failure";

  nodes = {
    web1.snowglobe.deploy.phase = "p1";
    web2 = { pkgs, ... }: {
      snowglobe.deploy.phase = "p2";
      systemd.services.storage-agent = {
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${pkgs.coreutils}/bin/false";
        };
      };
    };
    web3.snowglobe.deploy.phase = "p2";
    db1.snowglobe.deploy.phase = "p3";
  };

  demo = {
    title = "One host fails to activate, so the next phase is skipped";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    deploy = demo()
    deploy.failed()
    deploy.has(
        "✓ Phase p1", "✗ Phase p2 (1/2 failed", "✓ web3 (switch)",
        "✗ web2 at activate: activated, but storage-agent.service is failing",
        "⊘ Phase p3 (1 host skipped)", "⊘ db1", "✗ 2/4 deployed, 1 failed, 1 skipped",
    )
    assert "already failing" not in deploy.output, deploy.output
    deploy.logged("web2: failed at activate:", "web1: deployed", "web3: deployed")
    assert "db1: build started" not in deploy.log, deploy.log
    # NixOS keeps a system whose services failed to start
    for host in ["web1", "web2", "web3"]:
        runs(host, deployed(host))
    runs("db1", base("db1"))
  '';
}
