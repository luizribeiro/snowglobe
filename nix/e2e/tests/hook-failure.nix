{
  name = "hook-failure";

  nodes = {
    cerium = { };
    radon.snowglobe.hooks.pre-deploy.wait-storage-healthy = {
      location = "local";
      command = ''echo "storage on $SNOW_HOSTNAME is degraded" >&2; exit 1'';
      timeout = 60;
    };
  };

  demo = {
    title = "A pre-deploy hook fails, so its host is never activated";
    script = ''
      run("snow push")
    '';
  };

  testScript = ''
    push = demo()
    push.failed()
    push.has("radon", "wait-storage-healthy", "✗ radon at pre-deploy: storage on radon is degraded")
    push.logged("radon: failed at", "storage on radon is degraded", "cerium: deployed")
    runs("cerium", pushed("cerium"))
    runs("radon", base("radon"))
  '';
}
