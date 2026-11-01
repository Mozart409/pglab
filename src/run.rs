//! One scenario run: every cell, repeated, written as a manifest and a summary
//! (committed under `results/`) plus raw data (local only, never published).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::host::{self, HostInfo};
use crate::pgbench::{self, Outcome};
use crate::postgres::{Server, ensure_image};
use crate::spec::{Cell, Spec, Workload};
use crate::stats::{Spread, spread, to_f64};

pub struct Options {
    pub results_dir: PathBuf,
    pub raw_dir: PathBuf,
    pub repetitions: Option<u32>,
    pub duration_s: Option<u32>,
    pub warmup_s: Option<u32>,
    pub only: Vec<String>,
    pub db_cpus: Option<String>,
    pub client_cpus: Option<String>,
    pub keep_data: bool,
}

#[derive(Serialize)]
struct Manifest<'a> {
    scenario: &'a str,
    description: &'a str,
    run_id: &'a str,
    started_unix: u64,
    finished_unix: Option<u64>,
    pglab_version: &'static str,
    git: Option<Git>,
    host: &'a HostInfo,
    warnings: &'a [String],
    db_cpus: Option<&'a str>,
    client_cpus: Option<&'a str>,
    repetitions: u32,
    /// Overrides of the spec's durations; results with these are smoke runs.
    duration_s: Option<u32>,
    warmup_s: Option<u32>,
    spec: &'a Spec,
}

#[derive(Serialize)]
struct Git {
    rev: String,
    dirty: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Summary {
    pub scenario: String,
    pub run_id: String,
    pub cells: Vec<CellSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CellSummary {
    pub id: String,
    pub labels: BTreeMap<String, String>,
    pub image: String,
    pub postgres_version: String,
    pub tps: Option<Spread>,
    pub p99_us: Option<Spread>,
    /// Probe values after the measurement, and their change during it.
    pub probes: BTreeMap<String, ProbeSpread>,
    pub repetitions: Vec<Repetition>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProbeSpread {
    pub after: Option<Spread>,
    pub delta: Option<Spread>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repetition {
    pub outcome: Outcome,
    pub probes: BTreeMap<String, Probed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probed {
    pub before: Value,
    pub after: Value,
}

pub async fn run(spec: &Spec, opts: &Options) -> anyhow::Result<PathBuf> {
    let cells: Vec<&Cell> = spec
        .cells
        .iter()
        .filter(|c| opts.only.is_empty() || opts.only.contains(&c.id))
        .collect();
    anyhow::ensure!(!cells.is_empty(), "no cell matches {:?}", opts.only);

    let pinned = match &opts.db_cpus {
        Some(list) => host::parse_cpuset(list)?,
        None => Vec::new(),
    };
    let host = host::collect(&pinned);
    let warnings = host::warnings(&host);
    for w in &warnings {
        eprintln!("warning: {w}");
    }

    let started = unix_now();
    let run_id = run_id(started);
    let out_dir = opts.results_dir.join(&spec.scenario).join(&run_id);
    let raw_dir = opts.raw_dir.join(&spec.scenario).join(&run_id);
    fs::create_dir_all(&out_dir)?;
    fs::create_dir_all(&raw_dir)?;

    let repetitions = opts.repetitions.unwrap_or(spec.repetitions);
    let mut manifest = Manifest {
        scenario: &spec.scenario,
        description: &spec.description,
        run_id: &run_id,
        started_unix: started,
        finished_unix: None,
        pglab_version: env!("CARGO_PKG_VERSION"),
        git: git(),
        host: &host,
        warnings: &warnings,
        db_cpus: opts.db_cpus.as_deref(),
        client_cpus: opts.client_cpus.as_deref(),
        repetitions,
        duration_s: opts.duration_s,
        warmup_s: opts.warmup_s,
        spec,
    };
    write_json(&out_dir.join("manifest.json"), &manifest)?;

    for cell in &cells {
        ensure_image(&cell.image)?;
    }

    // Repetition-major order: rep 1 of every cell, then rep 2, ... so slow
    // drift (thermals, background work) spreads over all cells alike.
    let mut reps: BTreeMap<&str, Vec<Repetition>> = BTreeMap::new();
    for rep in 1..=repetitions {
        for cell in &cells {
            eprintln!("[{}/{repetitions}] {}", rep, cell.id);
            let dir = raw_dir.join(&cell.id).join(format!("rep-{rep}"));
            let r = repetition(spec, cell, &dir, opts)
                .await
                .with_context(|| format!("cell {} repetition {rep}", cell.id))?;
            eprintln!("  tps {:.1}", r.outcome.tps);
            reps.entry(&cell.id).or_default().push(r);
            write_json(
                &out_dir.join("summary.json"),
                &summarize(spec, &run_id, &cells, &reps),
            )?;
        }
    }

    manifest.finished_unix = Some(unix_now());
    write_json(&out_dir.join("manifest.json"), &manifest)?;
    eprintln!("raw data: {}", raw_dir.display());
    Ok(out_dir)
}

async fn repetition(
    spec: &Spec,
    cell: &Cell,
    dir: &Path,
    opts: &Options,
) -> anyhow::Result<Repetition> {
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir)?;
    let name = format!("pglab-{}-{}", spec.scenario, cell.id);
    let mut server =
        Server::start(cell, &name, spec.port, &data_dir, opts.db_cpus.as_deref()).await?;

    let measured = measure(&mut server, cell, dir, opts).await;
    fs::write(dir.join("postgres.log"), server.logs())?;
    server.stop().await?;
    if !opts.keep_data {
        fs::remove_dir_all(&data_dir)?;
    }
    measured
}

async fn measure(
    server: &mut Server,
    cell: &Cell,
    dir: &Path,
    opts: &Options,
) -> anyhow::Result<Repetition> {
    let client = opts.client_cpus.as_deref();
    for file in &cell.setup {
        server.run_file(file).await?;
    }
    let Workload::Pgbench(w) = &cell.workload;
    let warmup = opts.warmup_s.unwrap_or(w.warmup_s);
    if warmup > 0 {
        pgbench::run(server, w, warmup, None, client)?;
    }

    let before = probe_all(server, cell).await?;
    write_json(
        &dir.join("stats-before.json"),
        &server.stat_snapshot().await,
    )?;
    let outcome = pgbench::run(
        server,
        w,
        opts.duration_s.unwrap_or(w.duration_s),
        Some(dir),
        client,
    )?;
    let mut after = probe_all(server, cell).await?;
    write_json(&dir.join("stats-after.json"), &server.stat_snapshot().await)?;
    if let Ok(stats) = server.container_stats() {
        fs::write(dir.join("container-stats.json"), stats)?;
    }

    let probes = before
        .into_iter()
        .map(|(name, before)| {
            let after = after.remove(&name).unwrap_or(Value::Null);
            (name, Probed { before, after })
        })
        .collect();
    Ok(Repetition { outcome, probes })
}

async fn probe_all(server: &mut Server, cell: &Cell) -> anyhow::Result<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    for p in &cell.probes {
        let v = server
            .scalar(&p.sql)
            .await
            .with_context(|| format!("probe {}", p.name))?;
        out.insert(p.name.clone(), v);
    }
    Ok(out)
}

fn summarize(
    spec: &Spec,
    run_id: &str,
    cells: &[&Cell],
    reps: &BTreeMap<&str, Vec<Repetition>>,
) -> Summary {
    let cells = cells
        .iter()
        .filter_map(|cell| {
            let reps = reps.get(cell.id.as_str())?;
            let tps: Vec<f64> = reps.iter().map(|r| r.outcome.tps).collect();
            let p99: Vec<f64> = reps
                .iter()
                .filter_map(|r| r.outcome.latency_us.map(|l| to_f64(l.p99)))
                .collect();
            let probes = cell
                .probes
                .iter()
                .map(|p| {
                    let pairs: Vec<(f64, f64)> = reps
                        .iter()
                        .filter_map(|r| {
                            let v = r.probes.get(&p.name)?;
                            Some((v.before.as_f64()?, v.after.as_f64()?))
                        })
                        .collect();
                    let after: Vec<f64> = pairs.iter().map(|(_, a)| *a).collect();
                    let delta: Vec<f64> = pairs.iter().map(|(b, a)| a - b).collect();
                    (
                        p.name.clone(),
                        ProbeSpread {
                            after: spread(&after),
                            delta: spread(&delta),
                        },
                    )
                })
                .collect();
            Some(CellSummary {
                id: cell.id.clone(),
                labels: cell.labels.clone(),
                image: cell.image.reference.clone(),
                postgres_version: cell.image.postgres_version.clone(),
                tps: spread(&tps),
                p99_us: spread(&p99),
                probes,
                repetitions: reps.clone(),
            })
        })
        .collect();
    Summary {
        scenario: spec.scenario.clone(),
        run_id: run_id.to_owned(),
        cells,
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let mut s = serde_json::to_string_pretty(value)?;
    s.push('\n');
    fs::write(path, s).with_context(|| format!("writing {}", path.display()))
}

fn git() -> Option<Git> {
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    Some(Git {
        rev: run(&["rev-parse", "HEAD"])?,
        dirty: !run(&["status", "--porcelain", "--", "."])?.is_empty(),
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `YYYYMMDDTHHMMSSZ` in UTC: sortable, and safe in a path.
fn run_id(unix: u64) -> String {
    let days = i64::try_from(unix / 86_400).unwrap_or(0);
    let secs = unix % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::run_id;

    #[test]
    fn run_id_is_utc() {
        assert_eq!(run_id(0), "19700101T000000Z");
        assert_eq!(run_id(1_793_526_245), "20261101T094405Z");
        assert_eq!(run_id(951_782_400), "20000229T000000Z");
    }
}
