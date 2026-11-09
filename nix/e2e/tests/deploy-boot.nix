{
  name = "deploy-boot";

  nodes.boron.snowglobe.deploy.rebootPolicy = "never";

  demo = {
    title = "snow deploy --boot sets the new system for the next boot, without rebooting";
    script = ''
      run("snow deploy --boot boron")
    '';
  };

  testScript = ''
    before = boot_id("boron")
    deploy = demo()
    deploy.succeeded()
    deploy.has("✓ boron (boot)")
    deploy.logged("boron: deployed")
    assert "reboot started" not in deploy.log, deploy.log
    assert boot_id("boron") == before
    boots("boron", deployed("boron"))
    runs("boron", base("boron"))
  '';
}
