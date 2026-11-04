let
  missingPath = "/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6y-wait-storage-healthy";
in
{
  name = "missing-hook";

  nodes = {
    boron = { };
    cerium.snowglobe.hooks.pre-deploy.wait-storage-healthy = {
      location = "local";
      command = "${missingPath}/bin/wait-storage-healthy";
      timeout = 60;
    };
  };

  demo = {
    title = "A local hook's program is missing, so the push stops before building";
    script = ''
      run("snow push")
    '';
  };

  testScript = ''
    push = demo()
    push.failed()
    # The error wraps the path after /nix/store/
    push.has("${builtins.baseNameOf missingPath}")
    assert "build started" not in push.log, push.log
    for host in hosts:
        runs(host, base(host))
  '';
}
