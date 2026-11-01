//! Spread across repetitions: median and interquartile range, the numbers the
//! writeups quote instead of a single best run.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Spread {
    pub median: f64,
    pub q1: f64,
    pub q3: f64,
    pub min: f64,
    pub max: f64,
    pub n: usize,
}

pub fn spread(values: &[f64]) -> Option<Spread> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(Spread {
        median: quantile(&v, 0.5),
        q1: quantile(&v, 0.25),
        q3: quantile(&v, 0.75),
        min: v[0],
        max: v[v.len() - 1],
        n: v.len(),
    })
}

/// Linear interpolation between closest ranks (type 7, numpy's default).
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let pos = q * to_f64((sorted.len() - 1) as u64);
    let lo = pos.floor();
    let frac = pos - lo;
    let i = to_usize(lo);
    sorted
        .get(i + 1)
        .map_or(sorted[i], |next| frac.mul_add(next - sorted[i], sorted[i]))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "counts and latencies in µs stay far below 2^52, where f64 is still exact"
)]
pub const fn to_f64(n: u64) -> f64 {
    n as f64
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "only called with a non-negative, already floored index"
)]
const fn to_usize(x: f64) -> usize {
    x as usize
}

#[cfg(test)]
mod tests {
    use super::spread;

    #[test]
    fn quartiles_of_five() {
        let s = spread(&[5.0, 1.0, 3.0, 2.0, 4.0]).unwrap();
        assert!((s.median - 3.0).abs() < f64::EPSILON);
        assert!((s.q1 - 2.0).abs() < f64::EPSILON);
        assert!((s.q3 - 4.0).abs() < f64::EPSILON);
        assert_eq!(s.n, 5);
    }

    #[test]
    fn empty_is_none() {
        assert!(spread(&[]).is_none());
    }
}
