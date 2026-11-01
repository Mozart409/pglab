# pglab

Reproducible PostgreSQL benchmarks. Each scenario compares server versions,
settings or extensions. Every server runs in a container built by Nix from a
pinned nixpkgs, so the same commit gives the same binaries on your machine
as on ours.

We publish summaries and writeups, not raw data. Numbers from one machine
say little about another, so run the scenarios yourself and compare.

## Scenarios

| Name | Question |
| --- | --- |
| `uuid-keys` | bigint vs uuid v4 vs uuid v7 vs ULID primary keys under an insert-only load, PostgreSQL 17 vs 18: throughput, p99, index size and density, WAL volume |

## Running one

You need Linux, Nix with flakes, and rootless podman.

```sh
# A quick check that everything runs (1 repetition per cell, 10 s each)
nix run github:Mozart409/pglab#scenarios.uuid-keys -- \
  --repetitions 1 --warmup-s 0 --duration-s 10 --results-dir /tmp/pglab-smoke

# The real thing. Pin the server and the client to separate cores, and set
# the performance governor first.
nix run github:Mozart409/pglab#scenarios.uuid-keys -- --db-cpus 2-7 --client-cpus 8-11
```

Results go to `results/<scenario>/<run-id>/`:

- `manifest.json` records the host, the warnings, the git rev and the full
  spec.
- `summary.json` has the median and interquartile range of tps, p99 and
  every probe.

Raw data goes to `$XDG_DATA_HOME/pglab/runs`: pgbench logs, `pg_stat_*`
snapshots and server logs. To get markdown tables:

```sh
nix run github:Mozart409/pglab -- report results
```

## How it works

- `nix/images.nix` builds one OCI image per PostgreSQL build with the
  extensions it needs.
- `nix/scenarios/*.nix` defines the matrix.
- `nix/matrix.nix` turns a scenario into a JSON spec and a runner.
- The Rust harness then runs these steps for each cell and repetition:
  1. Start a fresh container.
  2. Run the setup SQL, then warm up.
  3. Run pgbench with per-transaction logging, which gives HDR latency
     percentiles.
  4. Run the probe queries and snapshot `pg_stat_*` before and after.
- Repetitions are interleaved across cells, so slow drift such as heat or
  background work affects all cells alike.

## License

MIT
