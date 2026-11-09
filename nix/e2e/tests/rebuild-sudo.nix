let
  user = import ../sudo-user.nix;
in
{
  name = "rebuild-sudo";

  base.workstation = user.module;

  nodes.workstation = { };

  demo = {
    title = "snow rebuild asks for the sudo password once, before it starts";
    runOn = "workstation";
    user = user.name;
    script = ''
      run("snow rebuild")
      expect("${user.prompt}")
      press("${user.password}", ENTER)
      expect("workstation (switch)")
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
    rebuild.cursor_hidden_at("workstation  build")
    rebuild.has("workstation needs sudo", "✓ workstation (switch) in")
    rebuild.logged("workstation: activated (Switch mode)")
    runs("workstation", deployed("workstation"))
  '';
}
