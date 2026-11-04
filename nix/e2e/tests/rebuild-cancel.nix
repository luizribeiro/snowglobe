{
  name = "rebuild-cancel";

  # A new initrd, so boron has to reboot into it
  nodes.boron.boot.initrd.availableKernelModules = [ "nbd" ];

  demo = {
    title = "snow rebuild warns before switching without a reboot, and is cancelled";
    runOn = "boron";
    script = ''
      run("snow rebuild")
      expect("What now?")
      press(DOWN, DOWN, ENTER)
      expect("Switch anyway?")
      press("n")
      expect("Cancelled, nothing changed")
    '';
  };

  testScript = ''
    rebuild = demo()
    rebuild.succeeded()
    rebuild.cursor_shown_at("Switch anyway?")
    rebuild.cursor_hidden_at("Rebuilding", "What now?", "mixes old and new parts")
    rebuild.has("Reboot needed:", "• initrd changed", "mixes old and new parts", "⊘ Cancelled, nothing changed")
    rebuild.logged("boron: reboot required: initrd changed", "boron: chose to cancel")
    boots("boron", base("boron"))
    runs("boron", base("boron"))
  '';
}
