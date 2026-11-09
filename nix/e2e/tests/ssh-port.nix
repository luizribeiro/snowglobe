{
  name = "ssh-port";

  # Booted as well as deployed: web1's sshd never listens on 22, and only
  # its own config tells snow where to find it
  base.web1 = {
    services.openssh.ports = [ 2222 ];
    snowglobe.deploy.sshPort = 2222;
  };

  nodes = {
    web1.snowglobe.hooks.post-deploy.mark-deployed = {
      location = "remote";
      command = "touch /run/snowglobe-hook-ran";
      timeout = 60;
    };
  };

  testScript = ''
    status, output = deployer.execute("ssh -o BatchMode=yes -o ConnectTimeout=5 root@web1 true 2>&1")
    assert status != 0 and "connect to host web1 port 22" in output, output
    status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    assert status == 0, output
    runs("web1", deployed("web1"))
    boots("web1", deployed("web1"))
    hosts["web1"].succeed("test -e /run/snowglobe-hook-ran")
  '';
}
