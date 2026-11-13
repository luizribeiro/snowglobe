let
  drainMarker = "/root/drained";
  slowStarted = "/tmp/slow-started";
  failureMarker = "/tmp/on-failure";
in
{
  name = "deploy-cancel";

  nodes.web1 = {
    # Every switch runs activation scripts, so this holds each deploy in
    # activation for 30s, long enough to interrupt
    system.activationScripts.slow-start = "touch ${slowStarted}; sleep 30";
    snowglobe.hooks = {
      pre-deploy.drain = {
        location = "remote";
        command = "touch ${drainMarker}";
      };
      post-deploy.undrain = {
        location = "remote";
        command = "rm ${drainMarker}";
      };
      on-failure.undrain = {
        location = "local";
        command = ''echo "$SNOW_HOSTNAME" >> ${failureMarker}'';
        continueOnFailure = true;
      };
    };
  };

  testScript = ''
    web1 = hosts["web1"]
    # In a terminal, as Ctrl-C is pressed there; sh's pid is snow's after exec
    deployer.succeed(
        "cat > /tmp/deploy.sh <<'EOF'\n"
        "echo $$ > /tmp/snow.pid\n"
        "exec snow deploy\n"
        "EOF"
    )
    deployer.succeed(
        f"systemd-run --unit=deploy --working-directory={FLAKE} "
        "--setenv=PATH=/run/wrappers/bin:/run/current-system/sw/bin "
        "--setenv=HOME=/root --setenv=TERM=xterm-256color "
        "/bin/sh -c 'script -qfec \"sh /tmp/deploy.sh\" /tmp/deploy.out; echo $? > /tmp/deploy.status'"
    )
    def snow_exited() -> bool:
        return deployer.execute("test -e /tmp/deploy.status")[0] == 0

    def snow_output() -> str:
        return deployer.execute("cat /tmp/deploy.out /tmp/snow-*.log")[1]

    def activation_reached_or_snow_exited(last_try: bool) -> bool:
        reached = web1.execute("test -e ${slowStarted}")[0] == 0 or snow_exited()
        if last_try and not reached:
            print(snow_output())
        return reached

    retry(activation_reached_or_snow_exited, timeout_seconds=600)
    assert not snow_exited(), snow_output()
    # A terminal sends Ctrl-C's SIGINT to the whole foreground process group
    deployer.succeed("kill -INT -- -$(ps -o pgid= -p $(cat /tmp/snow.pid) | tr -d ' ')")
    deployer.wait_for_file("/tmp/deploy.status")
    assert int(deployer.succeed("cat /tmp/deploy.status")) == 130
    output = plain(deployer.succeed("cat /tmp/deploy.out"))
    snow_log = deployer.succeed("cat /tmp/snow-*.log")
    # snow exits on the spot, its board frozen mid-activation. ssh runs the
    # activation without a terminal, so the host sends it no SIGHUP when
    # ssh dies, and it carries on without snow
    contains(output, ("web1  activate",))
    assert "✗ web1" not in output, output
    contains(snow_log, ("web1: activate started",))
    assert "web1: activate finished" not in snow_log, snow_log
    web1.succeed("pgrep -f '[b]in/switch-to-configuration switch'")
    web1.wait_until_succeeds("! pgrep -f '[b]in/switch-to-configuration switch'")
    runs("web1", deployed("web1"))
    boots("web1", deployed("web1"))
    # Neither post-deploy nor on-failure hooks run, so the host stays drained
    web1.succeed("test -e ${drainMarker}")
    deployer.fail("test -e ${failureMarker}")

    # The host already runs the new system, so only --redeploy runs its hooks
    unchanged = plain(deployer.succeed(f"cd {FLAKE} && snow deploy 2>&1"))
    contains(unchanged, ("✓ web1 (unchanged)",))
    web1.succeed("test -e ${drainMarker}")
    deployer.succeed(f"cd {FLAKE} && snow deploy --redeploy web1 2>&1")
    web1.fail("test -e ${drainMarker}")
  '';
}
