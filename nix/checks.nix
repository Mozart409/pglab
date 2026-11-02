# What `just pglab-ci` runs (fmt, clippy, tests) as crane checks, called from
# the yggdrasil root flake with its crane and the monorepo's one stable Rust:
#
#   import ./rust/pglab/nix/checks.nix {inherit pkgs crane toolchain;}
#
# As Nix builds they are cached by their inputs: a push that does not change
# this source finds them already in the store, and the rust-ci pre-push gate
# (the root's legacyPackages.rust-ci.pglab) costs an evaluation instead of a
# compile. The standalone flake does not use this file (it has no crane input).
#
# Returns `checks` (fmt/clippy/test) and `cargoArtifacts`.
{
  pkgs,
  crane,
  toolchain,
}: let
  inherit (pkgs) lib;

  craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

  # The same source as nix/package.nix: editing a scenario, a workload or a
  # result does not rerun the checks.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
    ];
  };

  commonArgs = {
    inherit src;
    pname = "pglab";
    version = (lib.importTOML ../Cargo.toml).package.version;
    strictDeps = true;
    # The dev profile `cargo test` uses, so the tests keep debug assertions and
    # overflow checks (crane defaults to release).
    CARGO_PROFILE = "dev";
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in {
  inherit cargoArtifacts;

  checks = {
    fmt = craneLib.cargoFmt {
      inherit src;
      pname = "pglab-fmt";
    };

    clippy = craneLib.cargoClippy (commonArgs
      // {
        inherit cargoArtifacts;
        pname = "pglab-clippy";
        # Kept equal to `just pglab-clippy`.
        cargoClippyExtraArgs = "--all-targets -- -D warnings";
      });

    test = craneLib.cargoTest (commonArgs
      // {
        inherit cargoArtifacts;
        pname = "pglab-test";
      });
  };
}
