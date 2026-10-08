use flow_like_ml_core::{Result, TensorData, content_digest, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Imputation {
    #[default]
    Median,
    Mean,
    Constant {
        value: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TabularOptions {
    pub numeric_columns: Vec<String>,
    pub categorical_columns: Vec<String>,
    #[serde(default)]
    pub imputation: Imputation,
    #[serde(default = "default_standardize")]
    pub standardize: bool,
    #[serde(default = "default_categories")]
    pub maximum_categories_per_column: usize,
    #[serde(default = "default_features")]
    pub maximum_output_features: usize,
}
fn default_standardize() -> bool {
    true
}
fn default_categories() -> usize {
    256
}
fn default_features() -> usize {
    4096
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FittedColumn {
    Numeric {
        name: String,
        imputation: f64,
        offset: f64,
        scale: f64,
    },
    /// Missing and unseen values have separate slots before the sorted vocabulary.
    Categorical {
        name: String,
        vocabulary: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FittedTabularPreprocessor {
    pub format_version: u32,
    pub columns: Vec<FittedColumn>,
    pub output_features: Vec<String>,
    pub training_rows: usize,
    pub training_digest: String,
}

fn cell<'a>(row: &'a Value, name: &str) -> Result<&'a Value> {
    row.as_object()
        .and_then(|row| row.get(name))
        .ok_or_else(|| flow_like_ml_core::Error::Invalid(format!("Row is missing column '{name}'")))
}
fn numeric(value: &Value, name: &str) -> Result<Option<f64>> {
    if value.is_null() {
        return Ok(None);
    }
    let value = value.as_f64().ok_or_else(|| {
        flow_like_ml_core::Error::Invalid(format!("Column '{name}' must contain numbers or null"))
    })?;
    require(
        value.is_finite(),
        format!("Column '{name}' contains a non-finite number"),
    )?;
    Ok(Some(value))
}
fn category(value: &Value, name: &str) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    require(
        value.is_string() || value.is_boolean() || value.is_number(),
        format!("Categorical column '{name}' must contain scalar values"),
    )?;
    let key = serde_json::to_string(value)?;
    require(key.len() <= 4096, "Categorical value exceeds 4096 bytes")?;
    Ok(Some(key))
}

impl FittedTabularPreprocessor {
    /// Only the supplied training indices contribute to statistics and vocabulary.
    pub fn fit(rows: &[Value], training: &[usize], options: &TabularOptions) -> Result<Self> {
        require(!training.is_empty(), "Preprocessing requires training rows")?;
        require(
            (1..=4096).contains(&options.maximum_categories_per_column)
                && (1..=65536).contains(&options.maximum_output_features),
            "Invalid preprocessing feature limits",
        )?;
        let mut indices = HashSet::new();
        require(
            training
                .iter()
                .all(|index| *index < rows.len() && indices.insert(*index)),
            "Training indices must be unique and valid",
        )?;
        let mut names = HashSet::new();
        require(
            options
                .numeric_columns
                .iter()
                .chain(&options.categorical_columns)
                .all(|name| {
                    !name.trim().is_empty() && name.len() <= 1024 && names.insert(name.as_str())
                })
                && !names.is_empty()
                && names.len() <= options.maximum_output_features,
            "Feature columns must be explicit, unique and bounded",
        )?;
        let mut columns = Vec::new();
        let mut output_features = Vec::new();
        for name in &options.numeric_columns {
            let values = training
                .iter()
                .map(|index| numeric(cell(&rows[*index], name)?, name))
                .collect::<Result<Vec<_>>>()?;
            let mut present = values.iter().flatten().copied().collect::<Vec<_>>();
            let imputation = match options.imputation {
                Imputation::Constant { value } => {
                    require(value.is_finite(), "Imputation constant must be finite")?;
                    value
                }
                Imputation::Mean => {
                    require(
                        !present.is_empty(),
                        format!(
                            "Column '{name}' is entirely missing in training; select constant imputation or remove it"
                        ),
                    )?;
                    present
                        .iter()
                        .map(|value| value / present.len() as f64)
                        .sum()
                }
                Imputation::Median => {
                    require(
                        !present.is_empty(),
                        format!(
                            "Column '{name}' is entirely missing in training; select constant imputation or remove it"
                        ),
                    )?;
                    present.sort_by(f64::total_cmp);
                    let middle = present.len() / 2;
                    if present.len() % 2 == 0 {
                        present[middle - 1] / 2.0 + present[middle] / 2.0
                    } else {
                        present[middle]
                    }
                }
            };
            let filled = values
                .iter()
                .map(|value| value.unwrap_or(imputation))
                .collect::<Vec<_>>();
            let mean = filled
                .iter()
                .map(|value| value / filled.len() as f64)
                .sum::<f64>();
            let variance = filled
                .iter()
                .map(|value| (value - mean).powi(2) / filled.len() as f64)
                .sum::<f64>();
            let (offset, scale) = if options.standardize {
                (mean, if variance > 0.0 { variance.sqrt() } else { 1.0 })
            } else {
                (0.0, 1.0)
            };
            require(
                imputation.is_finite() && offset.is_finite() && scale.is_finite() && scale > 0.0,
                "Numeric preprocessing statistics overflowed",
            )?;
            columns.push(FittedColumn::Numeric {
                name: name.clone(),
                imputation,
                offset,
                scale,
            });
            output_features.push(name.clone());
        }
        for name in &options.categorical_columns {
            let mut vocabulary = BTreeSet::new();
            for index in training {
                if let Some(value) = category(cell(&rows[*index], name)?, name)? {
                    vocabulary.insert(value);
                }
                require(
                    vocabulary.len() <= options.maximum_categories_per_column,
                    format!("Column '{name}' exceeds its category budget"),
                )?;
            }
            let vocabulary = vocabulary.into_iter().collect::<Vec<_>>();
            output_features.extend([format!("{name}:missing"), format!("{name}:unknown")]);
            output_features.extend(vocabulary.iter().map(|value| format!("{name}:{value}")));
            require(
                output_features.len() <= options.maximum_output_features,
                "Encoded features exceed the output feature budget",
            )?;
            columns.push(FittedColumn::Categorical {
                name: name.clone(),
                vocabulary,
            });
        }
        let training_values = training
            .iter()
            .map(|index| {
                options
                    .numeric_columns
                    .iter()
                    .chain(&options.categorical_columns)
                    .map(|name| cell(&rows[*index], name).cloned())
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        let result = Self {
            format_version: 1,
            columns,
            output_features,
            training_rows: training.len(),
            training_digest: content_digest(&serde_json::to_vec(&training_values)?),
        };
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<()> {
        require(
            self.format_version == 1
                && self.training_rows > 0
                && !self.columns.is_empty()
                && self.columns.len() <= 65536,
            "Invalid preprocessing manifest version or dimensions",
        )?;
        require(
            self.training_digest.len() == 64
                && self
                    .training_digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "Invalid preprocessing training digest",
        )?;
        let mut names = HashSet::new();
        let mut features = Vec::new();
        for column in &self.columns {
            let name = match column {
                FittedColumn::Numeric {
                    name,
                    imputation,
                    offset,
                    scale,
                } => {
                    require(
                        imputation.is_finite()
                            && offset.is_finite()
                            && scale.is_finite()
                            && *scale > 0.0,
                        "Invalid fitted numeric statistics",
                    )?;
                    features.push(name.clone());
                    name
                }
                FittedColumn::Categorical { name, vocabulary } => {
                    require(
                        vocabulary.len() <= 4096
                            && features.len() + vocabulary.len() + 2 <= 65536
                            && vocabulary.windows(2).all(|pair| pair[0] < pair[1])
                            && vocabulary.iter().all(|value| value.len() <= 4096),
                        "Categorical vocabulary must be sorted, unique and bounded",
                    )?;
                    features.extend([format!("{name}:missing"), format!("{name}:unknown")]);
                    features.extend(vocabulary.iter().map(|value| format!("{name}:{value}")));
                    name
                }
            };
            require(
                !name.trim().is_empty() && name.len() <= 1024 && names.insert(name),
                "Preprocessing contains duplicate or empty column names",
            )?;
        }
        require(
            features == self.output_features && features.len() <= 65536,
            "Preprocessing output schema differs from fitted columns",
        )
    }

    pub fn transform_row(&self, row: &Value) -> Result<TensorData> {
        self.validate()?;
        self.transform_validated_row(row)
    }

    fn transform_validated_row(&self, row: &Value) -> Result<TensorData> {
        let mut values = Vec::with_capacity(self.output_features.len());
        for column in &self.columns {
            match column {
                FittedColumn::Numeric {
                    name,
                    imputation,
                    offset,
                    scale,
                } => {
                    let value = ((numeric(cell(row, name)?, name)?.unwrap_or(*imputation) - offset)
                        / scale) as f32;
                    require(
                        value.is_finite(),
                        format!("Transformed column '{name}' exceeds float32 range"),
                    )?;
                    values.push(value);
                }
                FittedColumn::Categorical { name, vocabulary } => {
                    let value = category(cell(row, name)?, name)?;
                    let index = match value {
                        None => 0,
                        Some(value) => vocabulary
                            .binary_search(&value)
                            .map_or(1, |index| index + 2),
                    };
                    let start = values.len();
                    values.resize(start + vocabulary.len() + 2, 0.0);
                    values[start + index] = 1.0;
                }
            }
        }
        Ok(TensorData {
            shape: vec![values.len()],
            values,
        })
    }

    pub fn transform_rows(
        &self,
        rows: &[Value],
        maximum_elements: usize,
    ) -> Result<Vec<TensorData>> {
        self.validate()?;
        require(
            rows.len()
                .checked_mul(self.output_features.len())
                .is_some_and(|count| count <= maximum_elements),
            "Transformed dataset exceeds its element budget",
        )?;
        rows.iter()
            .map(|row| self.transform_validated_row(row))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn options() -> TabularOptions {
        TabularOptions {
            numeric_columns: vec!["value".into()],
            categorical_columns: vec!["category".into()],
            imputation: Imputation::Median,
            standardize: true,
            maximum_categories_per_column: 10,
            maximum_output_features: 20,
        }
    }

    #[test]
    fn held_out_values_never_change_scaling_or_vocabulary() {
        let rows = vec![
            json!({"value":0,"category":"A"}),
            json!({"value":2,"category":"A"}),
            json!({"value":1000,"category":"test-only"}),
        ];
        let fitted = FittedTabularPreprocessor::fit(&rows, &[0, 1], &options()).unwrap();
        assert_eq!(
            fitted.transform_row(&rows[0]).unwrap().values,
            vec![-1.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            fitted.transform_row(&rows[2]).unwrap().values,
            vec![999.0, 0.0, 1.0, 0.0]
        );
        let mut changed = rows.clone();
        changed[2] = json!({"value":-100000,"category":"different"});
        assert_eq!(
            fitted,
            FittedTabularPreprocessor::fit(&changed, &[0, 1], &options()).unwrap()
        );
        let restored: FittedTabularPreprocessor =
            serde_json::from_slice(&serde_json::to_vec(&fitted).unwrap()).unwrap();
        assert_eq!(
            restored.transform_row(&rows[2]).unwrap(),
            fitted.transform_row(&rows[2]).unwrap()
        );
    }
    #[test]
    fn imputation_and_missing_unknown_slots_are_stable_and_bounded() {
        let rows = vec![
            json!({"value":1,"category":null}),
            json!({"value":3,"category":"A"}),
            json!({"value":null,"category":"A"}),
        ];
        let fitted = FittedTabularPreprocessor::fit(&rows, &[0, 1, 2], &options()).unwrap();
        assert_eq!(
            fitted
                .transform_row(&json!({"value":null,"category":null}))
                .unwrap()
                .values,
            vec![0.0, 1.0, 0.0, 0.0]
        );
        assert!(fitted.transform_rows(&rows, 11).is_err());
        let mut corrupt = fitted;
        corrupt.output_features.clear();
        assert!(corrupt.validate().is_err());
        assert!(FittedTabularPreprocessor::fit(&rows, &[0, 0], &options()).is_err());
        assert!(
            FittedTabularPreprocessor::fit(
                &[json!({"value":null,"category":"A"})],
                &[0],
                &options()
            )
            .is_err()
        );
    }
}
