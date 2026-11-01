//! The run spec: what `nix/matrix.nix` writes for one scenario. Scenarios are
//! defined in Nix only; this side never invents defaults beyond the CLI
//! overrides in `main.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Spec {
    pub scenario: String,
    pub description: String,
    pub repetitions: u32,
    /// Loopback port the server listens on (the container uses host networking).
    pub port: u16,
    pub cells: Vec<Cell>,
}

/// One point of the matrix: an image, its settings and a workload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Cell {
    pub id: String,
    /// Free-form dimensions for the report, e.g. `{"postgres": "18", "key": "uuidv7"}`.
    pub labels: BTreeMap<String, String>,
    pub image: Image,
    /// `postgresql.conf` settings, passed to the server as `-c name=value`.
    pub settings: BTreeMap<String, Value>,
    pub container: Container,
    /// SQL files run with psql, in order, before the warmup.
    pub setup: Vec<PathBuf>,
    pub workload: Workload,
    /// Single-value queries run after the warmup and after the measurement.
    pub probes: Vec<Probe>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Image {
    /// `localhost/<name>:<tag>`, the tag being Nix's content hash.
    #[serde(rename = "ref")]
    pub reference: String,
    /// The `streamLayeredImage` script that writes the image tarball to stdout.
    pub stream: PathBuf,
    pub postgres_version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Container {
    pub shm_size: String,
    pub memory: Option<String>,
    /// `io_uring` syscalls are outside podman's default seccomp profile.
    pub seccomp_unconfined: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Workload {
    Pgbench(Pgbench),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Pgbench {
    pub scripts: Vec<PathBuf>,
    pub clients: u32,
    pub jobs: u32,
    pub duration_s: u32,
    pub warmup_s: u32,
    /// `simple`, `extended` or `prepared`.
    pub protocol: String,
    /// Appended to the pgbench command line as is.
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Probe {
    pub name: String,
    pub sql: String,
}

/// A setting as the server takes it on the command line.
pub fn setting_arg(name: &str, value: &Value) -> String {
    match value {
        Value::String(s) => format!("{name}={s}"),
        Value::Bool(b) => format!("{name}={}", if *b { "on" } else { "off" }),
        other => format!("{name}={other}"),
    }
}
