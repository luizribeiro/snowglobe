# A user whose sudo asks for a password, for tests that run snow as them.
# `module` goes on the host's base system, so the user is there to run snow.
rec {
  name = "alice";
  password = "snowglobe";
  prompt = "[sudo] password for ${name}: ";

  module = {
    users.users.${name} = {
      isNormalUser = true;
      extraGroups = [ "wheel" ];
      inherit password;
    };
    # The first sudo's lecture would crowd the demo
    security.sudo.extraConfig = "Defaults lecture = never";
  };
}
