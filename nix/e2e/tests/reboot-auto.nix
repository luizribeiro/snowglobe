{
  name = "reboot-auto";

  # Both hosts keep the default rebootPolicy, "auto"
  nodes = {
    web1.boot.initrd.systemd.contents."/etc/snowglobe-e2e".text = "reboot-auto";
    web2 = { pkgs, ... }: {
      environment.systemPackages = [ pkgs.hello ];
    };
  };

  testScript = ''
    before = {host: boot_id(host) for host in hosts}
    status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    output = plain(output)
    assert status == 0, output
    contains(output, (
        "✓ web1 (boot, rebooted)", "✓ web2 (switch)",
        "web1: reboot started", "web1: reboot finished in",
        "web1: deployed", "web2: deployed",
    ))
    assert "web2: reboot started" not in output, output
    reconnect("web1")
    assert boot_id("web1") != before["web1"]
    # A test VM boots the kernel it started with, so it runs its base system
    # again; the profile shows what it was told to boot
    boots("web1", deployed("web1"))
    assert boot_id("web2") == before["web2"]
    runs("web2", deployed("web2"))
    boots("web2", deployed("web2"))
  '';
}
