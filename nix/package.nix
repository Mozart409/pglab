{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "pglab";
  # Read out of Cargo.toml rather than duplicated here.
  version = (lib.importTOML ../Cargo.toml).package.version;

  # Only what the build reads, so editing a scenario, a workload or a result
  # does not rebuild the harness.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  meta = {
    description = "Reproducible PostgreSQL benchmarks: versions, settings and extensions in Nix-built containers";
    mainProgram = "pglab";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
  };
}
