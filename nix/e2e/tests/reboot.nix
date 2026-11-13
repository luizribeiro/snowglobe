{
  name = "reboot";

  # web1 keeps the default rebootPolicy, "auto", so only its new initrd
  # makes snow reboot it
  nodes.web1.boot.initrd.systemd.contents."/etc/snowglobe-e2e".text = "reboot";

  demo = {
    title = "A host whose new initrd needs a reboot is set to boot the new system, then rebooted";
    script = ''
      run("snow deploy web1")
    '';
  };

  testScript = ''
    before = boot_id("web1")
    deploy = demo()
    deploy.succeeded()
    deploy.has("✓ web1 (boot, rebooted)")
    deploy.logged("web1: reboot started", "web1: reboot finished in", "web1: deployed")
    reconnect("web1")
    assert boot_id("web1") != before
    # A test VM boots the kernel it started with, so it runs its base system
    # again; the profile shows what it was told to boot
    boots("web1", deployed("web1"))
  '';
}
