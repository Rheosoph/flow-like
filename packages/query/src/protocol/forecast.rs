//! Time-series forecasts over the rows of one statement, and their backtests. Lane L7 of
//! `todo/1140-query-service` owns this file.

use super::QueryShape;
use flow_like_types::Value;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ForecastOperation {
    /// Reads the request's SQL (and future SQL) on one app's workbench surface, stages `model`
    /// and forecasts the rows. Answers one JSON `StudioForecastResult`. The rows never leave the
    /// service, so the forecast's own budget caps them, not `limits.rows`.
    Run {
        /// The forecasting engine's `StudioForecastRequest` as the API received it: SQL, params,
        /// column mapping, horizon, quantiles, backtest window and budget.
        request: Value,
        /// `workbench`: the surface and saved views the SQL reads.
        shape: QueryShape,
        model: ForecastModelSpec,
    },
}

/// A Forecasting Bit's checkpoint as the API authorized it: the family that loads it and where
/// each file can be fetched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastModelSpec {
    pub bit_id: String,
    /// The engine's family name, such as `chronos2` or `timesfm2_5`.
    pub family: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    pub files: Vec<ModelFile>,
}

impl ForecastModelSpec {
    pub fn total_bytes(&self) -> u64 {
        self.files
            .iter()
            .fold(0, |total, file| total.saturating_add(file.size))
    }
}

/// One checkpoint file. The service keeps fetched files between requests, keyed by content.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    /// Name inside the checkpoint directory: `config.json`, weights, an index or a shard.
    pub name: String,
    /// The Bit's content hash: the lowercase hex BLAKE3 of the bytes when `verify` is set,
    /// otherwise a stable identity that only keys the cache.
    pub hash: String,
    #[serde(default)]
    pub verify: bool,
    pub size: u64,
    /// A short-lived signed storage URL or the Bit's download link. It can carry credentials,
    /// so it never appears in logs, errors or `Debug` output.
    pub url: String,
}

impl fmt::Debug for ModelFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelFile")
            .field("name", &self.name)
            .field("hash", &self.hash)
            .field("verify", &self.verify)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Operation, SurfaceSpec, ViewSpec};
    use serde_json::json;

    fn spec() -> ForecastModelSpec {
        ForecastModelSpec {
            bit_id: "chronos".to_string(),
            family: "chronos2".to_string(),
            repository: Some("amazon/chronos-2".to_string()),
            revision: None,
            files: vec![
                ModelFile {
                    name: "config.json".to_string(),
                    hash: "c".repeat(64),
                    verify: true,
                    size: 1_024,
                    url: "https://bucket.example/config.json?X-Amz-Signature=secret".to_string(),
                },
                ModelFile {
                    name: "model.safetensors".to_string(),
                    hash: "weights".to_string(),
                    verify: false,
                    size: 480_000_000,
                    url: "https://hub.example/model.safetensors".to_string(),
                },
            ],
        }
    }

    #[test]
    fn forecast_operations_round_trip_under_their_tags() {
        let operation = Operation::Forecast {
            forecast: ForecastOperation::Run {
                request: json!({ "model_bit_id": "chronos", "sql": "SELECT 1", "horizon": 4 }),
                shape: QueryShape::Workbench {
                    surface: SurfaceSpec::Native,
                    views: vec![ViewSpec {
                        name: "sales".to_string(),
                        sql: "SELECT * FROM orders".to_string(),
                    }],
                },
                model: spec(),
            },
        };
        let wire = serde_json::to_value(&operation).unwrap();
        assert_eq!(wire["op"], "forecast");
        assert_eq!(wire["forecast"]["kind"], "run");
        assert_eq!(wire["forecast"]["shape"]["kind"], "workbench");
        assert_eq!(wire["forecast"]["model"]["files"][1]["verify"], false);

        let Operation::Forecast {
            forecast:
                ForecastOperation::Run {
                    request,
                    shape,
                    model,
                },
        } = serde_json::from_value(wire).unwrap()
        else {
            panic!("the operation changed kind");
        };
        assert_eq!(request["horizon"], 4);
        assert!(matches!(shape, QueryShape::Workbench { views, .. } if views.len() == 1));
        assert_eq!(model, spec());
    }

    #[test]
    fn optional_model_fields_default_when_absent() {
        let model: ForecastModelSpec = serde_json::from_value(json!({
            "bit_id": "ttm",
            "family": "tiny_time_mixer",
            "files": [{ "name": "config.json", "hash": "h", "size": 3, "url": "https://x" }],
        }))
        .unwrap();
        assert_eq!(model.repository, None);
        assert!(!model.files[0].verify);
        assert_eq!(model.total_bytes(), 3);
        assert_eq!(spec().total_bytes(), 480_001_024);
    }

    #[test]
    fn debug_output_never_shows_file_urls() {
        let printed = format!("{:?}", spec());
        assert!(printed.contains("model.safetensors"));
        assert!(!printed.contains("X-Amz-Signature"));
        assert!(!printed.contains("https://"));
    }
}
