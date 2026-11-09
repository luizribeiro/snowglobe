{
  name = "deploy-boot";

  nodes.db1.snowglobe.deploy.rebootPolicy = "never";

  demo = {
    title = "snow deploy --boot sets the new system for the next boot, without rebooting";
    script = ''
      run("snow deploy --boot db1")
    '';
  };

  testScript = ''
    before = boot_id("db1")
    deploy = demo()
    deploy.succeeded()
    deploy.has("✓ db1 (boot)")
    deploy.logged("db1: deployed")
    assert "reboot started" not in deploy.log, deploy.log
    assert boot_id("db1") == before
    boots("db1", deployed("db1"))
    runs("db1", base("db1"))
  '';
}
