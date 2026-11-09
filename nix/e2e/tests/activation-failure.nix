{
  name = "activation-failure";

  nodes = {
    web1 = { };
    web2 = { pkgs, ... }: {
      systemd.services.storage-agent = {
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${pkgs.coreutils}/bin/false";
        };
      };
    };
    web3 = { };
  };

  demo = {
    title = "One host fails to activate while the others succeed";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    deploy = demo()
    deploy.failed()
    deploy.has("web2", "1/3 failed", "✗ web2 at activate: activated, but storage-agent.service is failing")
    assert "already failing" not in deploy.output, deploy.output
    deploy.logged("web2: failed at activate:", "web1: deployed", "web3: deployed")
    # NixOS keeps a system whose services failed to start
    for host in hosts:
        runs(host, deployed(host))
  '';
}
