{
  name = "push-boot";

  nodes.boron.snowglobe.push.rebootPolicy = "never";

  demo = {
    title = "snow push --boot sets the new system for the next boot, without rebooting";
    script = ''
      run("snow push --boot boron")
    '';
  };

  testScript = ''
    before = boot_id("boron")
    push = demo()
    push.succeeded()
    push.has("✓ boron (boot)")
    push.logged("boron: deployed")
    assert "reboot started" not in push.log, push.log
    assert boot_id("boron") == before
    boots("boron", pushed("boron"))
    runs("boron", base("boron"))
  '';
}
