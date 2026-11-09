let
  user = import ../sudo-user.nix;
in
{
  name = "rebuild-sudo";

  base.boron = user.module;

  nodes.boron = { };

  demo = {
    title = "snow rebuild asks for the sudo password once, before it starts";
    runOn = "boron";
    user = user.name;
    script = ''
      run("snow rebuild")
      expect("${user.prompt}")
      press("${user.password}", ENTER)
      expect("boron (switch)")
    '';
  };

  testScript = ''
    prompt = "${user.prompt}"
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
    runs("boron", deployed("boron"))
  '';
}
