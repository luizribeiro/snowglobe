{
  name = "switch";

  nodes = {
    web1.snowglobe.deploy.phase = "p1";
    web2.snowglobe.deploy.phase = "p2";
    web3.snowglobe.deploy.phase = "p2";
  };

  demo = {
    title = "Three hosts in two phases switch to a new system";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    deploy = demo()
    deploy.succeeded()
    deploy.has("Deploying 3 hosts in 2 phases:", "p1  web1", "p2  web2, web3", "✓ Phase p1", "✓ Phase p2")
    assert "could not read installed systems" not in deploy.output, deploy.output
    deploy.logged("web1: activate started", "web1: deployed", "web2: deployed", "web3: deployed")
    assert deploy.log.index("web1: deployed") < deploy.log.index("web2: build started"), deploy.log
    for host in hosts:
        runs(host, deployed(host))
  '';
}
