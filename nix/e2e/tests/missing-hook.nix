let
  missingPath = "/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6y-wait-storage-healthy";
in
{
  name = "missing-hook";

  nodes = {
    web1 = { };
    web2.snowglobe.hooks.pre-deploy.wait-storage-healthy = {
      location = "local";
      command = "${missingPath}/bin/wait-storage-healthy";
      timeout = 60;
    };
  };

  demo = {
    title = "A local hook's program is missing, so the deployment stops before building";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    deploy = demo()
    deploy.failed()
    # The error wraps the path after /nix/store/
    deploy.has("${builtins.baseNameOf missingPath}")
    assert "build started" not in deploy.log, deploy.log
    for host in hosts:
        runs(host, base(host))
  '';
}
