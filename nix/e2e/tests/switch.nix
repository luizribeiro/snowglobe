{
  name = "switch";

  nodes = {
    boron.snowglobe.deploy.phase = "p1";
    cerium.snowglobe.deploy.phase = "p2";
    radon.snowglobe.deploy.phase = "p2";
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
    deploy.has("Deploying 3 hosts in 2 phases:", "p1  boron", "p2  cerium, radon", "✓ Phase p1", "✓ Phase p2")
    assert "could not read installed systems" not in deploy.output, deploy.output
    deploy.logged("boron: activate started", "boron: deployed", "cerium: deployed", "radon: deployed")
    assert deploy.log.index("boron: deployed") < deploy.log.index("cerium: build started"), deploy.log
    for host in hosts:
        runs(host, deployed(host))
  '';
}
