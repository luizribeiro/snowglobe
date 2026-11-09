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
      run("snow push")
    '';
  };

  testScript = ''
    push = demo()
    push.succeeded()
    push.has("Pushing 3 hosts in 2 phases:", "p1  boron", "p2  cerium, radon", "✓ Phase p1", "✓ Phase p2")
    assert "could not read installed systems" not in push.output, push.output
    push.logged("boron: activate started", "boron: deployed", "cerium: deployed", "radon: deployed")
    assert push.log.index("boron: deployed") < push.log.index("cerium: build started"), push.log
    for host in hosts:
        runs(host, pushed(host))
  '';
}
