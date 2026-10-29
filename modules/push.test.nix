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
}
