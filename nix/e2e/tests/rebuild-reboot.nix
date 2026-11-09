{
  name = "rebuild-reboot";

  # A new initrd, so boron has to reboot into it
  nodes.boron.boot.initrd.availableKernelModules = [ "nbd" ];

  demo = {
    title = "snow rebuild reboots into the new system";
    runOn = "boron";
    script = ''
      run("snow rebuild")
      expect("What now?")
      press(ENTER)
    '';
  };

  testScript = ''
    before = boot_id("boron")
    rebuild = demo(wait=False)
    # Asked from sodium: boron's own shell goes down with it
    sodium.wait_until_succeeds(
        f"ssh -o ConnectTimeout=5 root@boron 'test $(cat /proc/sys/kernel/random/boot_id) != {before}'",
        timeout=300,
    )
    reconnect("boron")
    rebuild.collect()
    rebuild.logged("boron: chose to reboot now", "boron: activated (Boot mode)", "boron: rebooting")
    boots("boron", deployed("boron"))
    recorded = boron.succeed("cat /var/lib/demo/demo.cast")
    assert "(boot, rebooting)" in recorded, recorded
  '';
}
