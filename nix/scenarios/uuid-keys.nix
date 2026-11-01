# Primary key types under an insert-only load: random uuid v4 against the
# time-ordered uuid v7 and ULID, with a bigint identity as the baseline, on
# PostgreSQL 17 and 18.
#
# Random keys land anywhere in the B-tree: more page splits, a sparser and
# bigger index, and more full-page images in the WAL after each checkpoint.
# Time-ordered keys append at the right edge like a sequence. The probes put
# numbers on that; tps and p99 show what it costs.
{
  pkgs,
  images,
}: let
  versions = {
    pg17 = images.pg17;
    pg18 = images.pg18;
  };

  # The key column and the extension it needs, per server version.
  keys = {
    bigint = _: {
      column = "id bigint generated always as identity primary key";
      extension = null;
    };
    uuidv4 = _: {
      column = "id uuid primary key default gen_random_uuid()";
      extension = null;
    };
    # PostgreSQL 18 has uuidv7() built in; 17 takes it from pg_uuidv7.
    uuidv7 = version:
      if version == "pg18"
      then {
        column = "id uuid primary key default uuidv7()";
        extension = null;
      }
      else {
        column = "id uuid primary key default uuid_generate_v7()";
        extension = "pg_uuidv7";
      };
    ulid = _: {
      column = "id ulid primary key default gen_ulid()";
      extension = "ulid";
    };
  };

  setupFor = version: key: let
    k = keys.${key} version;
  in
    pkgs.writeText "uuid-keys-${version}-${key}.sql" ''
      create extension if not exists pgstattuple;
      ${pkgs.lib.optionalString (k.extension != null)
        "create extension if not exists ${k.extension};"}
      create table items (
        ${k.column},
        created_at timestamptz not null default now(),
        payload text not null
      );
    '';

  cell = version: key: {
    id = "${version}-${key}";
    labels = {
      postgres = pkgs.lib.removePrefix "pg" version;
      inherit key;
    };
    image = versions.${version};
    settings = {
      shared_buffers = "2GB";
      # Room for the WAL of a whole run, so checkpoints come from
      # checkpoint_timeout (default 5 min) and not from WAL volume.
      max_wal_size = "16GB";
      track_io_timing = true;
    };
    setup = [(setupFor version key)];
    workload = {
      scripts = [../../workloads/uuid-keys/insert.sql];
      clients = 8;
      jobs = 4;
      duration_s = 180;
      warmup_s = 30;
    };
    probes = [
      {
        name = "rows";
        sql = "select count(*) from items";
      }
      {
        name = "table_bytes";
        sql = "select pg_relation_size('items')";
      }
      {
        name = "index_bytes";
        sql = "select pg_relation_size('items_pkey')";
      }
      {
        name = "leaf_density";
        sql = "select avg_leaf_density from pgstatindex('items_pkey')";
      }
      {
        name = "leaf_fragmentation";
        sql = "select leaf_fragmentation from pgstatindex('items_pkey')";
      }
      {
        name = "wal_bytes";
        sql = "select wal_bytes from pg_stat_wal";
      }
      {
        name = "wal_fpi";
        sql = "select wal_fpi from pg_stat_wal";
      }
    ];
  };
in {
  name = "uuid-keys";
  description = "Primary key types (bigint, uuid v4, uuid v7, ULID) under an insert-only load, PostgreSQL 17 vs 18";
  repetitions = 5;
  cells = pkgs.lib.concatMap (version: map (cell version) (builtins.attrNames keys)) (builtins.attrNames versions);
}
