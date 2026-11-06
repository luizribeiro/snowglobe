{
  name = "rebuild-sudo";

  # Booted with them too, so the user is there to run snow
  base.boron = {
    users.users.luiz = {
      isNormalUser = true;
      extraGroups = [ "wheel" ];
      password = "snowglobe";
    };
    # The first sudo's lecture would crowd the demo
    security.sudo.extraConfig = "Defaults lecture = never";
  };

  nodes.boron = { };

  demo = {
    title = "snow rebuild asks for the sudo password once, before it starts";
    runOn = "boron";
    user = "luiz";
    script = ''
      run("snow rebuild")
      expect("password for luiz:")
      press("snowglobe", ENTER)
      expect("boron (switch)")
    '';
  };

  testScript = ''
    prompt = "[sudo] password for luiz: "
    rebuild = demo()
    rebuild.succeeded()
    assert rebuild.output.count(prompt) == 1, rebuild.output
    # Nothing redraws over the prompt while sudo waits for the password
    answered = rebuild.raw[rebuild.raw.index(prompt) + len(prompt):]
    assert answered.startswith(("\r\n", "\n")), repr(answered[:80])
    rebuild.cursor_shown_at(prompt)
    rebuild.cursor_hidden_at("boron  build")
    rebuild.has("boron needs sudo", "✓ boron (switch) in")
    rebuild.logged("boron: activated (Switch mode)")
    runs("boron", pushed("boron"))
  '';
}
