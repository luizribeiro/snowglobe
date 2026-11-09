{
  name = "activation-failure";

  nodes = {
    boron = { };
    cerium = { pkgs, ... }: {
      systemd.services.storage-agent = {
        wantedBy = [ "multi-user.target" ];
        serviceConfig = {
          Type = "oneshot";
          ExecStart = "${pkgs.coreutils}/bin/false";
        };
      };
    };
    radon = { };
  };

  demo = {
    title = "One host fails to activate while the others succeed";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    push = demo()
    push.failed()
    push.has("cerium", "1/3 failed", "✗ cerium at activate: activated, but storage-agent.service is failing")
    assert "already failing" not in push.output, push.output
    push.logged("cerium: failed at activate:", "boron: deployed", "radon: deployed")
    # NixOS keeps a system whose services failed to start
    for host in hosts:
        runs(host, pushed(host))
  '';
}
