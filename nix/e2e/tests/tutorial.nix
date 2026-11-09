# The steps of the tutorial (docs/tutorial.md), recorded for its page
{
  name = "tutorial";

  nodes = {
    web1 = {
      snowglobe.deploy.phase = "p1";
      # The tutorial's first change has to switch without a reboot: nginx
      # adds a kernel module and caddy changes sysctls, so either would
      # make snow reboot web1
      services.prometheus.exporters.node = {
        enable = true;
        openFirewall = true;
      };
    };
    web2.snowglobe.deploy.phase = "p2";
  };

  demos = [
    {
      name = "tutorial-list";
      title = "snow list shows web1, with no phase or tags yet";
      script = ''run("snow list")'';
    }
    {
      name = "tutorial-dry-run";
      title = "snow deploy --dry-run previews the change to web1";
      script = ''run("snow deploy --dry-run web1")'';
    }
    {
      name = "tutorial-deploy";
      title = "snow deploy switches web1 to its new system";
      script = ''run("snow deploy web1")'';
    }
    # The same command again, now that web1 runs its new system
    {
      name = "tutorial-unchanged";
      title = "web1 already runs its new system, so it is left alone";
      script = ''run("snow deploy web1")'';
    }
    {
      name = "tutorial-phases";
      title = "web1 and web2 deploy in two phases";
      script = ''
        run("snow list")
        run("snow deploy")
      '';
    }
  ];

  testScript = ''
    import json

    hosts_json = f"{FLAKE}/hosts.json"
    finished = json.loads(deployer.succeed(f"cat {hosts_json}"))

    def flake_has(hosts: dict) -> None:
        text = json.dumps(finished | {"hosts": hosts})
        deployer.succeed(f"cat > {hosts_json} <<'EOF'\n{text}\nEOF")

    # Until its last step, the tutorial has only web1, without a phase
    flake_has({"web1": finished["hosts"]["web1"] | {"phase": None}})

    listed = demo("tutorial-list")
    listed.succeeded()
    listed.has("HOST   PHASE   TAGS", "web1   -       -")
    assert "web2" not in listed.output, listed.output

    dry_run = demo("tutorial-dry-run")
    dry_run.succeeded()
    dry_run.has("Deploying 1 host: web1", "✓ web1 (dry-run)", "✓ 1 host deployed")
    dry_run.logged("would reload the following units:", "firewall.service")
    runs("web1", base("web1"))

    deploy = demo("tutorial-deploy")
    deploy.succeeded()
    deploy.has("✓ web1 (switch)")
    runs("web1", deployed("web1"))
    web1.succeed("systemctl is-active prometheus-node-exporter")

    unchanged = demo("tutorial-unchanged")
    unchanged.succeeded()
    unchanged.has("✓ web1 (unchanged)")

    # The last step changes both hosts. web1 goes back to its base system,
    # so that it too has a new system to deploy; snow compares with the
    # system profile as well as the running system, so both go back.
    old = base("web1")
    web1.succeed(f"nix-env -p /nix/var/nix/profiles/system --set {old} && {old}/bin/switch-to-configuration switch")
    runs("web1", old)
    boots("web1", old)
    flake_has(finished["hosts"])

    phases = demo("tutorial-phases")
    phases.succeeded()
    phases.has(
        "web1   p1      -", "web2   p2      -",
        "Deploying 2 hosts in 2 phases:", "p1  web1", "p2  web2",
        "✓ web1 (switch)", "✓ web2 (switch)", "✓ 2 hosts deployed",
    )
    for host in hosts:
        runs(host, deployed(host))
  '';
}
