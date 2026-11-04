# The pushes the test makes, in order. Each host gets one of push.nix's
# variants; the order matters, since a host already running its variant is
# left alone.
let
  localHook = command: { location = "local"; inherit command; timeout = 60; };
  slowDrain = { pre-deploy.drain-node = localHook "sleep 6"; };
  missingPath = "/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6y-wait-storage-healthy";
  # Run on boron itself, which gets a new initrd and so has to reboot
  rebuild = answers: {
    command = "rebuild";
    on = "boron";
    args = "";
    inherit answers;
    hosts.boron.variant = "initrd";
  };
  menu = "What now?";
  down = "\\x1b[B";
  enter = "\\r";
in
[
  {
    name = "switch";
    title = "Three hosts in two phases switch to a new system";
    args = "";
    hosts = {
      boron = { variant = "v1"; phase = "p1"; };
      cerium = { variant = "v1"; phase = "p2"; };
      radon = { variant = "v1"; phase = "p2"; };
    };
  }
  {
    name = "reboot";
    title = "A host that must reboot is set to boot the new system, then rebooted";
    args = "boron";
    hosts.boron = { variant = "v2"; rebootPolicy = "always"; };
  }
  {
    name = "activation-failure";
    title = "One host fails to activate while the others succeed";
    args = "";
    hosts = {
      boron.variant = "v3";
      cerium.variant = "broken";
      radon.variant = "v2";
    };
  }
  {
    name = "hook-failure";
    title = "A pre-deploy hook fails, so its host is never activated";
    args = "";
    hosts = {
      cerium.variant = "v1";
      radon = {
        variant = "v3";
        hooks.pre-deploy.wait-storage-healthy =
          localHook ''echo "storage on $SNOW_HOSTNAME is degraded" >&2; exit 1'';
      };
    };
  }
  {
    name = "missing-hook";
    title = "A local hook's program is missing, so the push stops before building";
    args = "";
    inherit missingPath;
    hosts = {
      boron.variant = "v1";
      cerium = {
        variant = "v2";
        hooks.pre-deploy.wait-storage-healthy = localHook "${missingPath}/bin/wait-storage-healthy";
      };
    };
  }
  {
    name = "unreachable";
    title = "A host is off the network: ssh times out and only that host fails";
    args = "";
    hosts = {
      boron.variant = "v1";
      cerium.variant = "v2";
    };
  }
  {
    name = "hung-agent";
    title = "The ssh-agent never answers, so the push stops at once";
    args = "";
    hosts.cerium.variant = "v2";
  }
  {
    name = "serial-group";
    title = "Two hosts in a serial group take turns";
    args = "";
    hosts = {
      cerium = { variant = "v2"; serialGroup = "storage"; hooks = slowDrain; };
      radon = { variant = "v3"; serialGroup = "storage"; hooks = slowDrain; };
    };
  }
  {
    name = "reboot-deadline";
    title = "A rebooted host never comes back before the deadline";
    args = "--reboot-timeout 20";
    hosts.radon = { variant = "v1"; rebootPolicy = "always"; };
  }
  (rebuild [ [ menu "${down} ${down} ${enter}" ] [ "Switch anyway?" "n" ] ] // {
    name = "rebuild-cancel";
    title = "snow rebuild warns before switching without a reboot, and is cancelled";
  })
  (rebuild [ [ menu "${down} ${enter}" ] ] // {
    name = "rebuild-boot";
    title = "snow rebuild sets the new system to start on the next boot";
  })
  (rebuild [ [ menu enter ] ] // {
    name = "rebuild-reboot";
    title = "snow rebuild reboots into the new system";
  })
]
