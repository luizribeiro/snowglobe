{
  name = "reboot-deadline";

  nodes.radon.core.rebootPolicy = "always";

  demo = {
    title = "A rebooted host never comes back before the deadline";
    script = ''
      run("snow push --reboot-timeout 20")
    '';
  };

  testScript = ''
    # Restarted without allow_reboot, so a reboot powers radon off
    radon.shutdown()
    radon.start()
    sodium.wait_until_succeeds("ssh -o BatchMode=yes root@radon true", timeout=60)
    push = demo()
    push.failed()
    push.has("radon")
    push.logged("radon: failed at reboot:", "did not come back online within 20s")
    radon.wait_for_shutdown()
  '';
}
