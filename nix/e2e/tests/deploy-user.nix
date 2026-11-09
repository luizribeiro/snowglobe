{
  name = "deploy-user";

  # Booted by every VM, so the hosts start out ready for a deploy user
  common = { lib, pkgs, ... }: {
    users.users.deploy = {
      isNormalUser = true;
      # The deploy user's ssh checks before the push would otherwise leave
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
    # have to honor as push does
    programs.ssh.extraConfig = lib.mkAfter ''
      Host boron cerium
        User deploy
    '';
  };

  nodes = {
    boron.snowglobe.hooks.post-deploy.record-user = {
      location = "remote";
      command = "id -un > /tmp/hook-user";
      timeout = 30;
    };
    # systemd-sysctl.service would restart, so cerium has to reboot
    cerium.boot.kernel.sysctl."vm.swappiness" = 10;
  };

  testScript = ''
    # -n, as ssh would otherwise read the test driver's commands as its input
    for host in hosts:
        assert sodium.succeed(f"ssh -n {host} id -un").strip() == "deploy"
        sodium.fail(f"ssh -n {host} sudo -n true")

    before = {host: boot_id(host) for host in hosts}
    status, output = sodium.execute(f"cd {FLAKE} && snow deploy 2>&1")
    assert status == 0, output
    contains(output, ("boron: deployed", "cerium: reboot started", "cerium: deployed"))
    assert "boron: reboot started" not in output, output

    runs("boron", pushed("boron"))
    assert boot_id("boron") == before["boron"]
    assert boron.succeed("cat /tmp/hook-user").strip() == "deploy"

    # Nothing but the sysctl differs from the base system, so only the
    # restart of systemd-sysctl.service can have called for this reboot
    reconnect("cerium")
    assert boot_id("cerium") != before["cerium"]
    boots("cerium", pushed("cerium"))
  '';
}
