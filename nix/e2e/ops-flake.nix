# The flake a test's deployer pushes from. It can't evaluate the hosts'
# NixOS configs, having no nixpkgs, so it carries what snow reads from them,
# evaluated when the test was built: hosts.json, written by harness.nix.
{
  outputs = _:
    let
      built = builtins.fromJSON (builtins.readFile ./hosts.json);

      # A derivation, as the snowglobe module's hooks file is: snow builds it
      # to fetch what local hooks need
      hooksFile = file: derivation {
        name = "snowglobe-hooks.json";
        inherit (built) system;
        builder = "${builtins.storePath built.bash}/bin/bash";
        args = [ "-c" "printf %s \"$(< ${builtins.storePath file})\" > $out" ];
      };

      host = _: config: {
        config = {
          networking = { inherit (config) hostName domain; };
          snowglobe = {
            push = { inherit (config) phase tags rebootPolicy serialGroup sshPort; };
            hooksFile = hooksFile config.hooksFile;
          };
          system.build.toplevel = builtins.storePath config.toplevel;
        };
      };
    in
    {
      nixosConfigurations = builtins.mapAttrs host built.hosts;
    };
}
