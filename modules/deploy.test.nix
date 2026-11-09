{ pkgs }:

let
  evalDeploy = config: (pkgs.lib.evalModules {
    modules = [ ./deploy.nix config ];
  }).config.snowglobe.deploy;

  accepts = name: value:
    (builtins.tryEval (builtins.deepSeq (evalDeploy { snowglobe.deploy.${name} = value; }).${name} true)).success;
in
{
  testPhaseDefaultsToNull = {
    expr = (evalDeploy { }).phase;
    expected = null;
  };

  testPhaseAcceptsNames = {
    expr = map (accepts "phase") [ "p1" "p10-k8s" "never" "storage_nodes" ];
    expected = [ true true true true ];
  };

  testPhaseRejectsOtherCharacters = {
    expr = map (accepts "phase") [ "" "p 1" "p1.k8s" "prod!" "日本" ];
    expected = [ false false false false false ];
  };

  testPhaseRejectsDefault = {
    expr = accepts "phase" "default";
    expected = false;
  };

  testTagsDefaultToEmpty = {
    expr = (evalDeploy { }).tags;
    expected = [ ];
  };

  testTagsAcceptList = {
    expr = (evalDeploy { snowglobe.deploy.tags = [ "web" "database" ]; }).tags;
    expected = [ "web" "database" ];
  };

  testTagsAcceptNames = {
    expr = accepts "tags" [ "etcd" "kubernetes" "dns-master" "storage_nodes" "k8s2" ];
    expected = true;
  };

  testTagsRejectOtherCharacters = {
    expr = map (tag: accepts "tags" [ "web" tag ]) [ "" "dns master" "dns.master" "web!" "web\n" "日本" ];
    expected = [ false false false false false false ];
  };

  testTagsRejectNonStrings = {
    expr = map (accepts "tags") [ "web" [ "web" 1 ] ];
    expected = [ false false ];
  };

  testRebootPolicyDefaultsToAuto = {
    expr = (evalDeploy { }).rebootPolicy;
    expected = "auto";
  };

  testRebootPolicyAcceptsPolicies = {
    expr = map (accepts "rebootPolicy") [ "auto" "never" "always" ];
    expected = [ true true true ];
  };

  testRebootPolicyRejectsOthers = {
    expr = accepts "rebootPolicy" "sometimes";
    expected = false;
  };

  testOptionalDefaultsToFalse = {
    expr = (evalDeploy { }).optional;
    expected = false;
  };

  testOptionalAcceptsTrue = {
    expr = (evalDeploy { snowglobe.deploy.optional = true; }).optional;
    expected = true;
  };

  testOptionalRejectsString = {
    expr = accepts "optional" "yes";
    expected = false;
  };

  testSerialGroupDefaultsToNull = {
    expr = (evalDeploy { }).serialGroup;
    expected = null;
  };

  testSerialGroupAcceptsName = {
    expr = (evalDeploy { snowglobe.deploy.serialGroup = "kubernetes"; }).serialGroup;
    expected = "kubernetes";
  };

  testSerialGroupRejectsNonString = {
    expr = (builtins.tryEval (evalDeploy { snowglobe.deploy.serialGroup = 3; }).serialGroup).success;
    expected = false;
  };

  testSerialGroupRejectsEmptyName = {
    expr = (builtins.tryEval (evalDeploy { snowglobe.deploy.serialGroup = ""; }).serialGroup).success;
    expected = false;
  };

  testIgnoredFailedUnitsDefaultToEmpty = {
    expr = (evalDeploy { }).ignoredFailedUnits;
    expected = [ ];
  };

  testIgnoredFailedUnitsAcceptUnitNames = {
    expr = accepts "ignoredFailedUnits" [
      "unifi.service"
      "mnt-data.mount"
      "logrotate.timer"
      "systemd-fsck@dev-disk-by\\x2dlabel-data.service"
    ];
    expected = true;
  };

  testIgnoredFailedUnitsRejectNamesWithoutAType = {
    expr = map (unit: accepts "ignoredFailedUnits" [ unit ])
      [ "unifi" "unifi.services" ".service" "" "unifi service" "units/unifi.service" ];
    expected = [ false false false false false false ];
  };

  testSshPortDefaultsToNull = {
    expr = (evalDeploy { }).sshPort;
    expected = null;
  };

  testSshPortAcceptsPort = {
    expr = (evalDeploy { snowglobe.deploy.sshPort = 2222; }).sshPort;
    expected = 2222;
  };

  testSshPortRejectsOutOfRange = {
    expr = (builtins.tryEval (evalDeploy { snowglobe.deploy.sshPort = 70000; }).sshPort).success;
    expected = false;
  };
}
