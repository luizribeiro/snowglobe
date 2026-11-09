# Must evaluate for any host in nixosConfigurations or darwinConfigurations,
# including ones without the snowglobe module.
#
# When this machine's platform differs from the host's, the host is
# re-evaluated with `snowglobe.localSystem` set to it, so local hooks get
# packages that run here. The re-evaluation costs about half a second per
# host, so it's skipped when the platforms already match.
localSystem: host:
let
  needsLocalSystem = host ? options && host.options ? snowglobe && host.options.snowglobe ? localSystem
    && host.config.snowglobe.localSystem != localSystem;
  extended = if needsLocalSystem
    then host.extendModules {
      modules = [ ({ lib, ... }: { snowglobe.localSystem = lib.mkForce localSystem; }) ];
    }
    else host;
  config = extended.config;
in {
  hostName = config.networking.hostName;
  domain = config.networking.domain or "";
  system = config.nixpkgs.hostPlatform.system or null;
  phase = config.snowglobe.deploy.phase or null;
  tags = config.snowglobe.deploy.tags or [];
  rebootPolicy = config.snowglobe.deploy.rebootPolicy or "auto";
  serialGroup = config.snowglobe.deploy.serialGroup or null;
  sshPort = config.snowglobe.deploy.sshPort or null;
  optional = config.snowglobe.deploy.optional or false;
  ignoredFailedUnits = config.snowglobe.deploy.ignoredFailedUnits or [];
  hooksFile = if config ? snowglobe && config.snowglobe ? hooksFile
    then config.snowglobe.hooksFile.drvPath
    else null;
}
