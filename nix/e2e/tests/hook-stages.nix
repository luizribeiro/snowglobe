let
  marker = "/tmp/hooks-ran";
  record = stage: {
    location = "local";
    command = ''echo "${stage} $SNOW_HOSTNAME" >> ${marker}'';
  };
  hooks = {
    post-reboot.record = record "post-reboot";
    post-deploy.record = record "post-deploy";
  };
in
{
  name = "hook-stages";

  nodes = {
    web1 = { lib, ... }: {
      snowglobe.deploy.rebootPolicy = "always";
      snowglobe.hooks = lib.mkMerge [
        hooks
        {
          post-reboot.boot-id = {
            location = "remote";
            command = "cat /proc/sys/kernel/random/boot_id > /root/post-reboot-boot-id";
          };
        }
      ];
    };
    web2.snowglobe.hooks = hooks;
  };

  testScript = ''
    def hooks_ran(host: str) -> list[str]:
        lines = deployer.succeed("cat ${marker}").splitlines()
        return [line.split()[0] for line in lines if line.endswith(f" {host}")]

    before = boot_id("web1")
    status, output = deployer.execute(f"cd {FLAKE} && snow deploy 2>&1")
    output = plain(output)
    assert status == 0, output
    contains(output, ("✓ web1 (boot, rebooted)", "✓ web2 (switch)"))
    assert hooks_ran("web1") == ["post-reboot", "post-deploy"], hooks_ran("web1")
    assert hooks_ran("web2") == ["post-deploy"], hooks_ran("web2")
    reconnect("web1")
    after = boot_id("web1")
    assert after != before
    ran_on = hosts["web1"].succeed("cat /root/post-reboot-boot-id").strip()
    assert ran_on == after, (ran_on, before, after)
  '';
}
