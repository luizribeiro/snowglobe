let
  # Slow enough that the second host is seen waiting for the first
  storageHost = {
    snowglobe.push.serialGroup = "storage";
    snowglobe.hooks.pre-deploy.drain-node = {
      location = "local";
      command = "sleep 6";
      timeout = 60;
    };
  };
in
{
  name = "serial-group";

  nodes = {
    cerium = storageHost;
    radon = storageHost;
  };

  demo = {
    title = "Two hosts in a serial group take turns";
    script = ''
      run("snow push")
    '';
  };

  testScript = ''
    push = demo()
    push.succeeded()
    waiting = re.search(r"(\w+): waiting for serial group 'storage' \(held by (\w+)\)", push.log)
    assert waiting, push.log
    waiter, holder = waiting.groups()
    assert {waiter, holder} == {"cerium", "radon"}, push.log
    assert push.log.index(f"{holder}: deployed") < push.log.index(f"{waiter}: activate started"), push.log
    for host in hosts:
        runs(host, pushed(host))
  '';
}
