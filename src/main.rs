//! pglab: run a Nix-defined PostgreSQL benchmark scenario and summarize it.
//!
//! `nix run .#scenarios.<name>` builds the images and the run spec, then calls
//! `pglab run <spec.json>`. Summaries go to `results/` (committed), raw data to
//! `$XDG_DATA_HOME/pglab/runs` (never published).

mod cmd;
mod host;
mod pgbench;
mod postgres;
mod report;
mod run;
mod spec;
mod stats;

use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run every cell of a scenario spec.
    Run {
        /// The spec JSON written by nix/matrix.nix.
        spec: PathBuf,
        /// Where manifest.json and summary.json go (committed).
        #[arg(long, default_value = "results")]
        results_dir: PathBuf,
        /// Where raw data goes [default: `$XDG_DATA_HOME/pglab/runs`].
        #[arg(long)]
        raw_dir: Option<PathBuf>,
        /// Override the spec's repetitions.
        #[arg(long)]
        repetitions: Option<u32>,
        /// Override the measured duration (smoke runs).
        #[arg(long)]
        duration_s: Option<u32>,
        /// Override the warmup duration (smoke runs).
        #[arg(long)]
        warmup_s: Option<u32>,
        /// Run only these cells (repeatable).
        #[arg(long)]
        only: Vec<String>,
        /// Pin the server to these CPUs, e.g. `2-7`.
        #[arg(long)]
        db_cpus: Option<String>,
        /// Pin pgbench to these CPUs, e.g. `8-11`.
        #[arg(long)]
        client_cpus: Option<String>,
        /// Keep each repetition's data directory.
        #[arg(long)]
        keep_data: bool,
    },
    /// Print the host state and the preflight warnings, then exit.
    Preflight {
        #[arg(long)]
        db_cpus: Option<String>,
    },
    /// Print markdown tables for every summary.json under a directory.
    Report {
        #[arg(default_value = "results")]
        dir: PathBuf,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Cmd::Run {
            spec,
            results_dir,
            raw_dir,
            repetitions,
            duration_s,
            warmup_s,
            only,
            db_cpus,
            client_cpus,
            keep_data,
        } => {
            let spec: spec::Spec = serde_json::from_str(
                &fs::read_to_string(&spec)
                    .with_context(|| format!("reading {}", spec.display()))?,
            )?;
            let opts = run::Options {
                results_dir,
                raw_dir: raw_dir.map_or_else(default_raw_dir, Ok)?,
                repetitions,
                duration_s,
                warmup_s,
                only,
                db_cpus,
                client_cpus,
                keep_data,
            };
            let out = run::run(&spec, &opts).await?;
            println!("{}", out.display());
        }
        Cmd::Preflight { db_cpus } => {
            let pinned = db_cpus
                .as_deref()
                .map_or_else(|| Ok(Vec::new()), host::parse_cpuset)?;
            let info = host::collect(&pinned);
            println!("{}", serde_json::to_string_pretty(&info)?);
            for w in host::warnings(&info) {
                eprintln!("warning: {w}");
            }
        }
        Cmd::Report { dir } => print!("{}", report::markdown(&dir)?),
    }
    Ok(())
}

fn default_raw_dir() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .context("neither XDG_DATA_HOME nor HOME is set")?;
    Ok(base.join("pglab/runs"))
}
