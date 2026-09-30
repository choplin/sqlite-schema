{
  description = "Declarative schema migration tool for SQLite";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0";
    flake-parts.url = "github:hercules-ci/flake-parts";
    devshell = {
      url = "github:numtide/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      nixpkgs,
      flake-parts,
      devshell,
      fenix,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];

      perSystem =
        {
          system,
          ...
        }:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ devshell.overlays.default ];
          };
          rustToolchain = fenix.packages.${system}.stable.withComponents [
            "cargo"
            "clippy"
            "rust-analyzer"
            "rust-src"
            "rustc"
            "rustfmt"
          ];
        in
        {
          devShells.default = pkgs.devshell.mkShell {
            imports = [ (pkgs.devshell.importTOML ./devshell.toml) ];
            packages = [ rustToolchain ];
          };
        };
    };
}
