{
  description = "pesde; A package manager for the Luau programming language, supporting multiple runtimes including Roblox and Lune";
  inputs = {
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = {
    flake-utils,
    nixpkgs,
    ...
  } @ inputs:
    flake-utils.lib.eachDefaultSystem (system: let
      treefmtEval = inputs.treefmt-nix.lib.evalModule inputs.nixpkgs.legacyPackages.${system} ./treefmt.nix;
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      devShells.default = import ./nix/shell.nix {inherit pkgs;};

      packages = let
        pesde = pkgs.callPackage ./nix/package.nix {};
      in {
        inherit pesde;
        default = pesde;
      };
    });
}
