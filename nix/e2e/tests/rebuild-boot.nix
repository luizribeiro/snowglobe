{
  name = "rebuild-boot";

  # A new initrd, so workstation has to reboot into it
  nodes.workstation.boot.initrd.availableKernelModules = [ "nbd" ];

  demo = {
    title = "snow rebuild sets the new system to start on the next boot";
    runOn = "workstation";
    script = ''
      run("snow rebuild")
      expect("What now?")
      press(DOWN, ENTER)
    '';
  };

  testScript = ''
    rebuild = demo()
    rebuild.succeeded()
    rebuild.has("✓ workstation (boot) in")
    rebuild.logged("workstation: built", "workstation: chose to activate on the next boot", "workstation: activated (Boot mode)")
    boots("workstation", deployed("workstation"))
    runs("workstation", base("workstation"))
  '';
}
