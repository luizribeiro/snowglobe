{ pkgs, pushTest }:

(import ../demos).renderDemos {
  inherit pkgs;
  name = "snow-push-recordings";
  casts = "${pushTest}/casts";
  scenarios = map
    (s: {
      inherit (s) name title;
      command = "snow ${s.command or "push"}${pkgs.lib.optionalString (s.args != "") " ${s.args}"}";
    })
    (import ./scenarios.nix);
}
