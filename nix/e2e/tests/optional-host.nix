{
  name = "optional-host";

  nodes = {
    web1.snowglobe.deploy = {
      phase = "p1";
      optional = true;
    };
    web2.snowglobe.deploy.phase = "p2";
  };

  testScript = ''
    web1.block()
    try:
        status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    finally:
        web1.unblock()
    assert status == 0, output
    contains(output, (
        "! Phase p1 (1/1 failed",
        "✗ web1 (optional) at",
        "web2: deployed",
        "✓ Phase p2",
        "1/2 deployed, 1 failed (optional)",
    ))
    runs("web1", base("web1"))
    runs("web2", deployed("web2"))
  '';
}
