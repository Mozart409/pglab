//! The server under test: a podman container from a Nix-built image, reached
//! over loopback with one sqlx connection.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::{AssertSqlSafe, ConnectOptions, Connection};

use crate::cmd::{output, pinned};
use crate::spec::{Cell, Image, setting_arg};

/// Views snapshotted before and after the measurement (raw data only).
const STAT_VIEWS: &[&str] = &[
    "pg_stat_bgwriter",
    "pg_stat_checkpointer",
    "pg_stat_database",
    "pg_stat_io",
    "pg_stat_wal",
];

/// Load the image into podman unless this exact tag is already there.
pub fn ensure_image(image: &Image) -> anyhow::Result<()> {
    let exists = Command::new("podman")
        .args(["image", "exists", &image.reference])
        .status()
        .context("running podman")?
        .success();
    if exists {
        return Ok(());
    }
    eprintln!("loading {}", image.reference);
    let mut stream = Command::new(&image.stream)
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", image.stream.display()))?;
    let stdout = stream.stdout.take().context("image stream stdout")?;
    let load = Command::new("podman")
        .args(["load", "--quiet"])
        .stdin(stdout)
        .status()
        .context("running podman load")?;
    let streamed = stream.wait()?;
    if !streamed.success() || !load.success() {
        bail!("loading {} failed", image.reference);
    }
    Ok(())
}

pub struct Server {
    name: String,
    port: u16,
    conn: PgConnection,
}

impl Server {
    /// Start a fresh server on `data_dir` (initdb runs inside the container)
    /// and connect to it.
    pub async fn start(
        cell: &Cell,
        name: &str,
        port: u16,
        data_dir: &Path,
        db_cpus: Option<&str>,
    ) -> anyhow::Result<Self> {
        // A container left over from an interrupted run would hold the port.
        let _ = Command::new("podman")
            .args(["rm", "--force", "--ignore", name])
            .stdout(Stdio::null())
            .status();

        let mut run = pinned("podman", db_cpus);
        run.args(["run", "--detach", "--name", name, "--network=host"])
            // Files in the data dir stay owned by the invoking user.
            .arg("--userns=keep-id:uid=999,gid=999")
            .arg(format!("--shm-size={}", cell.container.shm_size))
            .arg(format!("--volume={}:/data", data_dir.display()));
        if let Some(mem) = &cell.container.memory {
            run.arg(format!("--memory={mem}"));
        }
        if cell.container.seccomp_unconfined {
            run.arg("--security-opt=seccomp=unconfined");
        }
        run.arg(&cell.image.reference);
        run.args(["-c", &format!("port={port}")]);
        for (k, v) in &cell.settings {
            run.args(["-c", &setting_arg(k, v)]);
        }
        output(&mut run)?;

        let conn = connect(name, port).await?;
        Ok(Self {
            name: name.to_owned(),
            port,
            conn,
        })
    }

    /// The server log. Postgres logs to stderr, which `podman logs` replays on
    /// its own stderr, so both streams are kept.
    pub fn logs(&self) -> String {
        logs(&self.name)
    }

    /// The `podman stats` sample of the container, as JSON (raw data only).
    pub fn container_stats(&self) -> anyhow::Result<String> {
        output(Command::new("podman").args([
            "stats",
            "--no-stream",
            "--format",
            "json",
            &self.name,
        ]))
    }

    /// Connection settings for client tools such as pgbench.
    pub fn conn_env(&self, c: &mut Command) {
        c.env("PGHOST", "127.0.0.1")
            .env("PGPORT", self.port.to_string())
            .env("PGUSER", "postgres")
            .env("PGDATABASE", "postgres");
    }

    /// Run a setup file. `raw_sql` uses the simple query protocol, so the file
    /// may hold many statements.
    pub async fn run_file(&mut self, file: &Path) -> anyhow::Result<()> {
        let sql = tokio::fs::read_to_string(file)
            .await
            .with_context(|| format!("reading {}", file.display()))?;
        // The scenario's own SQL from the Nix store: running it is the point.
        sqlx::raw_sql(AssertSqlSafe(sql))
            .execute(&mut self.conn)
            .await
            .with_context(|| format!("setup {}", file.display()))?;
        Ok(())
    }

    /// A query returning one value, as JSON (numbers stay numbers).
    pub async fn scalar(&mut self, sql: &str) -> anyhow::Result<Value> {
        let wrapped = format!(
            "select to_jsonb(t.v) from ({}) as t(v)",
            sql.trim().trim_end_matches(';')
        );
        Ok(sqlx::query_scalar::<_, Value>(AssertSqlSafe(wrapped))
            .fetch_one(&mut self.conn)
            .await?)
    }

    /// Every row of the cumulative statistics views, keyed by view name.
    pub async fn stat_snapshot(&mut self) -> Value {
        let mut snap = serde_json::Map::new();
        for view in STAT_VIEWS {
            let sql = format!("select coalesce(json_agg(t), '[]'::json) from {view} t");
            // `view` comes from the STAT_VIEWS constant.
            let value = sqlx::query_scalar::<_, Value>(AssertSqlSafe(sql))
                .fetch_one(&mut self.conn)
                .await
                .unwrap_or(Value::Null);
            snap.insert((*view).to_owned(), value);
        }
        Value::Object(snap)
    }

    pub async fn stop(self) -> anyhow::Result<()> {
        // Closing first lets the server shut down without a client attached.
        let _ = self.conn.close().await;
        output(Command::new("podman").args(["stop", "--time", "60", &self.name]))?;
        output(Command::new("podman").args(["rm", &self.name]))?;
        Ok(())
    }
}

/// Retry until initdb and startup are done, failing early if the container
/// exits.
async fn connect(name: &str, port: u16) -> anyhow::Result<PgConnection> {
    let opts = PgConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username("postgres")
        .database("postgres")
        .disable_statement_logging();
    for _ in 0..240 {
        if let Ok(conn) = opts.connect().await {
            return Ok(conn);
        }
        let running = output(Command::new("podman").args([
            "inspect",
            "--format",
            "{{.State.Running}}",
            name,
        ]))?;
        if running.trim() != "true" {
            bail!("server exited during startup:\n{}", logs(name));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    bail!("server not ready after 2 minutes:\n{}", logs(name))
}

fn logs(name: &str) -> String {
    Command::new("podman")
        .args(["logs", name])
        .output()
        .map_or_else(
            |e| e.to_string(),
            |o| {
                let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                s.push_str(&String::from_utf8_lossy(&o.stderr));
                s
            },
        )
}
