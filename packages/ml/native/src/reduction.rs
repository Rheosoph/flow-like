use flow_like_ml_core::{Result, TensorData, content_digest, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, time::Instant};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FittedPca {
    pub format_version: u32,
    pub input_features: usize,
    pub mean: Vec<f64>,
    pub components: Vec<Vec<f64>>,
    pub explained_variance: Vec<f64>,
    pub explained_variance_ratio: Vec<f64>,
    pub training_rows: usize,
    pub training_digest: String,
}
impl FittedPca {
    pub fn fit(inputs: &[TensorData], training: &[usize], components: usize) -> Result<Self> {
        let started = Instant::now();
        let check_time = || {
            require(
                started.elapsed().as_secs() < 30,
                "PCA fitting exceeded its 30-second budget",
            )
        };
        require(
            (2..=100_000).contains(&training.len()),
            "PCA requires 2 to 100000 training rows",
        )?;
        let mut seen = BTreeSet::new();
        require(
            training
                .iter()
                .all(|index| *index < inputs.len() && seen.insert(*index)),
            "PCA training indices must be valid and unique",
        )?;
        let width = inputs[training[0]].values.len();
        require(
            (1..=128).contains(&width)
                && (1..=32).contains(&components)
                && components <= width
                && components < training.len(),
            "PCA supports at most 128 features and 32 components below the training row count",
        )?;
        let mut mean = vec![0.; width];
        let count = training.len() as f64;
        for index in training {
            check_time()?;
            let input = &inputs[*index];
            input.validate(128)?;
            require(
                input.shape == [width],
                "PCA inputs must be fixed-width feature vectors",
            )?;
            for (mean, value) in mean.iter_mut().zip(&input.values) {
                *mean += *value as f64 / count;
            }
        }
        let mut covariance = vec![vec![0.; width]; width];
        for index in training {
            check_time()?;
            let row = &inputs[*index].values;
            for a in 0..width {
                let x = row[a] as f64 - mean[a];
                for b in 0..=a {
                    covariance[a][b] += x * (row[b] as f64 - mean[b]) / (count - 1.);
                }
            }
        }
        for a in 0..width {
            for b in 0..a {
                covariance[b][a] = covariance[a][b];
            }
        }
        let trace = (0..width)
            .map(|index| covariance[index][index])
            .sum::<f64>();
        require(
            trace.is_finite() && trace > 0.,
            "PCA training data has no finite variation",
        )?;
        let mut vectors = vec![vec![0.; width]; width];
        for (index, row) in vectors.iter_mut().enumerate() {
            row[index] = 1.;
        }
        let tolerance = trace * 1e-12;
        let mut converged = false;
        // Cyclic Jacobi rotations diagonalize the symmetric covariance matrix without RNG.
        for _ in 0..64 {
            check_time()?;
            let mut maximum = 0f64;
            for p in 0..width {
                for q in p + 1..width {
                    let off = covariance[p][q];
                    maximum = maximum.max(off.abs());
                    if off.abs() <= tolerance {
                        continue;
                    }
                    let tau = (covariance[q][q] - covariance[p][p]) / (2. * off);
                    let t = if tau >= 0. { 1. } else { -1. } / (tau.abs() + tau.hypot(1.));
                    let cosine = 1. / (1. + t * t).sqrt();
                    let sine = t * cosine;
                    covariance[p][p] -= t * off;
                    covariance[q][q] += t * off;
                    covariance[p][q] = 0.;
                    covariance[q][p] = 0.;
                    for r in 0..width {
                        if r != p && r != q {
                            let rp = covariance[r][p];
                            let rq = covariance[r][q];
                            covariance[r][p] = cosine * rp - sine * rq;
                            covariance[p][r] = covariance[r][p];
                            covariance[r][q] = sine * rp + cosine * rq;
                            covariance[q][r] = covariance[r][q];
                        }
                        let vp = vectors[r][p];
                        let vq = vectors[r][q];
                        vectors[r][p] = cosine * vp - sine * vq;
                        vectors[r][q] = sine * vp + cosine * vq;
                    }
                }
            }
            if maximum <= tolerance {
                converged = true;
                break;
            }
        }
        require(
            converged,
            "PCA covariance did not converge within 64 Jacobi sweeps",
        )?;
        let mut order = (0..width).collect::<Vec<_>>();
        order.sort_by(|a, b| {
            covariance[*b][*b]
                .total_cmp(&covariance[*a][*a])
                .then(a.cmp(b))
        });
        let mut basis = Vec::new();
        let mut variance = Vec::new();
        for index in order.into_iter().take(components) {
            let eigenvalue = covariance[index][index];
            require(
                eigenvalue > trace * 1e-10,
                "PCA requested more components than the numerical rank of its training data",
            )?;
            let mut vector = vectors.iter().map(|row| row[index]).collect::<Vec<_>>();
            let pivot = vector
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(index, _)| index)
                .unwrap();
            if vector[pivot] < 0. {
                for value in &mut vector {
                    *value = -*value;
                }
            }
            basis.push(vector);
            variance.push(eigenvalue);
        }
        let fitted = Self {
            format_version: 1,
            input_features: width,
            mean,
            components: basis,
            explained_variance_ratio: variance.iter().map(|value| value / trace).collect(),
            explained_variance: variance,
            training_rows: training.len(),
            training_digest: content_digest(&serde_json::to_vec(
                &training
                    .iter()
                    .map(|index| &inputs[*index])
                    .collect::<Vec<_>>(),
            )?),
        };
        fitted.validate()?;
        Ok(fitted)
    }
    pub fn validate(&self) -> Result<()> {
        let count = self.components.len();
        require(
            self.format_version == 1
                && (1..=128).contains(&self.input_features)
                && (1..=32).contains(&count)
                && count <= self.input_features
                && self.training_rows > count
                && self.training_rows <= 100_000,
            "Invalid fitted PCA dimensions or version",
        )?;
        require(
            self.mean.len() == self.input_features
                && self.mean.iter().all(|value| value.is_finite())
                && self.components.iter().all(|row| {
                    row.len() == self.input_features && row.iter().all(|value| value.is_finite())
                }),
            "PCA means or components have invalid dimensions/values",
        )?;
        require(
            self.explained_variance.len() == count
                && self
                    .explained_variance
                    .iter()
                    .all(|value| value.is_finite() && *value > 0.)
                && self.explained_variance_ratio.len() == count
                && self
                    .explained_variance_ratio
                    .iter()
                    .all(|value| value.is_finite() && *value > 0. && *value <= 1. + 1e-8)
                && self.explained_variance_ratio.iter().sum::<f64>() <= 1. + 1e-8,
            "PCA variance metadata is invalid",
        )?;
        require(
            self.training_digest.len() == 64
                && self
                    .training_digest
                    .bytes()
                    .all(|value| value.is_ascii_hexdigit()),
            "PCA training digest is invalid",
        )?;
        for (a, left) in self.components.iter().enumerate() {
            for (b, right) in self.components.iter().enumerate().take(a + 1) {
                let dot = left.iter().zip(right).map(|(x, y)| x * y).sum::<f64>();
                require(
                    (dot - if a == b { 1. } else { 0. }).abs() < 1e-6,
                    "PCA components must be orthonormal",
                )?;
            }
        }
        Ok(())
    }
    fn project(&self, input: &TensorData) -> Result<TensorData> {
        input.validate(128)?;
        require(
            input.shape == [self.input_features],
            "PCA input width differs from its fitted feature schema",
        )?;
        let values = self
            .components
            .iter()
            .map(|component| {
                let value = component
                    .iter()
                    .zip(input.values.iter().zip(&self.mean))
                    .map(|(weight, (value, mean))| weight * (*value as f64 - mean))
                    .sum::<f64>() as f32;
                require(
                    value.is_finite(),
                    "Projected PCA value exceeds float32 range",
                )?;
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(TensorData {
            shape: vec![values.len()],
            values,
        })
    }
    pub fn transform(&self, input: &TensorData) -> Result<TensorData> {
        self.validate()?;
        self.project(input)
    }
    pub fn transform_rows(
        &self,
        inputs: &[TensorData],
        maximum_elements: usize,
    ) -> Result<Vec<TensorData>> {
        self.validate()?;
        require(
            inputs
                .len()
                .checked_mul(self.components.len())
                .is_some_and(|count| count <= maximum_elements),
            "PCA output exceeds tensor element budget",
        )?;
        inputs.iter().map(|row| self.project(row)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tensor(values: &[f32]) -> TensorData {
        TensorData {
            shape: vec![values.len()],
            values: values.to_vec(),
        }
    }
    #[test]
    fn pca_fits_training_only_preserves_variance_and_reconstructs_rank_one_data() {
        let mut rows = vec![
            tensor(&[-2., -4.]),
            tensor(&[-1., -2.]),
            tensor(&[1., 2.]),
            tensor(&[2., 4.]),
            tensor(&[1000., -1000.]),
        ];
        let fitted = FittedPca::fit(&rows, &[0, 1, 2, 3], 1).unwrap();
        assert!(fitted.explained_variance_ratio[0] > 1. - 1e-10);
        assert!((fitted.explained_variance[0] - 50. / 3.).abs() < 1e-9);
        for row in &rows[..4] {
            let projected = fitted.transform(row).unwrap().values[0] as f64;
            for (index, value) in row.values.iter().enumerate() {
                assert!(
                    (fitted.mean[index] + projected * fitted.components[0][index] - *value as f64)
                        .abs()
                        < 1e-5
                );
            }
        }
        rows[4] = tensor(&[-1e8, 1e8]);
        assert_eq!(FittedPca::fit(&rows, &[0, 1, 2, 3], 1).unwrap(), fitted);
        assert!(FittedPca::fit(&rows, &[0, 1, 2, 3], 2).is_err());
        let restored: FittedPca =
            serde_json::from_slice(&serde_json::to_vec(&fitted).unwrap()).unwrap();
        assert_eq!(
            restored.transform_rows(&rows, 10).unwrap(),
            fitted.transform_rows(&rows, 10).unwrap()
        );
        assert!(fitted.transform_rows(&rows, 4).is_err());
        assert!(fitted.transform(&tensor(&[1.])).is_err());
    }
    #[test]
    fn pca_orders_distinct_axes_and_rejects_invalid_fits_and_records() {
        let rows = vec![
            tensor(&[-2., -1.]),
            tensor(&[-2., 1.]),
            tensor(&[2., -1.]),
            tensor(&[2., 1.]),
        ];
        let fitted = FittedPca::fit(&rows, &[0, 1, 2, 3], 2).unwrap();
        assert!((fitted.explained_variance_ratio[0] - 0.8).abs() < 1e-10);
        assert!(fitted.components[0][0].abs() > 0.999);
        assert!(FittedPca::fit(&rows, &[0, 0], 1).is_err());
        assert!(FittedPca::fit(&[tensor(&[1.]), tensor(&[1.])], &[0, 1], 1).is_err());
        let mut corrupt = fitted.clone();
        corrupt.components[0][0] = 10.;
        assert!(corrupt.validate().is_err());
    }
}
