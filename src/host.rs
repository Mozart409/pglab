//! What the machine looked like during a run, and the preflight warnings. The
//! manifest records both, so a reader can judge a result without trusting it.

use std::fs;
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HostInfo {
    pub cpu_model: Option<String>,
    pub cpus: usize,
    pub mem_total_kib: Option<u64>,
    pub kernel: Option<String>,
    pub podman: Option<String>,
    /// Scaling governor per CPU the run is pinned to (all CPUs when unpinned).
    pub governors: Vec<String>,
    pub load_avg_1m: Option<f64>,
    pub max_temp_c: Option<f64>,
}

const LOAD_WARN: f64 = 1.0;
const TEMP_WARN_C: f64 = 80.0;

pub fn collect(pinned: &[usize]) -> HostInfo {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let cpu_model = cpuinfo
        .lines()
        .find_map(|l| l.strip_prefix("model name"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_owned());
    let cpus = std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get);
    let mem_total_kib = fs::read_to_string("/proc/meminfo").ok().and_then(|m| {
        m.lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))
            .and_then(|v| v.split_whitespace().next()?.parse().ok())
    });
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_owned());
    let podman = Command::new("podman")
        .args(["version", "--format", "{{.Client.Version}}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());

    let cpus_to_check: Vec<usize> = if pinned.is_empty() {
        (0..cpus).collect()
    } else {
        pinned.to_vec()
    };
    let mut governors: Vec<String> = cpus_to_check
        .iter()
        .filter_map(|c| {
            fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{c}/cpufreq/scaling_governor"
            ))
            .ok()
        })
        .map(|g| g.trim().to_owned())
        .collect();
    governors.sort();
    governors.dedup();

    let load_avg_1m = fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok());

    HostInfo {
        cpu_model,
        cpus,
        mem_total_kib,
        kernel,
        podman,
        governors,
        load_avg_1m,
        max_temp_c: max_temp_c(),
    }
}

fn max_temp_c() -> Option<f64> {
    let zones = fs::read_dir("/sys/class/thermal").ok()?;
    zones
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("thermal_zone"))
        .filter_map(|e| fs::read_to_string(e.path().join("temp")).ok())
        .filter_map(|t| t.trim().parse::<f64>().ok())
        .map(|milli| milli / 1000.0)
        .reduce(f64::max)
}

/// Conditions that make numbers noisy. They are warnings, not errors: the run
/// goes ahead and the manifest keeps them next to the results.
pub fn warnings(host: &HostInfo) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(load) = host.load_avg_1m
        && load > LOAD_WARN
    {
        out.push(format!(
            "1-minute load average is {load:.2} (> {LOAD_WARN}): close other work"
        ));
    }
    if host.governors.iter().any(|g| g != "performance") {
        out.push(format!(
            "CPU governor is {:?}, not performance (sudo cpupower frequency-set -g performance)",
            host.governors
        ));
    }
    if let Some(t) = host.max_temp_c
        && t > TEMP_WARN_C
    {
        out.push(format!(
            "hottest thermal zone is {t:.0} °C (> {TEMP_WARN_C} °C)"
        ));
    }
    out
}

/// Parse a cpuset list such as `2-5,8` into CPU numbers.
pub fn parse_cpuset(list: &str) -> anyhow::Result<Vec<usize>> {
    let mut cpus = Vec::new();
    for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((a, b)) = part.split_once('-') {
            let (a, b): (usize, usize) = (a.parse()?, b.parse()?);
            anyhow::ensure!(a <= b, "bad cpu range {part}");
            cpus.extend(a..=b);
        } else {
            cpus.push(part.parse()?);
        }
    }
    Ok(cpus)
}

#[cfg(test)]
mod tests {
    use super::parse_cpuset;

    #[test]
    fn cpuset_ranges_and_singles() {
        assert_eq!(parse_cpuset("2-4,8").unwrap(), vec![2, 3, 4, 8]);
        assert_eq!(parse_cpuset("0").unwrap(), vec![0]);
        assert!(parse_cpuset("5-2").is_err());
    }
}
