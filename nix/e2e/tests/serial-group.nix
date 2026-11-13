let
  # Slow enough that the second host is seen waiting for the first
  storageHost = {
    snowglobe.deploy.serialGroup = "storage";
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
    web2 = storageHost;
    web3 = storageHost;
  };

  demo = {
    title = "Two hosts in a serial group take turns";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    deploy = demo()
    deploy.succeeded()
    waiting = re.search(r"(\w+): waiting for serial group 'storage' \(held by (\w+)\)", deploy.log)
    assert waiting, deploy.log
    waiter, holder = waiting.groups()
    assert {waiter, holder} == {"web2", "web3"}, deploy.log
    assert deploy.log.index(f"{holder}: deployed") < deploy.log.index(f"{waiter}: switch started"), deploy.log
    for host in hosts:
        runs(host, deployed(host))
  '';
}
