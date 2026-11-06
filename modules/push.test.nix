{ pkgs }:

let
  evalPush = config: (pkgs.lib.evalModules {
    modules = [ ./push.nix config ];
  }).config.snowglobe.push;
in
{
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
