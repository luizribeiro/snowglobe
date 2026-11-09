{
  name = "unreachable";

  nodes = {
    web1 = { };
    web2 = { };
  };

  demo = {
    title = "A host is off the network: ssh times out and only that host fails";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    web2.block()
    try:
        deploy = demo()
    finally:
        web2.unblock()
    deploy.failed()
    deploy.has("web2")
    deploy.logged("web2: failed at", "web1: deployed")
    runs("web1", deployed("web1"))
    runs("web2", base("web2"))
  '';
}
