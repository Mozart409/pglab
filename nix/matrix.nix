# Turns a scenario (nix/scenarios/*.nix) into the JSON spec `pglab run` reads
# and a runner around it, so `nix run .#scenarios.<name>` is the whole recipe:
# it builds the images, the harness and the spec, then runs every cell.
{
  pkgs,
  pglab,
}: scenario: let
  # Store paths, not source paths: "${p}" copies a ./file into the store.
  storePaths = map (p: "${p}");

  imageSpec = img: {
    ref = "localhost/${img.imageName}:${img.imageTag}";
    stream = "${img}";
    postgres_version = img.postgresVersion;
  };

  cellSpec = cell: {
    inherit (cell) id;
    labels = cell.labels or {};
    image = imageSpec cell.image;
    settings = cell.settings or {};
    container =
      {
        shm_size = "1g";
        memory = null;
        seccomp_unconfined = false;
      }
      // cell.container or {};
    setup = storePaths (cell.setup or []);
    workload =
      {
        kind = "pgbench";
        protocol = "prepared";
        warmup_s = 30;
        extra_args = [];
      }
      // cell.workload
      // {scripts = storePaths cell.workload.scripts;};
    probes = cell.probes or [];
  };

  spec = pkgs.writeText "pglab-${scenario.name}.json" (builtins.toJSON {
    scenario = scenario.name;
    inherit (scenario) description;
    repetitions = scenario.repetitions or 5;
    port = scenario.port or 55432;
    cells = map cellSpec scenario.cells;
  });
in
  pkgs.writeShellApplication {
    name = "pglab-${scenario.name}";
    # podman comes from the host: rootless podman needs the host's newuidmap
    # wrappers and subuid ranges. pgbench is always PG 18's, so every cell is
    # driven by the same client.
    runtimeInputs = [pglab pkgs.postgresql_18 pkgs.util-linux];
    text = ''
      exec pglab run ${spec} "$@"
    '';
    derivationArgs.passthru = {inherit spec;};
  }
