{ pkgs, snowglobeModule }:

let
  inherit (pkgs) lib;

  eval = import (pkgs.path + "/nixos/lib/eval-config.nix") {
    system = null;
    modules = [
      snowglobeModule
      { nixpkgs.hostPlatform = pkgs.stdenv.hostPlatform.system; }
    ];
  };

  groups = [
    {
      name = "deploy";
      intro = "Options that decide whether, when and how `snow deploy` deploys a host.";
    }
    {
      name = "hooks";
      intro = "Commands that `snow deploy` runs around each host's deployment. Each stage is an attribute set of hooks keyed by a name of your choice; every hook takes the same options, listed once below as `snowglobe.hooks.<stage>.<name>.*`.";
    }
    { name = "localSystem"; }
  ];

  # Every stage declares the same hook options, so one stage's stand for all
  documentedStage = "pre-deploy";

  isHookOption = opt: lib.length opt.loc == 5 && lib.elemAt opt.loc 1 == "hooks";

  listHookOptionsOnce = opt:
    if !isHookOption opt then
      opt
    else if lib.elemAt opt.loc 2 != documentedStage then
      opt // { visible = false; }
    else
      let loc = lib.take 2 opt.loc ++ [ "<stage>" ] ++ lib.drop 3 opt.loc;
      in opt // { inherit loc; name = lib.showOption loc; };

  optionsDoc = pkgs.nixosOptionsDoc {
    options = { inherit (eval.options) snowglobe; };
    transformOptions = listHookOptionsOnce;
  };

  options = lib.mapAttrsToList (name: opt: opt // { inherit name; }) optionsDoc.optionsNix;

  groupOf = opt: lib.elemAt opt.loc 1;
  groupNames = map (group: group.name) groups;
  ungrouped = lib.subtractLists groupNames (lib.unique (map groupOf options));
  hookOptionsListed = lib.any isHookOption options;

  stagesBeforeTheirOptions = a: b:
    if lib.length a.loc != lib.length b.loc
    then lib.length a.loc < lib.length b.loc
    else a.name < b.name;

  optionsIn = group: lib.sort stagesBeforeTheirOptions (lib.filter (opt: groupOf opt == group.name) options);

  renderValue = label: value:
    let text = lib.trim value.text; in
    if value._type == "literalMD" then
      "**${label}:** ${text}"
    else if lib.hasInfix "\n" text then
      "**${label}:**\n\n```nix\n${text}\n```"
    else
      "**${label}:** `${text}`";

  renderOption = opt: lib.concatStringsSep "\n\n" (
    [
      "### `${opt.name}`"
      (lib.trim opt.description)
      "**Type:** ${opt.type}"
    ]
    ++ lib.optional (opt ? default) (renderValue "Default" opt.default)
    ++ lib.optional (opt ? example) (renderValue "Example" opt.example)
  );

  renderGroup = group: lib.concatStringsSep "\n\n" (
    [ "## ${group.name}" ]
    ++ lib.optional (group ? intro) group.intro
    ++ map renderOption (optionsIn group)
  );

  page = builtins.readFile ./header.md + "\n"
    + lib.concatMapStringsSep "\n\n" renderGroup groups + "\n";
in
assert lib.assertMsg (ungrouped == [ ])
  "nix/options-doc: add a section for snowglobe.${lib.concatStringsSep ", snowglobe." ungrouped}";
assert lib.assertMsg hookOptionsListed
  "nix/options-doc: no hook stage is named ${documentedStage}, so no hook options would be listed";
pkgs.writeText "options.md" page
