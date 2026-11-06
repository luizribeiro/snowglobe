{
  name = "rebuild-boot";

  # A new initrd, so boron has to reboot into it
  nodes.boron.boot.initrd.availableKernelModules = [ "nbd" ];

  demo = {
    title = "snow rebuild sets the new system to start on the next boot";
    runOn = "boron";
    script = ''
      run("snow rebuild")
      expect("What now?")
      press(DOWN, ENTER)
    '';
  };

  testScript = ''
    rebuild = demo()
    rebuild.succeeded()
    rebuild.has("✓ boron (boot) in")
    rebuild.logged("boron: built", "boron: chose to activate on the next boot", "boron: activated (Boot mode)")
    boots("boron", pushed("boron"))
    runs("boron", base("boron"))
  '';
}
