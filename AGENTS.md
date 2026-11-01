# AGENTS.md

Guidance for AI coding agents working on pglab. `README.md` is the overview
for people.

## What this is

`pglab` runs reproducible PostgreSQL benchmarks: server versions, settings and
extensions, each in a podman container built by Nix. A scenario is a Nix
attrset (`nix/scenarios/<name>.nix`). `nix/matrix.nix` turns it into a JSON
spec plus a runner, and the Rust harness (`src/`) executes every cell of it,
repeated. pgbench drives the load, and sqlx does everything else on the
database side: readiness, setup files, probes and stats snapshots.

This directory is published to GitHub (`Mozart409/pglab`, see the root
`exports.toml`). Nothing private goes in here: no homelab hostnames, no vault
content, no data from the homelab databases.

## Published vs local

- **Committed:** `results/<scenario>/<run-id>/manifest.json` and
  `summary.json` (a few KB per run), plus the writeups.
- **Never committed or published:** raw data. That means the pgbench
  per-transaction logs, `pg_stat_*` snapshots, container stats, server logs
  and data directories. They stay in `$XDG_DATA_HOME/pglab/runs`. Readers
  rerun the scenarios on their own hardware instead of downloading ours.
- Manifests must not hold local paths or hostnames. They record the CPU
  model, kernel, governors, load, temperature, the git rev and the full
  spec. The spec contains `/nix/store` paths, which are fine because they
  identify the builds.

## Environment

- Use the yggdrasil root dev shell for cargo. The `pglab-*` recipes are root
  recipes and run in this directory.
- Running a scenario needs **rootless podman** on the host. The runner brings
  pglab, pgbench (always PostgreSQL 18's, so every cell has the same client)
  and taskset, but uses the host's podman.
- Bot users without subuid ranges (e.g. `agent` on wotan) cannot run
  podman. Build and evaluate, and leave the real runs to the user.

## Build / lint / test

| Task | Command |
| --- | --- |
| Build | `just pglab-build` |
| Clippy (all + pedantic + nursery, `-D warnings`) | `just pglab-clippy` |
| Test | `just pglab-test` |
| Everything, check mode | `just pglab-ci` |
| Nix package (root flake) | `just pglab-nix-build` |
| Spec only, no run | `cd rust/pglab && nix build .#scenarios.<name>.spec` |
| Smoke run (1 rep, 10 s, results outside the repo) | `just pglab-smoke <name>` |
| Real run | `just pglab-run <name> --db-cpus 2-7 --client-cpus 8-11` |
| Markdown tables from summaries | `just pglab-report` |

Only the root lefthook runs, and it has no Rust jobs, so run `just pglab-ci`
before committing.

## How a run works

1. `pglab run <spec>` collects host state and prints preflight warnings
   (load > 1, governor not `performance`, > 80 °C). It warns and does not
   refuse: the warnings go into the manifest.
2. The image for each cell is loaded with `podman load` unless that exact
   tag exists. The tag is Nix's content hash.
3. Repetitions run in rep-major order (rep 1 of every cell, then rep 2, …)
   so drift spreads evenly. Each repetition goes through these steps:
   1. A fresh data dir and a fresh container (`--network=host`, loopback
      only, trust auth).
   2. The setup SQL files run, then the warmup, then the probes and stats
      snapshot.
   3. The measured pgbench run, with `-l` per-transaction logs that are
      folded into an HDR histogram for p50…p99.9.
   4. Probes and stats snapshot again, then the server log is saved and the
      container stopped. The data dir is removed unless `--keep-data` is
      given.
4. `summary.json` is rewritten after every repetition, so an interrupted
   run keeps what it finished.

Pinning uses `taskset`, not `--cpuset-cpus`. User sessions on wotan get only
the `cpu io memory pids` cgroup controllers, not `cpuset`, and children
inherit the affinity.

## Adding a scenario

Write `nix/scenarios/<name>.nix`, an attrset with `name`, `description`,
`repetitions` and `cells`. See `uuid-keys.nix`. Then list it in
`nix/scenarios/default.nix`. Each cell has these fields:

- `id`, `labels`, `image` (from `nix/images.nix`, or `images.mkImage`).
- `settings` (`postgresql.conf`, passed as `-c`).
- `setup` (SQL files, run with sqlx `raw_sql`, so many statements are fine).
- `workload` (pgbench scripts, clients, jobs, durations).
- `probes` (single-value queries; the harness wraps them in `to_jsonb`).
- `container` (`shm_size`, `memory`, `seccomp_unconfined`).

`nix/matrix.nix` fills in the defaults.

Extensions come from nixpkgs (`postgresqlNNPackages`). Check that an
extension exists for every major version a scenario uses.

## Planned

- `pg18-aio` scenario: PG17 vs PG18 with `io_method` set to `sync`,
  `worker` or `io_uring`. It needs a cold OS cache between repetitions,
  which in turn needs root for `drop_caches`. io_uring probably needs
  `seccomp_unconfined`; that is unverified.
- A native sqlx load driver (`Workload::Native`) for app-side key
  generation and workloads that pgbench scripts cannot express.
- perf profiles for selected cells.
- The static report and the writeups.

## Commits

Follow the root `AGENTS.md`: single-line conventional commits scoped `pglab`
(`feat(pglab): ...`).
