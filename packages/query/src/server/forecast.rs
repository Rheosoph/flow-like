//! Forecasts over the rows of one statement. Lane L7 of `todo/1140-query-service` owns this file.
//! The API authorizes the model and says where each checkpoint file is; the service reads the
//! rows on the app's workbench surface, stages the checkpoint in its model cache and runs the CPU
//! forecasting engine, all within the request's deadline.

use super::ServiceConfig;
use super::ops::{FrameSink, Opened};
use crate::protocol::WireError;
use crate::protocol::forecast::ForecastOperation;
use flow_like_storage::databases::workbench::ExecutionBudget;
use std::time::Instant;

#[cfg(feature = "forecast")]
mod checkpoint;
#[cfg(feature = "forecast")]
mod run;

pub(super) async fn execute(
    _config: &ServiceConfig,
    opened: &Opened,
    operation: ForecastOperation,
    budget: &ExecutionBudget,
    sink: &FrameSink,
    warnings: Vec<String>,
    started: Instant,
) -> Result<(), WireError> {
    match operation {
        ForecastOperation::Run {
            request,
            shape,
            model,
        } => {
            #[cfg(feature = "forecast")]
            {
                let result = run::forecast(opened, request, shape, model, budget, started).await?;
                let trailer = crate::protocol::Trailer {
                    warnings,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    ..crate::protocol::Trailer::default()
                };
                super::ops::send_json(sink, &result, trailer).await
            }
            #[cfg(not(feature = "forecast"))]
            {
                let _ = (
                    opened, request, shape, model, budget, sink, warnings, started,
                );
                Err(WireError::new(
                    crate::protocol::ErrorCode::Unavailable,
                    "This query service was built without forecasting (cargo feature `forecast`)",
                ))
            }
        }
    }
}
