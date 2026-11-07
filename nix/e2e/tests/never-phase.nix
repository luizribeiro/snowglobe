{
  name = "never-phase";

  nodes = {
    boron.snowglobe.push.tags = [ "web" ];
    curium.snowglobe.push = {
      phase = "never";
      tags = [ "web" ];
    };
  };

  testScript = ''
    def push(args: str) -> str:
        status, output = deployer.execute(f"cd {FLAKE} && snow push {args} 2>&1")
        assert status == 0, output
        return output

    output = push("")
    contains(output, ("Pushing 1 host: boron", "boron: deployed"))
    runs("boron", pushed("boron"))
    runs("curium", base("curium"))

    # curium matches the tag, but only its name or phase selects it
    output = push("--tag web")
    contains(output, ("Pushing 1 host: boron",))
    runs("curium", base("curium"))

    output = push("curium")
    contains(output, ("Pushing 1 host in 1 phase:", "never  curium", "curium: deployed"))
    runs("curium", pushed("curium"))
  '';
}
