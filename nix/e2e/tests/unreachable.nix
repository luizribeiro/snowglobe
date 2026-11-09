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
        push = demo()
    finally:
        cerium.unblock()
    push.failed()
    push.has("cerium")
    push.logged("cerium: failed at", "boron: deployed")
    runs("boron", pushed("boron"))
    runs("cerium", base("cerium"))
  '';
}
