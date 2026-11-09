{
  name = "hook-failure";

  nodes = {
    web2 = { };
    web3.snowglobe.hooks.pre-deploy.wait-storage-healthy = {
      location = "local";
      command = ''echo "storage on $SNOW_HOSTNAME is degraded" >&2; exit 1'';
      timeout = 60;
    };
  };

  demo = {
    title = "A pre-deploy hook fails, so its host is never activated";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    deploy = demo()
    deploy.failed()
    deploy.has("web3", "wait-storage-healthy", "✗ web3 at pre-deploy: storage on web3 is degraded")
    deploy.logged("web3: failed at", "storage on web3 is degraded", "web2: deployed")
    runs("web2", deployed("web2"))
    runs("web3", base("web3"))
  '';
}
