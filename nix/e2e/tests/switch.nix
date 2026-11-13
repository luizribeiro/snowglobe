{
  name = "switch";

  # A fleet with a host per role, rolled out by risk as the phases guide
  # suggests: a canary and a host nobody depends on, then the stateless
  # services, then the data, then the shared infrastructure
  nodes = {
    api-1.snowglobe.deploy.phase = "p1";
    metrics.snowglobe.deploy.phase = "p1";
    api-2.snowglobe.deploy.phase = "p2";
    api-3.snowglobe.deploy.phase = "p2";
    worker-1.snowglobe.deploy.phase = "p2";
    worker-2.snowglobe.deploy.phase = "p2";
    pg-primary.snowglobe.deploy.phase = "p3";
    pg-replica-1.snowglobe.deploy.phase = "p3";
    redis-1.snowglobe.deploy.phase = "p3";
    bastion.snowglobe.deploy.phase = "p4";
    lb-1.snowglobe.deploy.phase = "p4";
    lb-2.snowglobe.deploy.phase = "p4";
  };

  # Thirteen VMs at the default 1 GB would outgrow a 16 GB CI runner
  common = { lib, ... }: {
    virtualisation.memorySize = lib.mkForce 512;
  };

  demo = {
    title = "Twelve hosts in four phases switch to a new system";
    script = ''
      run("snow deploy")
    '';
  };

  testScript = ''
    # As on a laptop that deployed before, so reading the settings doesn't
    # wait for twelve hooks files to build
    attrs = " ".join(f".#nixosConfigurations.{host}.config.snowglobe.hooksFile" for host in hosts)
    deployer.succeed(f"cd {FLAKE} && nix build --no-link {attrs}")
    # Builds as long as a small change's, so each host's progress shows
    deploy = demo(env={"SNOW_E2E_BUILD_SECONDS": "3"})
    deploy.succeeded()
    deploy.has(
        "Deploying 12 hosts in 4 phases:",
        "p1  api-1, metrics",
        "p2  api-2, api-3, worker-1, worker-2",
        "p3  pg-primary, pg-replica-1, redis-1",
        "p4  bastion, lb-1, lb-2",
        "✓ Phase p1", "✓ Phase p2", "✓ Phase p3", "✓ Phase p4",
    )
    assert "could not read installed systems" not in deploy.output, deploy.output
    deploy.logged(*(f"{host}: deployed" for host in hosts))
    assert deploy.log.index("api-1: deployed") < deploy.log.index("api-2: build started"), deploy.log
    assert deploy.log.index("redis-1: deployed") < deploy.log.index("bastion: build started"), deploy.log
    for host in hosts:
        runs(host, deployed(host))
  '';
}
