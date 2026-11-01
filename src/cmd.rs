//! Running external tools (podman, psql, pgbench) with useful errors.

use std::process::{Command, Stdio};

use anyhow::{Context, bail};

/// Run to completion and return stdout; stderr goes into the error on failure.
pub fn output(cmd: &mut Command) -> anyhow::Result<String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("starting {}", describe(cmd)))?;
    if !out.status.success() {
        bail!(
            "{} failed ({}):\n{}",
            describe(cmd),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Wrap a command in `taskset -c <cpus>` when pinning was asked for. Children
/// inherit the affinity, so this pins podman's conmon and the container too
/// (rootless podman cannot use --cpuset-cpus: user sessions do not get the
/// cpuset cgroup controller delegated).
pub fn pinned(program: &str, cpus: Option<&str>) -> Command {
    cpus.map_or_else(
        || Command::new(program),
        |cpus| {
            let mut c = Command::new("taskset");
            c.args(["-c", cpus, program]);
            c
        },
    )
}

fn describe(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().into_owned();
    for a in cmd.get_args().take(4) {
        s.push(' ');
        s.push_str(&a.to_string_lossy());
    }
    s
}
