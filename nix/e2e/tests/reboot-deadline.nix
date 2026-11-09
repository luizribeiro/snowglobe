{
  name = "reboot-deadline";

  nodes.web3.snowglobe.deploy.rebootPolicy = "always";

  demo = {
    title = "A rebooted host never comes back before the deadline";
    script = ''
      run("snow deploy --reboot-timeout 20")
    '';
  };

  testScript = ''
    # Restarted without allow_reboot, so a reboot powers web3 off
    web3.shutdown()
    web3.start()
    laptop.wait_until_succeeds("ssh -o BatchMode=yes root@web3 true")
    deploy = demo()
    deploy.failed()
    deploy.has("web3")
    deploy.logged("web3: failed at reboot:", "did not come back online within 20s")
    web3.wait_for_shutdown()
  '';
}
