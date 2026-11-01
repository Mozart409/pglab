# The servers under test: one OCI image per PostgreSQL build, from nixpkgs and
# pinned by flake.lock, so a reader who runs the same commit gets the same
# bits. `pglab run` loads an image into podman on first use; the tag is Nix's
# content hash, so a changed build is a new tag, never a stale one.
{pkgs}: let
  # initdb and postgres refuse to run as root. The harness maps this uid to
  # the invoking user (--userns=keep-id:uid=999,gid=999), so the data
  # directory on the host stays owned by them.
  nss = pkgs.dockerTools.fakeNss.override {
    extraPasswdLines = ["postgres:x:999:999:postgres:/var/empty:/bin/sh"];
    extraGroupLines = ["postgres:x:999:"];
  };

  # name: image name suffix; postgresql: a nixpkgs postgresql_NN;
  # extensions: ps -> [ps.foo], as for postgresql.withPackages.
  mkImage = {
    name,
    postgresql,
    extensions ? _: [],
  }: let
    server = postgresql.withPackages extensions;
    # initdb on first start, then the server in the foreground. The harness
    # mounts the data volume at /data and appends its `-c name=value`
    # settings. Trust auth is safe here: the server only listens on loopback.
    entrypoint = pkgs.writeShellApplication {
      name = "pglab-entrypoint";
      runtimeInputs = [server];
      text = ''
        export PGDATA=/data/pgdata
        if [ ! -s "$PGDATA/PG_VERSION" ]; then
          initdb --pgdata="$PGDATA" --username=postgres --auth=trust \
            --no-locale --encoding=UTF8 --no-instructions >/dev/null
        fi
        exec postgres -D "$PGDATA" -c listen_addresses=127.0.0.1 \
          -c unix_socket_directories=/tmp "$@"
      '';
    };
  in
    pkgs.dockerTools.streamLayeredImage {
      name = "pglab-${name}";
      contents = [nss];
      extraCommands = ''
        mkdir -p -m 1777 tmp
        mkdir -p data
      '';
      config = {
        User = "postgres";
        Entrypoint = ["${entrypoint}/bin/pglab-entrypoint"];
      };
      passthru.postgresVersion = postgresql.version;
    };
in {
  inherit mkImage;

  # The extensions the scenarios need; one image per major version keeps the
  # number of image builds small. A scenario that needs a different set calls
  # mkImage itself.
  pg17 = mkImage {
    name = "pg17";
    postgresql = pkgs.postgresql_17;
    extensions = ps: [ps.pg_uuidv7 ps.pgx_ulid];
  };
  pg18 = mkImage {
    name = "pg18";
    postgresql = pkgs.postgresql_18;
    extensions = ps: [ps.pg_uuidv7 ps.pgx_ulid];
  };
}
