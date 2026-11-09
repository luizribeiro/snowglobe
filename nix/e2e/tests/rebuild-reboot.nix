{
  name = "rebuild-reboot";

  # A new initrd, so workstation has to reboot into it
  nodes.workstation.boot.initrd.availableKernelModules = [ "nbd" ];

  demo = {
    title = "snow rebuild reboots into the new system";
    runOn = "workstation";
    script = ''
      run("snow rebuild")
      expect("What now?")
      press(ENTER)
    '';
  };

  testScript = ''
    before = boot_id("workstation")
    rebuild = demo(wait=False)
    # Asked from laptop: workstation's own shell goes down with it
    laptop.wait_until_succeeds(
        f"ssh -o ConnectTimeout=5 root@workstation 'test $(cat /proc/sys/kernel/random/boot_id) != {before}'",
        timeout=300,
    )
    reconnect("workstation")
    rebuild.collect()
    rebuild.logged("workstation: chose to reboot now", "workstation: activated (Boot mode)", "workstation: rebooting")
    boots("workstation", deployed("workstation"))
    recorded = workstation.succeed("cat /var/lib/demo/demo.cast")
    assert "(boot, rebooting)" in recorded, recorded
  '';
}
