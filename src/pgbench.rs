//! pgbench as a load driver. Throughput comes from its summary; latency
//! percentiles come from its per-transaction log (`-l`), folded into an HDR
//! histogram, because pgbench itself only reports the mean and stddev.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, bail};
use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

use crate::cmd::pinned;
use crate::postgres::Server;
use crate::spec::Pgbench;

const LOG_PREFIX: &str = "txn";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub tps: f64,
    pub transactions: u64,
    pub failed: u64,
    pub latency_us: Option<Latency>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Latency {
    pub mean: f64,
    pub p50: u64,
    pub p90: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

/// Run pgbench for `duration_s`. With `log_dir`, the per-transaction log and
/// pgbench's own output land there and latency percentiles are computed.
pub fn run(
    server: &Server,
    w: &Pgbench,
    duration_s: u32,
    log_dir: Option<&Path>,
    client_cpus: Option<&str>,
) -> anyhow::Result<Outcome> {
    let mut cmd = pinned("pgbench", client_cpus);
    server.conn_env(&mut cmd);
    cmd.args(["--no-vacuum", "--progress=10"])
        .arg(format!("--protocol={}", w.protocol))
        .arg(format!("--client={}", w.clients))
        .arg(format!("--jobs={}", w.jobs))
        .arg(format!("--time={duration_s}"));
    for script in &w.scripts {
        cmd.arg("--file").arg(script);
    }
    if let Some(dir) = log_dir {
        cmd.arg("--log")
            .arg(format!("--log-prefix={}", dir.join(LOG_PREFIX).display()));
    }
    cmd.args(&w.extra_args);

    let out = cmd.output().context("running pgbench")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if let Some(dir) = log_dir {
        fs::write(dir.join("pgbench.out"), format!("{stdout}\n{stderr}"))?;
    }
    if !out.status.success() {
        bail!("pgbench failed ({}):\n{}", out.status, stderr.trim());
    }

    let mut outcome = parse_summary(&stdout)?;
    if let Some(dir) = log_dir {
        outcome.latency_us = Some(latency_from_logs(dir)?);
    }
    Ok(outcome)
}

fn parse_summary(stdout: &str) -> anyhow::Result<Outcome> {
    let field = |prefix: &str| {
        stdout
            .lines()
            .find_map(|l| l.trim().strip_prefix(prefix))
            .and_then(|rest| rest.split_whitespace().next())
            .map(|v| v.split('/').next().unwrap_or(v).to_owned())
    };
    let tps = field("tps = ")
        .context("no tps line in pgbench output")?
        .parse()?;
    let transactions = field("number of transactions actually processed:")
        .context("no transaction count in pgbench output")?
        .parse()?;
    let failed = field("number of failed transactions:").map_or(Ok(0), |v| v.parse())?;
    Ok(Outcome {
        tps,
        transactions,
        failed,
        latency_us: None,
    })
}

/// Fold every `txn.*` log in `dir` into one histogram. The third field of a
/// log line is the latency in µs, or `failed`/`skipped`.
fn latency_from_logs(dir: &Path) -> anyhow::Result<Latency> {
    let mut hist = Histogram::<u64>::new(3)?;
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let is_log = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(&format!("{LOG_PREFIX}.")));
        if !is_log {
            continue;
        }
        for line in BufReader::new(File::open(&path)?).lines() {
            if let Some(us) = line?.split_whitespace().nth(2).and_then(|f| f.parse().ok()) {
                hist.record(us)?;
            }
        }
    }
    anyhow::ensure!(!hist.is_empty(), "no transactions in the pgbench logs");
    Ok(Latency {
        mean: hist.mean(),
        p50: hist.value_at_quantile(0.5),
        p90: hist.value_at_quantile(0.9),
        p95: hist.value_at_quantile(0.95),
        p99: hist.value_at_quantile(0.99),
        p999: hist.value_at_quantile(0.999),
        max: hist.max(),
    })
}

#[cfg(test)]
mod tests {
    use super::parse_summary;

    const SAMPLE: &str = "\
transaction type: /nix/store/x-insert.sql
scaling factor: 1
query mode: prepared
number of clients: 8
number of threads: 4
maximum number of tries: 1
duration: 10 s
number of transactions actually processed: 123456
number of failed transactions: 3 (0.002%)
latency average = 0.648 ms
initial connection time = 4.211 ms
tps = 12345.678901 (without initial connection time)
";

    #[test]
    fn summary_fields() {
        let o = parse_summary(SAMPLE).unwrap();
        assert!((o.tps - 12_345.678_901).abs() < 1e-6);
        assert_eq!(o.transactions, 123_456);
        assert_eq!(o.failed, 3);
    }
}
