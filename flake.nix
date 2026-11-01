{
  description = "pglab: reproducible PostgreSQL benchmarks in Nix-built containers";

  # In the yggdrasil monorepo this is a subflake of the root flake, which makes
  # every input below follow its own, so the toolchain is the monorepo's one
  # stable Rust (rust/toolchain.nix `build` there). Standalone (the public
  # GitHub repo), flake.lock pins the same revs.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs @ {flake-parts, ...}: let
    # Latest stable, pinned by flake.lock rather than a version literal.
    # Keep the component list equal to `build` in yggdrasil's
    # rust/toolchain.nix (the root's `rust-toolchain-sync` asserts it).
    toolchainFor = system:
      inputs.fenix.packages.${system}.stable.withComponents [
        "cargo"
        "clippy"
        "rust-src"
        "rustc"
        "rustfmt"
      ];

    packageFor = pkgs: let
      toolchain = toolchainFor pkgs.stdenv.hostPlatform.system;
    in
      pkgs.callPackage ./nix/package.nix {
        rustPlatform = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };
      };
  in
    flake-parts.lib.mkFlake {inherit inputs;} {
      # Linux only: the images are Linux containers and the harness reads /proc.
      systems = ["x86_64-linux" "aarch64-linux"];

      perSystem = {
        pkgs,
        system,
        ...
      }: let
        toolchain = toolchainFor system;
        pglab = packageFor pkgs;
        images = import ./nix/images.nix {inherit pkgs;};
        runner = import ./nix/matrix.nix {inherit pkgs pglab;};
      in {
        packages = {
          default = pglab;
          inherit pglab toolchain;
          image-pg17 = images.pg17;
          image-pg18 = images.pg18;
        };

        # nix run .#scenarios.<name> -- [pglab run flags]
        # The spec alone: nix build .#scenarios.<name>.spec
        legacyPackages.scenarios =
          builtins.mapAttrs (_: runner)
          (import ./nix/scenarios {inherit pkgs images;});

        checks.package = pglab;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # keep-sorted start
            cargo-edit
            inputs.fenix.packages.${system}.stable.rust-analyzer
            postgresql_18
            toolchain
            # keep-sorted end
          ];
        };
      };
    };
}
