{
  name = "never-phase";

  nodes = {
    boron.snowglobe.deploy = {
      tags = [ "web" ];
      serialGroup = "web";
    };
    curium.snowglobe.deploy = {
      phase = "never";
      tags = [ "web" ];
    };
  };

  testScript = ''
    import json

    # A bare list shows hosts in the never phase too
    status, output = deployer.execute(f"cd {FLAKE} && snow list --json")
    assert status == 0, output
    system = deployer.succeed("uname -m").strip() + "-linux"
    host = {"system": system, "rebootPolicy": "auto", "optional": False}
    assert json.loads(output) == [
        host | {"name": "boron", "phase": None, "tags": ["web"], "serialGroup": "web"},
        host | {"name": "curium", "phase": "never", "tags": ["web"], "serialGroup": None},
    ], output

    def push(args: str) -> str:
        status, output = deployer.execute(f"cd {FLAKE} && snow deploy {args} 2>&1")
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

    output = push("--phase never --dry-run")
    contains(output, ("Pushing 1 host in 1 phase:", "never  curium"))
    runs("curium", base("curium"))

    output = push("curium")
    contains(output, ("Pushing 1 host in 1 phase:", "never  curium", "curium: deployed"))
    runs("curium", pushed("curium"))
  '';
}
