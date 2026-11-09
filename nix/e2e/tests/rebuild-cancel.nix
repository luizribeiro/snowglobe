{
  name = "rebuild-cancel";

  # A new initrd, so workstation has to reboot into it
  nodes.workstation.boot.initrd.availableKernelModules = [ "nbd" ];

  demo = {
    title = "snow rebuild warns before switching without a reboot, and is cancelled";
    runOn = "workstation";
    script = ''
      run("snow rebuild")
      expect("What now?")
      press(DOWN, DOWN, ENTER)
      expect("Switch anyway?")
      press("n")
      expect("workstation (cancelled)")
    '';
  };

  testScript = ''
    rebuild = demo()
    rebuild.succeeded()
    rebuild.cursor_shown_at("Switch anyway?")
    rebuild.cursor_hidden_at("workstation  build", "What now?", "mixes old and new parts")
    rebuild.has("Reboot needed:", "• initrd changed", "mixes old and new parts", "⊘ workstation (cancelled) in")
    rebuild.logged("workstation: reboot required: initrd changed", "workstation: chose to cancel")
    boots("workstation", base("workstation"))
    runs("workstation", base("workstation"))
  '';
}
