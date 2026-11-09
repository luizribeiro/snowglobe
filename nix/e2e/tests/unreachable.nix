{
  name = "unreachable";

  nodes = {
    boron = { };
    cerium = { };
  };

  demo = {
    title = "A host is off the network: ssh times out and only that host fails";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    cerium.block()
    try:
        deploy = demo()
    finally:
        cerium.unblock()
    deploy.failed()
    deploy.has("cerium")
    deploy.logged("cerium: failed at", "boron: deployed")
    runs("boron", deployed("boron"))
    runs("cerium", base("cerium"))
  '';
}
