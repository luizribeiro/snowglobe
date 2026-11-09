{
  name = "reboot";

  nodes.web1.snowglobe.deploy.rebootPolicy = "always";

  demo = {
    title = "A host that must reboot is set to boot the new system, then rebooted";
    script = ''
      run("snow deploy web1")
    '';
  };

  testScript = ''
    before = boot_id("web1")
    deploy = demo()
    deploy.succeeded()
    deploy.logged("web1: reboot started", "web1: reboot finished in", "web1: deployed")
    reconnect("web1")
    assert boot_id("web1") != before
    # A test VM boots the kernel it started with, so it runs its base system
    # again; the profile shows what it was told to boot
    boots("web1", deployed("web1"))
  '';
}
