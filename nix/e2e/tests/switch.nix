{
  name = "switch";

  # A fleet with a host per role, rolled out from the least to the most
  # visible: management, then data, then the apps, then the load balancers
  nodes = {
    bastion.snowglobe.deploy.phase = "p1";
    metrics.snowglobe.deploy.phase = "p1";
    pg-primary.snowglobe.deploy.phase = "p2";
    pg-replica-1.snowglobe.deploy.phase = "p2";
    redis-1.snowglobe.deploy.phase = "p2";
    api-1.snowglobe.deploy.phase = "p3";
    api-2.snowglobe.deploy.phase = "p3";
    api-3.snowglobe.deploy.phase = "p3";
    worker-1.snowglobe.deploy.phase = "p3";
    worker-2.snowglobe.deploy.phase = "p3";
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
        "p1  bastion, metrics",
        "p2  pg-primary, pg-replica-1, redis-1",
        "p3  api-1, api-2, api-3, worker-1, worker-2",
        "p4  lb-1, lb-2",
        "✓ Phase p1", "✓ Phase p2", "✓ Phase p3", "✓ Phase p4",
    )
    assert "could not read installed systems" not in deploy.output, deploy.output
    deploy.logged(*(f"{host}: deployed" for host in hosts))
    assert deploy.log.index("metrics: deployed") < deploy.log.index("pg-primary: build started"), deploy.log
    assert deploy.log.index("worker-2: deployed") < deploy.log.index("lb-1: build started"), deploy.log
    for host in hosts:
        runs(host, deployed(host))
  '';
}
