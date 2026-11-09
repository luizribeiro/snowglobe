{
  name = "hung-agent";

  nodes.cerium = { };

  demo = {
    title = "The ssh-agent never answers, so the deployment stops at once";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    sodium.succeed(
        "systemd-run --unit=hung-agent socat UNIX-LISTEN:/tmp/hung-agent.sock,fork SYSTEM:'exec /run/current-system/sw/bin/sleep 600'"
    )
    sodium.wait_for_file("/tmp/hung-agent.sock")
    try:
        deploy = demo(env={"SSH_AUTH_SOCK": "/tmp/hung-agent.sock"})
    finally:
        sodium.succeed("systemctl stop hung-agent")
    deploy.failed()
    deploy.has("✗ The ssh-agent did not answer within 10s", "help: Every ssh connection would hang on it.")
    assert deploy.seconds < 60, deploy.seconds
    runs("cerium", base("cerium"))
  '';
}
