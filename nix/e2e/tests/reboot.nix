{
  name = "reboot";

  nodes.boron.snowglobe.deploy.rebootPolicy = "always";

  demo = {
    title = "A host that must reboot is set to boot the new system, then rebooted";
    script = ''
      run("snow deploy boron")
    '';
  };

  testScript = ''
    before = boot_id("boron")
    push = demo()
    push.succeeded()
    push.logged("boron: reboot started", "boron: reboot finished in", "boron: deployed")
    reconnect("boron")
    assert boot_id("boron") != before
    # A test VM boots the kernel it started with, so it runs its base system
    # again; the profile shows what it was told to boot
    boots("boron", pushed("boron"))
  '';
}
