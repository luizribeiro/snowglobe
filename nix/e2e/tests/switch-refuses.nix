let
  marker = "/tmp/hooks-ran";
  record = stage: {
    location = "local";
    command = ''echo "${stage} $SNOW_HOSTNAME" >> ${marker}'';
  };
in
{
  name = "switch-refuses";

  nodes.web1 = {
    # Restarts systemd-sysctl.service, which is not safe to restart live
    boot.kernel.sysctl."vm.swappiness" = 13;
    snowglobe.hooks = {
      pre-deploy.record = record "pre-deploy";
      on-failure.record = record "on-failure";
    };
  };

  testScript = ''
    def hooks_ran() -> list[str]:
        return deployer.succeed("cat ${marker} 2>/dev/null || true").splitlines()

    before = boot_id("web1")
    status, output = deployer.execute(f"cd {FLAKE} && snow deploy --switch web1 2>&1")
    output = plain(output)
    assert status != 0, output
    contains(output, (
        "✗ web1 at plan: systemd-sysctl.service would restart",
        "web1: failed at plan: systemd-sysctl.service would restart",
    ))
    assert "web1: activate started" not in output, output
    assert hooks_ran() == ["on-failure web1"], hooks_ran()
    runs("web1", base("web1"))
    boots("web1", base("web1"))

    status, output = deployer.execute(f"cd {FLAKE} && snow deploy --switch --force web1 2>&1")
    output = plain(output)
    assert status == 0, output
    contains(output, ("✓ web1 (switch)", "web1: deployed"))
    assert "web1: reboot started" not in output, output
    assert hooks_ran() == ["on-failure web1", "pre-deploy web1"], hooks_ran()
    assert boot_id("web1") == before
    runs("web1", deployed("web1"))
    boots("web1", deployed("web1"))
    assert hosts["web1"].succeed("sysctl -n vm.swappiness").strip() == "13"
  '';
}
