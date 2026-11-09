{
  name = "deploy-user";

  # Booted by every VM, so the hosts start out ready for a deploy user
  common = { lib, pkgs, ... }: {
    users.users.deploy = {
      isNormalUser = true;
      # The deploy user's ssh checks before the deployment would otherwise leave
      # its user manager stopping during the switch (see the harness)
      linger = true;
      openssh.authorizedKeys.keys =
        [ (import "${pkgs.path}/nixos/tests/ssh-keys.nix" pkgs).snakeOilPublicKey ];
    };
    snowglobe.deploy.allowedUsers = [ "deploy" ];
    # nix copy as a non-root user only imports the unsigned paths snow
    # copies when the user is trusted
    nix.settings.trusted-users = [ "deploy" ];
    # The user comes from the SSH config alone, with no --user, which hooks
    # have to honor as `snow deploy` does
    programs.ssh.extraConfig = lib.mkAfter ''
      Host web1 web2
        User deploy
    '';
  };

  nodes = {
    web1.snowglobe.hooks.post-deploy.record-user = {
      location = "remote";
      command = "id -un > /tmp/hook-user";
      timeout = 30;
    };
    # systemd-sysctl.service would restart, so web2 has to reboot
    web2.boot.kernel.sysctl."vm.swappiness" = 10;
  };

  testScript = ''
    # -n, as ssh would otherwise read the test driver's commands as its input
    for host in hosts:
        assert laptop.succeed(f"ssh -n {host} id -un").strip() == "deploy"
        laptop.fail(f"ssh -n {host} sudo -n true")

    before = {host: boot_id(host) for host in hosts}
    status, output = laptop.execute(f"cd {FLAKE} && snow deploy 2>&1")
    assert status == 0, output
    contains(output, ("web1: deployed", "web2: reboot started", "web2: deployed"))
    assert "web1: reboot started" not in output, output

    runs("web1", deployed("web1"))
    assert boot_id("web1") == before["web1"]
    assert web1.succeed("cat /tmp/hook-user").strip() == "deploy"

    # Nothing but the sysctl differs from the base system, so only the
    # restart of systemd-sysctl.service can have called for this reboot
    reconnect("web2")
    assert boot_id("web2") != before["web2"]
    boots("web2", deployed("web2"))
  '';
}
