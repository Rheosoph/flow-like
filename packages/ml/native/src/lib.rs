pub mod calibration;
pub mod dataset;
pub mod drift;
pub mod evaluation;
pub mod feature_engineering;
pub mod models;
pub mod preprocessing;
pub mod reduction;
pub mod signals;

pub use flow_like_ml_core::{Error, Result};

pub(crate) fn validate_matrix(rows: &[Vec<f64>]) -> Result<usize> {
    flow_like_ml_core::require(
        !rows.is_empty() && !rows[0].is_empty(),
        "Feature matrix must be nonempty",
    )?;
    let width = rows[0].len();
    flow_like_ml_core::require(
        rows.iter()
            .all(|r| r.len() == width && r.iter().all(|v| v.is_finite())),
        "Feature rows must have equal width and finite values",
    )?;
    Ok(width)
}

pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub(crate) fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1u64 << 53) as f64)
    }
    pub(crate) fn index(&mut self, len: usize) -> usize {
        (self.next() % len as u64) as usize
    }
    pub(crate) fn shuffle<T>(&mut self, values: &mut [T]) {
        for i in (1..values.len()).rev() {
            values.swap(i, self.index(i + 1));
        }
    }
}
