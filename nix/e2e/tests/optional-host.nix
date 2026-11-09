{
  name = "optional-host";

  nodes = {
    boron.snowglobe.deploy = {
      phase = "p1";
      optional = true;
    };
    cerium.snowglobe.deploy.phase = "p2";
  };

  testScript = ''
    boron.block()
    try:
        status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    finally:
        boron.unblock()
    assert status == 0, output
    contains(output, (
        "! Phase p1 (1/1 failed",
        "✗ boron (optional) at",
        "cerium: deployed",
        "✓ Phase p2",
        "1/2 deployed, 1 failed (optional)",
    ))
    runs("boron", base("boron"))
    runs("cerium", pushed("cerium"))
  '';
}
