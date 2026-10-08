use crate::Rng;
use flow_like_ml_core::{Annotation, Error, LabelProvenance, Result, Sample, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DatasetSplit {
    pub train: Vec<usize>,
    pub validation: Vec<usize>,
    pub test: Vec<usize>,
    pub purged: Vec<usize>,
}

impl DatasetSplit {
    fn empty() -> Self {
        Self {
            train: vec![],
            validation: vec![],
            test: vec![],
            purged: vec![],
        }
    }
    fn partition_mut(&mut self, partition: usize) -> &mut Vec<usize> {
        match partition {
            0 => &mut self.train,
            1 => &mut self.validation,
            _ => &mut self.test,
        }
    }
    pub fn validate(&self, samples: &[Sample]) -> Result<()> {
        let mut rows = HashSet::new();
        let mut groups = HashMap::new();
        for (partition, indices) in [&self.train, &self.validation, &self.test]
            .iter()
            .enumerate()
        {
            for &index in indices.iter() {
                require(
                    index < samples.len() && rows.insert(index),
                    "Split contains an invalid or repeated row",
                )?;
                let group = &samples[index].group_id;
                if let Some(previous) = groups.insert(group, partition) {
                    require(
                        previous == partition,
                        "A group occurs in multiple dataset partitions",
                    )?;
                }
            }
        }
        for index in &self.purged {
            require(
                *index < samples.len() && rows.insert(*index),
                "Purged row is invalid or also selected",
            )?;
        }
        require(
            rows.len() == samples.len(),
            "Split does not account for every sample",
        )
    }
}

pub fn split_grouped(
    samples: &[Sample],
    train_ratio: f64,
    validation_ratio: f64,
    seed: u64,
) -> Result<DatasetSplit> {
    require(
        train_ratio.is_finite()
            && validation_ratio.is_finite()
            && train_ratio > 0.0
            && validation_ratio > 0.0
            && train_ratio + validation_ratio < 1.0,
        "Split ratios must leave a positive train, validation and test share",
    )?;
    let mut by_group: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, sample) in samples.iter().enumerate() {
        require(!sample.group_id.is_empty(), "Every sample needs a group id")?;
        by_group.entry(&sample.group_id).or_default().push(index);
    }
    require(
        by_group.len() >= 3,
        "At least three independent groups are required",
    )?;
    let mut groups: Vec<_> = by_group.into_values().collect();
    Rng::new(seed).shuffle(&mut groups);
    // Place large groups first; shuffled ties preserve seeded randomization.
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let targets = [
        train_ratio,
        validation_ratio,
        1.0 - train_ratio - validation_ratio,
    ]
    .map(|r| r * samples.len() as f64);
    let mut counts = [0usize; 3];
    let mut result = DatasetSplit::empty();
    let n_groups = groups.len();
    for (index, group) in groups.into_iter().enumerate() {
        let empty: Vec<_> = (0..3).filter(|p| counts[*p] == 0).collect();
        let chosen = if n_groups - index == empty.len() {
            empty[0]
        } else {
            (0..3)
                .max_by(|a, b| {
                    (targets[*a] - counts[*a] as f64).total_cmp(&(targets[*b] - counts[*b] as f64))
                })
                .unwrap()
        };
        counts[chosen] += group.len();
        result.partition_mut(chosen).extend(group);
    }
    for indices in [&mut result.train, &mut result.validation, &mut result.test] {
        indices.sort_unstable();
    }
    result.validate(samples)?;
    Ok(result)
}

pub fn split_temporal(
    samples: &[Sample],
    train_end_ms: i64,
    validation_end_ms: i64,
    embargo_ms: u64,
    label_horizon_ms: u64,
) -> Result<DatasetSplit> {
    require(
        train_end_ms < validation_end_ms,
        "Temporal cutoffs must increase",
    )?;
    let embargo = i64::try_from(embargo_ms)
        .map_err(|_| Error::Invalid("Embargo exceeds timestamp range".into()))?;
    let horizon = i64::try_from(label_horizon_ms)
        .map_err(|_| Error::Invalid("Label horizon exceeds timestamp range".into()))?;
    let validation_start = train_end_ms
        .checked_add(embargo)
        .ok_or_else(|| Error::Invalid("Temporal cutoff overflows".into()))?;
    let test_start = validation_end_ms
        .checked_add(embargo)
        .ok_or_else(|| Error::Invalid("Temporal cutoff overflows".into()))?;
    require(
        validation_start < validation_end_ms,
        "Embargo leaves no validation interval",
    )?;
    let mut candidates = Vec::with_capacity(samples.len());
    let mut group_partitions: HashMap<&str, HashSet<usize>> = HashMap::new();
    for sample in samples {
        require(
            sample.window_start_ms <= sample.window_end_ms,
            "Sample window is reversed",
        )?;
        let mut end = sample
            .window_end_ms
            .checked_add(horizon)
            .ok_or_else(|| Error::Invalid("Label horizon overflows timestamp".into()))?;
        if let Some(outcome) = &sample.outcome {
            end = end.max(outcome.target_end_ms).max(outcome.available_at_ms);
        }
        let partition = if end < train_end_ms {
            Some(0)
        } else if sample.window_start_ms >= validation_start && end < validation_end_ms {
            Some(1)
        } else if sample.window_start_ms >= test_start {
            Some(2)
        } else {
            None
        };
        if let Some(p) = partition {
            group_partitions
                .entry(&sample.group_id)
                .or_default()
                .insert(p);
        }
        candidates.push(partition);
    }
    let mut result = DatasetSplit::empty();
    for (index, partition) in candidates.into_iter().enumerate() {
        match partition {
            Some(p) if group_partitions[samples[index].group_id.as_str()].len() == 1 => {
                result.partition_mut(p).push(index)
            }
            _ => result.purged.push(index),
        }
    }
    result.validate(samples)?;
    Ok(result)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Readiness {
    pub ready: bool,
    pub eligible: usize,
    pub reviewed: usize,
    pub per_class: BTreeMap<u32, usize>,
    pub reasons: Vec<String>,
}

pub fn training_readiness(
    samples: &[Sample],
    minimum_new: usize,
    minimum_per_class: usize,
    classes: usize,
    consumed_ids: &HashSet<String>,
) -> Readiness {
    let mut result = Readiness {
        ready: false,
        eligible: 0,
        reviewed: 0,
        per_class: BTreeMap::new(),
        reasons: vec![],
    };
    let mut seen = HashSet::new();
    for sample in samples {
        if consumed_ids.contains(&sample.id)
            || !seen.insert(&sample.id)
            || matches!(sample.annotation, Annotation::Unlabeled)
        {
            continue;
        }
        result.eligible += 1;
        if matches!(
            sample.provenance,
            LabelProvenance::Reviewed { .. } | LabelProvenance::Measured { .. }
        ) {
            result.reviewed += 1;
        }
        match &sample.annotation {
            Annotation::Class { class_id } => {
                *result.per_class.entry(*class_id).or_default() += 1;
            }
            Annotation::Boxes { boxes } => {
                for class in boxes.iter().map(|b| b.class_id).collect::<HashSet<_>>() {
                    *result.per_class.entry(class).or_default() += 1;
                }
            }
            _ => {}
        }
    }
    if result.eligible < minimum_new {
        result.reasons.push(format!(
            "Need {minimum_new} new labeled examples; have {}",
            result.eligible
        ));
    }
    for class in 0..classes as u32 {
        if result.per_class.get(&class).copied().unwrap_or(0) < minimum_per_class {
            result.reasons.push(format!(
                "Class {class} has fewer than {minimum_per_class} examples"
            ));
        }
    }
    result.ready = result.reasons.is_empty();
    result
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StandardScaler {
    pub mean: Vec<f64>,
    pub scale: Vec<f64>,
    pub count: usize,
}

impl StandardScaler {
    pub fn fit(rows: &[Vec<f64>]) -> Result<Self> {
        let width = crate::validate_matrix(rows)?;
        let mut mean = vec![0.0; width];
        let mut m2 = vec![0.0; width];
        for (index, row) in rows.iter().enumerate() {
            for j in 0..width {
                let delta = row[j] - mean[j];
                mean[j] += delta / (index + 1) as f64;
                m2[j] += delta * (row[j] - mean[j]);
            }
        }
        let scale: Vec<f64> = m2
            .into_iter()
            .map(|v| (v / rows.len() as f64).sqrt())
            .map(|v| if v <= f64::EPSILON { 1.0 } else { v })
            .collect();
        require(
            mean.iter().chain(&scale).all(|v| v.is_finite()),
            "Scaler statistics overflowed",
        )?;
        Ok(Self {
            mean,
            scale,
            count: rows.len(),
        })
    }
    pub fn transform(&self, row: &[f64]) -> Result<Vec<f64>> {
        require(
            row.len() == self.mean.len()
                && self.mean.len() == self.scale.len()
                && row.iter().all(|v| v.is_finite()),
            "Scaler input shape or values are invalid",
        )?;
        require(
            self.mean.iter().all(|v| v.is_finite())
                && self.scale.iter().all(|s| s.is_finite() && *s > 0.0),
            "Scaler state is invalid",
        )?;
        Ok(row
            .iter()
            .zip(&self.mean)
            .zip(&self.scale)
            .map(|((v, m), s)| (v - m) / s)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_ml_core::{Outcome, TensorData};
    fn sample(index: usize, group: usize, start: i64, end: i64) -> Sample {
        Sample {
            id: index.to_string(),
            group_id: group.to_string(),
            stream_id: "camera".into(),
            timestamp_ms: start,
            window_start_ms: start,
            window_end_ms: end,
            input: TensorData {
                shape: vec![1],
                values: vec![index as f32],
            },
            annotation: Annotation::Class {
                class_id: (index % 2) as u32,
            },
            provenance: LabelProvenance::Measured {
                source: "test".into(),
            },
            outcome: None,
        }
    }
    #[test]
    fn grouped_split_has_no_leakage_and_no_twenty_thousand_cap() {
        let samples: Vec<_> = (0..20_017)
            .map(|i| sample(i, i / 7, i as i64, i as i64))
            .collect();
        let split = split_grouped(&samples, 0.7, 0.15, 42).unwrap();
        assert_eq!(
            split.train.len() + split.validation.len() + split.test.len(),
            20_017
        );
        assert_eq!(split, split_grouped(&samples, 0.7, 0.15, 42).unwrap());
        split.validate(&samples).unwrap();
    }
    #[test]
    fn temporal_split_purges_overlap_future_targets_and_crossing_groups() {
        let mut samples = vec![
            sample(0, 0, 0, 10),
            sample(1, 1, 80, 95),
            sample(2, 2, 115, 125),
            sample(3, 3, 220, 230),
            sample(4, 4, 40, 50),
            sample(5, 4, 240, 250),
        ];
        samples[1].outcome = Some(Outcome {
            available_at_ms: 105,
            target_start_ms: 95,
            target_end_ms: 104,
        });
        let split = split_temporal(&samples, 100, 200, 10, 0).unwrap();
        assert_eq!(split.train, vec![0]);
        assert_eq!(split.validation, vec![2]);
        assert_eq!(split.test, vec![3]);
        assert_eq!(split.purged, vec![1, 4, 5]);
        let horizon = split_temporal(&samples, 100, 200, 10, 100).unwrap();
        assert!(horizon.train.is_empty());
    }
    #[test]
    fn scaler_reuses_training_statistics() {
        let fit = StandardScaler::fit(&[vec![1.0, 9.0], vec![3.0, 9.0]]).unwrap();
        assert_eq!(fit.transform(&[5.0, 9.0]).unwrap(), vec![3.0, 0.0]);
    }
}
