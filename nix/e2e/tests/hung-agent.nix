{
  name = "hung-agent";

  nodes.cerium = { };

  demo = {
    title = "The ssh-agent never answers, so the push stops at once";
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
        push = demo(env={"SSH_AUTH_SOCK": "/tmp/hung-agent.sock"})
    finally:
        sodium.succeed("systemctl stop hung-agent")
    push.failed()
    push.has("✗ The ssh-agent did not answer within 10s", "help: Every ssh connection would hang on it.")
    assert push.seconds < 60, push.seconds
    runs("cerium", base("cerium"))
  '';
}
