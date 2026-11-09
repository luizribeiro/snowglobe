{
  name = "never-phase";

  nodes = {
    web1.snowglobe.deploy = {
      tags = [ "web" ];
      serialGroup = "web";
    };
    web2.snowglobe.deploy = {
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
        host | {"name": "web1", "phase": None, "tags": ["web"], "serialGroup": "web"},
        host | {"name": "web2", "phase": "never", "tags": ["web"], "serialGroup": None},
    ], output

    def deploy(args: str) -> str:
        status, output = deployer.execute(f"cd {FLAKE} && snow deploy {args} 2>&1")
        assert status == 0, output
        return output

    output = deploy("")
    contains(output, ("Deploying 1 host: web1", "web1: deployed"))
    runs("web1", deployed("web1"))
    runs("web2", base("web2"))

    # web2 matches the tag, but only its name or phase selects it
    output = deploy("--tag web")
    contains(output, ("Deploying 1 host: web1",))
    runs("web2", base("web2"))

    output = deploy("--phase never --dry-run")
    contains(output, ("Deploying 1 host in 1 phase:", "never  web2"))
    runs("web2", base("web2"))

    output = deploy("web2")
    contains(output, ("Deploying 1 host in 1 phase:", "never  web2", "web2: deployed"))
    runs("web2", deployed("web2"))
  '';
}
