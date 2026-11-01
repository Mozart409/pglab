//! Markdown tables from committed summaries, the starting point of a writeup.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::run::Summary;
use crate::stats::Spread;

/// Every `summary.json` under `dir`, oldest run first.
pub fn markdown(dir: &Path) -> anyhow::Result<String> {
    let mut files = Vec::new();
    collect(dir, &mut files)?;
    files.sort();
    anyhow::ensure!(!files.is_empty(), "no summary.json under {}", dir.display());

    let mut out = String::new();
    for file in files {
        let summary: Summary = serde_json::from_str(&fs::read_to_string(&file)?)
            .with_context(|| format!("parsing {}", file.display()))?;
        table(&mut out, &summary)?;
    }
    Ok(out)
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else if path.file_name().is_some_and(|n| n == "summary.json") {
            files.push(path);
        }
    }
    Ok(())
}

fn table(out: &mut String, s: &Summary) -> anyhow::Result<()> {
    let probes: Vec<&String> = s
        .cells
        .first()
        .map(|c| c.probes.keys().collect())
        .unwrap_or_default();

    writeln!(out, "## {} ({})\n", s.scenario, s.run_id)?;
    write!(out, "| cell | postgres | tps | p99 µs |")?;
    for p in &probes {
        write!(out, " {p} |")?;
    }
    write!(out, "\n| --- | --- | ---: | ---: |")?;
    for _ in &probes {
        write!(out, " ---: |")?;
    }
    writeln!(out)?;

    for c in &s.cells {
        write!(
            out,
            "| {} | {} | {} | {} |",
            c.id,
            c.postgres_version,
            fmt(c.tps.as_ref()),
            fmt(c.p99_us.as_ref())
        )?;
        for p in &probes {
            let after = c.probes.get(*p).and_then(|v| v.after.as_ref());
            write!(out, " {} |", fmt(after))?;
        }
        writeln!(out)?;
    }
    writeln!(
        out,
        "\nMedian over repetitions, interquartile range in brackets. Probes are \
         the values after the measurement.\n"
    )?;
    Ok(())
}

fn fmt(s: Option<&Spread>) -> String {
    s.map_or_else(
        || "–".to_owned(),
        |s| format!("{:.1} [{:.1}–{:.1}]", s.median, s.q1, s.q3),
    )
}
