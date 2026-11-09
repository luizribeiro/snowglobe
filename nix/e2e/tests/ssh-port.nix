{
  name = "ssh-port";

  # Booted as well as deployed: boron's sshd never listens on 22, and only
  # its own config tells snow where to find it
  base.boron = {
    services.openssh.ports = [ 2222 ];
    snowglobe.deploy.sshPort = 2222;
  };

  nodes = {
    boron.snowglobe.hooks.post-deploy.mark-deployed = {
      location = "remote";
      command = "touch /run/snowglobe-hook-ran";
      timeout = 60;
    };
  };

  testScript = ''
    status, output = deployer.execute("ssh -o BatchMode=yes -o ConnectTimeout=5 root@boron true 2>&1")
    assert status != 0 and "connect to host boron port 22" in output, output
    status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    assert status == 0, output
    runs("boron", deployed("boron"))
    boots("boron", deployed("boron"))
    hosts["boron"].succeed("test -e /run/snowglobe-hook-ran")
  '';
}
