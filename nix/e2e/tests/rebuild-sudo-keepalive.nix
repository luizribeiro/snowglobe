let
  user = import ../sudo-user.nix;

  # sudo forgets the password before the build ends, unless snow's
  # keepalive (every 60s) refreshes it. snow rebuild runs `nix build` once,
  # so the build takes buildSeconds.
  sudoTimeoutMinutes = 2;
  buildSeconds = 150;
  # The build, then room for the check and the activation
  switchedWithinSeconds = buildSeconds + 120;
in
assert buildSeconds > sudoTimeoutMinutes * 60;
{
  name = "rebuild-sudo-keepalive";

  base.boron = {
    imports = [ user.module ];
    security.sudo.extraConfig = "Defaults timestamp_timeout=${toString sudoTimeoutMinutes}";
  };

  nodes.boron = { };

  demo = {
    title = "snow rebuild keeps sudo from forgetting the password during a long build";
    runOn = "boron";
    user = user.name;
    script = ''
      run("snow rebuild")
      expect("${user.prompt}")
      press("${user.password}", ENTER)
      expect("boron (switch)", timeout=${toString switchedWithinSeconds})
    '';
  };

  testScript = ''
    rebuild = demo(env={"SNOW_E2E_BUILD_SECONDS": "${toString buildSeconds}"})
    rebuild.succeeded()
    assert rebuild.seconds > ${toString buildSeconds}, rebuild.seconds
    assert rebuild.output.count("${user.prompt}") == 1, rebuild.output
    rebuild.has("✓ boron (switch) in")
    rebuild.logged("boron: activated (Switch mode)")
    runs("boron", pushed("boron"))
  '';
}
