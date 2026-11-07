{ pkgs }:

let
  evalPush = config: (pkgs.lib.evalModules {
    modules = [ ./push.nix config ];
  }).config.snowglobe.push;

  accepts = name: value:
    (builtins.tryEval (builtins.deepSeq (evalPush { snowglobe.push.${name} = value; }).${name} true)).success;
in
{
  testPhaseDefaultsToNull = {
    expr = (evalPush { }).phase;
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
    expr = (evalPush { }).tags;
    expected = [ ];
  };

  testTagsAcceptList = {
    expr = (evalPush { snowglobe.push.tags = [ "web" "database" ]; }).tags;
    expected = [ "web" "database" ];
  };

  testTagsRejectNonStrings = {
    expr = map (accepts "tags") [ "web" [ "web" 1 ] ];
    expected = [ false false ];
  };

  testRebootPolicyDefaultsToAuto = {
    expr = (evalPush { }).rebootPolicy;
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
    expr = (evalPush { }).optional;
    expected = false;
  };

  testOptionalAcceptsTrue = {
    expr = (evalPush { snowglobe.push.optional = true; }).optional;
    expected = true;
  };

  testOptionalRejectsString = {
    expr = accepts "optional" "yes";
    expected = false;
  };

  testSerialGroupDefaultsToNull = {
    expr = (evalPush { }).serialGroup;
    expected = null;
  };

  testSerialGroupAcceptsName = {
    expr = (evalPush { snowglobe.push.serialGroup = "kubernetes"; }).serialGroup;
    expected = "kubernetes";
  };

  testSerialGroupRejectsNonString = {
    expr = (builtins.tryEval (evalPush { snowglobe.push.serialGroup = 3; }).serialGroup).success;
    expected = false;
  };

  testSerialGroupRejectsEmptyName = {
    expr = (builtins.tryEval (evalPush { snowglobe.push.serialGroup = ""; }).serialGroup).success;
    expected = false;
  };

  testSshPortDefaultsToNull = {
    expr = (evalPush { }).sshPort;
    expected = null;
  };

  testSshPortAcceptsPort = {
    expr = (evalPush { snowglobe.push.sshPort = 2222; }).sshPort;
    expected = 2222;
  };

  testSshPortRejectsOutOfRange = {
    expr = (builtins.tryEval (evalPush { snowglobe.push.sshPort = 70000; }).sshPort).success;
    expected = false;
  };
}
