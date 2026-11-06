{
  name = "rebuild-sudo-keepalive";

  # sudo forgets the password after 90s, before the build ends
  base.boron = {
    users.users.luiz = {
      isNormalUser = true;
      extraGroups = [ "wheel" ];
      password = "snowglobe";
    };
    security.sudo.extraConfig = ''
      Defaults timestamp_timeout=1.5
      Defaults lecture = never
    '';
  };

  nodes.boron = { };

  demo = {
    title = "snow rebuild keeps sudo from forgetting the password during a long build";
    runOn = "boron";
    user = "luiz";
    script = ''
      run("snow rebuild")
      expect("password for luiz:")
      press("snowglobe", ENTER)
      expect("boron (switch)", timeout=400)
    '';
  };

  testScript = ''
    rebuild = demo(env={"SNOW_E2E_BUILD_SECONDS": "120"})
    rebuild.succeeded()
    assert rebuild.seconds > 120, rebuild.seconds
    assert rebuild.output.count("password for luiz:") == 1, rebuild.output
    rebuild.has("✓ boron (switch) in")
    rebuild.logged("boron: activated (Switch mode)")
    runs("boron", pushed("boron"))
  '';
}
