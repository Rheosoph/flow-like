use flow_like::flow::{
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{
    Value, async_trait,
    geometry::{GeometryKind, marker},
    json::json,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A device measurement. Geometry stays two-dimensional; altitude is in meters.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LocationFix {
    pub geometry: Value,
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    pub timestamp: f64,
    pub altitude: Option<f64>,
    pub altitude_accuracy: Option<f64>,
    pub speed: Option<f64>,
    pub heading: Option<f64>,
}

#[cfg(any(feature = "execute", test))]
impl LocationFix {
    fn validate(&self, started_at: f64, now: f64, maximum_age: f64) -> flow_like_types::Result<()> {
        use flow_like_types::{anyhow, geometry::validate_geometry};
        let require = |condition: bool, message: &str| -> flow_like_types::Result<()> {
            if condition {
                Ok(())
            } else {
                Err(anyhow!("{message}"))
            }
        };
        validate_geometry(&self.geometry, Some(GeometryKind::Point))?;
        require(
            self.latitude.is_finite()
                && (-90.0..=90.0).contains(&self.latitude)
                && self.longitude.is_finite()
                && (-180.0..=180.0).contains(&self.longitude),
            "Location coordinates are outside WGS 84 bounds",
        )?;
        let coordinates = &self.geometry["coordinates"];
        require(
            coordinates[0].as_f64() == Some(self.longitude)
                && coordinates[1].as_f64() == Some(self.latitude),
            "Location Geometry must match longitude and latitude",
        )?;
        require(
            self.accuracy.is_finite() && self.accuracy >= 0.0,
            "Invalid location accuracy",
        )?;
        require(
            self.timestamp.is_finite()
                && self.timestamp > 0.0
                // Remote workers and the requesting device can have slightly different clocks.
                && self.timestamp >= started_at - maximum_age - 5_000.0
                && self.timestamp <= now + 5_000.0,
            "Location measurement is stale or has an invalid timestamp",
        )?;
        require(
            self.altitude.is_none_or(f64::is_finite),
            "Invalid location altitude",
        )?;
        for value in [self.altitude_accuracy, self.speed] {
            require(
                value.is_none_or(|value| value.is_finite() && value >= 0.0),
                "Invalid location accuracy or speed",
            )?;
        }
        require(
            self.heading
                .is_none_or(|value| value.is_finite() && (0.0..360.0).contains(&value)),
            "Invalid location heading",
        )?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetCurrentLocationNode;

#[async_trait]
impl NodeLogic for GetCurrentLocationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geo_get_current_location",
            "Get Current Location",
            "Gets a location measurement from the local device, or from the invoking frontend for a remote Event. Requires location permission and an active app.",
            "Web/Geo/Location",
        );
        node.set_version(2);
        node.set_flowscript_name("geo", "getCurrentLocation");
        node.add_icon("/flow/icons/map.svg");
        node.set_long_running(true);
        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(7)
                .set_performance(7)
                .set_reliability(6)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "exec_in",
            "Execute",
            "Request the device location",
            VariableType::Execution,
        );
        node.add_input_pin(
            "high_accuracy",
            "High accuracy",
            "Request higher accuracy, which can use more power and take longer",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin("maximum_age_seconds", "Maximum age", "Maximum age of a cached measurement in seconds, from 0 to 300. Zero requests a fresh measurement", VariableType::Integer)
            .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "timeout_seconds",
            "Timeout",
            "Seconds allowed for permission and location acquisition, from 1 to 120",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));
        node.add_output_pin(
            "exec_success",
            "Success",
            "A valid location measurement is available",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Location permission, acquisition, or client connection failed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "geometry",
            "Geometry",
            "WGS 84 Point in longitude, latitude order",
            VariableType::Geometry,
        )
        .schema = Some(marker(GeometryKind::Point).to_string());
        node.add_output_pin("location", "Location", "Measurement with accuracy in meters, timestamp in Unix milliseconds, optional altitude in meters, speed in meters per second, and heading in degrees", VariableType::Struct)
            .set_schema::<LocationFix>();
        node.add_output_pin(
            "error",
            "Error",
            "Structured error code and message",
            VariableType::Struct,
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(
        &self,
        context: &mut flow_like::flow::execution::context::ExecutionContext,
    ) -> flow_like_types::Result<()> {
        // Operational errors belong on this node's Error branch as well as device errors.
        if let Err(error) = run_location(context).await {
            return fail(
                context,
                json!({"code":"location_failed","message":error.to_string()}),
            )
            .await;
        }
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(
        &self,
        _context: &mut flow_like::flow::execution::context::ExecutionContext,
    ) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
async fn run_location(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
) -> flow_like_types::Result<()> {
    use flow_like::flow::execution::device::local_device_command;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    context.deactivate_exec_pin("exec_success").await?;
    context.deactivate_exec_pin("exec_error").await?;
    // Unset the previous fix; null is not a Geometry value.
    let geometry = context.get_pin_by_name("geometry").await?;
    context.clear_pin_override(&geometry.id);
    geometry.reset().await;
    for pin in ["location", "error"] {
        context.set_pin_value(pin, Value::Null).await?;
    }
    let high_accuracy: bool = context.evaluate_pin("high_accuracy").await?;
    let maximum_age: i64 = context.evaluate_pin("maximum_age_seconds").await?;
    let timeout: i64 = context.evaluate_pin("timeout_seconds").await?;
    if !(0..=300).contains(&maximum_age) || !(1..=120).contains(&timeout) {
        return fail(context, json!({"code":"invalid_arguments","message":"Maximum age must be 0–300 seconds and timeout must be 1–120 seconds"})).await;
    }
    let started_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let timeout = Duration::from_secs(timeout as u64);
    let deadline = started_at.saturating_add(timeout.as_millis() as u64);
    let args = json!({"highAccuracy":high_accuracy,"maximumAgeMs":maximum_age * 1000,"timeoutMs":timeout.as_millis() as u64,"requestDeadline":deadline});
    let cancellation = context.cancellation_token().unwrap_or_default();
    let is_local = context.execution_environment().is_local();
    let operation = async {
        if is_local {
            if let Some(response) = local_device_command("location.current", args.clone()).await {
                let response = response?;
                if response["error"]["code"] != "unsupported" {
                    return Ok(response);
                }
            }
        }
        context
            .request_device("location.current", args, timeout)
            .await
    };
    let response = tokio::select! {
        biased;
        _ = cancellation.cancelled() => json!({"ok":false,"error":{"code":"cancelled","message":"Location request was cancelled"}}),
        result = tokio::time::timeout(timeout, operation) => match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => json!({"ok":false,"error":{"code":"location_failed","message":error.to_string()}}),
            Err(_) => json!({"ok":false,"error":{"code":"location_timeout","message":"Location acquisition exceeded its deadline"}}),
        },
    };
    if response["ok"] != true {
        let error = response
            .get("error")
            .filter(|error| {
                ["code", "message"].iter().all(|key| {
                    error.get(key).and_then(Value::as_str).is_some_and(|value| !value.is_empty())
                })
            })
            .cloned()
            .unwrap_or_else(|| json!({"code":"invalid_response","message":"Device did not acknowledge the location request"}));
        return fail(context, error).await;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as f64;
    let fix = flow_like_types::json::from_value::<LocationFix>(response["value"].clone())
        .map_err(flow_like_types::Error::from)
        .and_then(|fix| {
            fix.validate(started_at as f64, now, (maximum_age * 1000) as f64)?;
            Ok(fix)
        });
    let fix = match fix {
        Ok(fix) => fix,
        Err(error) => {
            return fail(
                context,
                json!({"code":"invalid_location","message":error.to_string()}),
            )
            .await;
        }
    };
    context
        .set_pin_value("geometry", fix.geometry.clone())
        .await?;
    context.set_pin_value("location", json!(fix)).await?;
    context.activate_exec_pin("exec_success").await?;
    Ok(())
}

#[cfg(feature = "execute")]
async fn fail(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    error: Value,
) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_success").await?;
    for name in ["geometry", "location"] {
        let pin = context.get_pin_by_name(name).await?;
        context.clear_pin_override(&pin.id);
        pin.reset().await;
    }
    context.set_pin_value("error", error).await?;
    context.activate_exec_pin("exec_error").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "execute")]
    async fn execution_context() -> flow_like::flow::execution::context::ExecutionContext {
        use ahash::AHashMap;
        use flow_like::{
            flow::{
                board::ExecutionStage,
                execution::{
                    LogLevel, context::ExecutionContext, internal_node::InternalNode,
                    internal_pin::InternalPin,
                },
            },
            profile::Profile,
            state::{FlowLikeConfig, FlowLikeState},
            utils::http::HTTPClient,
        };
        use flow_like_types::sync::{Mutex, RwLock};
        use std::sync::{Arc, Weak};

        let logic: Arc<dyn NodeLogic> = Arc::new(GetCurrentLocationNode);
        let node = logic.get_node();
        let mut pins = AHashMap::new();
        let mut names = AHashMap::<String, Vec<Arc<InternalPin>>>::new();
        for pin in node.pins.values() {
            let internal = Arc::new(InternalPin::new(pin, false));
            names
                .entry(pin.name.clone())
                .or_default()
                .push(internal.clone());
            pins.insert(pin.id.clone(), internal);
        }
        let current = Arc::new(InternalNode::new(node, pins, logic, names));
        for pin in current.pins.iter() {
            pin.init_node(Arc::downgrade(&current));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }
        ExecutionContext::new(
            Arc::new(AHashMap::from_iter([(
                current.node_id().to_owned(),
                current.clone(),
            )])),
            &Weak::new(),
            &Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            )),
            &current,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn node_requests_location_and_clears_a_previous_fix_before_device_failure() {
        use flow_like::flow::execution::device::set_local_device_handler;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let mut context = execution_context().await;
        let geometry = context.get_pin_by_name("geometry").await.unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        set_local_device_handler(Arc::new({
            let attempts = attempts.clone();
            let geometry = geometry.clone();
            move |command, args| {
                let attempts = attempts.clone();
                let geometry = geometry.clone();
                Box::pin(async move {
                    assert_eq!(command, "location.current");
                    assert!(geometry.get_raw_value().await.is_none());
                    if attempts.fetch_add(1, Ordering::SeqCst) > 0 {
                        return Ok(
                            json!({"ok":false,"error":{"code":"permission_denied","message":"Location access is denied"}}),
                        );
                    }
                    let mut fix = measurement();
                    fix.timestamp = args["requestDeadline"].as_f64().unwrap()
                        - args["timeoutMs"].as_f64().unwrap();
                    Ok(json!({"ok":true,"value":fix}))
                })
            }
        }));

        GetCurrentLocationNode.run(&mut context).await.unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(
            context.evaluate_pin::<Value>("geometry").await.unwrap(),
            measurement().geometry
        );
        assert!(context.evaluate_pin::<bool>("exec_success").await.unwrap());
        assert!(!context.evaluate_pin::<bool>("exec_error").await.unwrap());

        context.override_pin_value(&geometry.id, measurement().geometry);
        GetCurrentLocationNode.run(&mut context).await.unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert!(geometry.get_raw_value().await.is_none());
        assert!(context.evaluate_pin::<Value>("geometry").await.is_err());
        assert_eq!(
            context.evaluate_pin::<Value>("error").await.unwrap()["code"],
            "permission_denied"
        );
        assert!(!context.evaluate_pin::<bool>("exec_success").await.unwrap());
        assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());

        // Native transport errors, invalid replies and a sensor that never responds
        // must all finish on Error and allow the next request to succeed.
        for (reply, expected_code) in [
            (
                Some(Ok(json!({"ok":true,"value":null}))),
                "invalid_location",
            ),
            (Some(Ok(json!({}))), "invalid_response"),
            (
                Some(Ok(json!({"ok":false,"error":null}))),
                "invalid_response",
            ),
            (Some(Ok(json!({"ok":false,"error":{}}))), "invalid_response"),
            (
                Some(Err(flow_like_types::anyhow!(
                    "Native location callback failed"
                ))),
                "location_failed",
            ),
            (None, "location_timeout"),
        ] {
            let reply = Arc::new(std::sync::Mutex::new(reply));
            set_local_device_handler(Arc::new(move |_, _| {
                let reply = reply.lock().unwrap().take();
                Box::pin(async move {
                    match reply {
                        Some(reply) => reply,
                        None => std::future::pending().await,
                    }
                })
            }));
            context
                .set_pin_value("timeout_seconds", json!(1))
                .await
                .unwrap();
            flow_like::flow::execution::internal_node::InternalNode::trigger(
                &mut context,
                &mut None,
                false,
            )
            .await
            .unwrap();
            assert_eq!(
                context.evaluate_pin::<Value>("error").await.unwrap()["code"],
                expected_code
            );
            assert!(!context.evaluate_pin::<bool>("exec_success").await.unwrap());
            assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
            assert!(geometry.get_raw_value().await.is_none());
        }

        set_local_device_handler(Arc::new(|_, args| {
            Box::pin(async move {
                let mut fix = measurement();
                fix.timestamp =
                    args["requestDeadline"].as_f64().unwrap() - args["timeoutMs"].as_f64().unwrap();
                Ok(json!({"ok":true,"value":fix}))
            })
        }));
        flow_like::flow::execution::internal_node::InternalNode::trigger(
            &mut context,
            &mut None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            context.evaluate_pin::<Value>("geometry").await.unwrap(),
            measurement().geometry
        );
        assert!(context.evaluate_pin::<bool>("exec_success").await.unwrap());
        assert!(!context.evaluate_pin::<bool>("exec_error").await.unwrap());
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn invalid_location_inputs_follow_the_error_branch_and_clear_old_outputs() {
        for (pin, value, code) in [
            ("high_accuracy", json!("invalid"), "location_failed"),
            ("maximum_age_seconds", Value::Null, "location_failed"),
            ("timeout_seconds", json!(false), "location_failed"),
            ("timeout_seconds", json!(0), "invalid_arguments"),
        ] {
            let mut context = execution_context().await;
            context
                .set_pin_value("geometry", measurement().geometry)
                .await
                .unwrap();
            context.activate_exec_pin("exec_success").await.unwrap();
            context.set_pin_value(pin, value).await.unwrap();
            flow_like::flow::execution::internal_node::InternalNode::trigger(
                &mut context,
                &mut None,
                false,
            )
            .await
            .unwrap();
            assert_eq!(
                context.evaluate_pin::<Value>("error").await.unwrap()["code"],
                code
            );
            assert!(context.evaluate_pin::<Value>("geometry").await.is_err());
            assert!(!context.evaluate_pin::<bool>("exec_success").await.unwrap());
            assert!(context.evaluate_pin::<bool>("exec_error").await.unwrap());
        }
    }

    fn measurement() -> LocationFix {
        LocationFix {
            geometry: json!({"type":"Point","coordinates":[13.4,52.5]}),
            longitude: 13.4,
            latitude: 52.5,
            accuracy: 12.0,
            timestamp: 1_000_000.5,
            altitude: Some(-2.0),
            altitude_accuracy: None,
            speed: None,
            heading: None,
        }
    }

    #[test]
    fn location_outputs_typed_geometry_without_local_only_restriction() {
        let node = GetCurrentLocationNode.get_node();
        assert!(!node.only_offline);
        let pin = node
            .pins
            .values()
            .find(|pin| pin.name == "geometry")
            .unwrap();
        assert_eq!(pin.data_type, VariableType::Geometry);
        assert_eq!(pin.schema.as_deref(), Some(marker(GeometryKind::Point)));
    }

    #[test]
    fn measurement_preserves_two_dimensional_axis_order_and_nullable_metadata() {
        let fix = measurement();
        fix.validate(1_000_000.0, 1_000_100.0, 0.0).unwrap();
        let value = json!(fix);
        assert_eq!(value["geometry"]["coordinates"], json!([13.4, 52.5]));
        assert_eq!(value["altitudeAccuracy"], Value::Null);
        assert_eq!(value["altitude"], -2.0);
    }

    #[test]
    fn stale_or_future_measurements_are_rejected_but_explicit_cache_age_is_honored() {
        let fix = measurement();
        assert!(fix.validate(1_010_000.0, 1_010_100.0, 0.0).is_err());
        fix.validate(1_010_000.0, 1_010_100.0, 10_000.0).unwrap();
        fix.validate(1_001_000.0, 1_001_100.0, 0.0).unwrap();
        assert!(fix.validate(990_000.0, 990_100.0, 0.0).is_err());
    }

    #[test]
    fn mismatched_geometry_or_invalid_sensor_values_are_rejected() {
        let mut fix = measurement();
        fix.geometry = json!({"type":"Point","coordinates":[52.5,13.4]});
        assert!(fix.validate(1_000_000.0, 1_000_100.0, 0.0).is_err());
        fix = measurement();
        fix.geometry = json!({"type":"Point","coordinates":[13.4,52.5,10.0]});
        assert!(fix.validate(1_000_000.0, 1_000_100.0, 0.0).is_err());
        for (speed, heading, accuracy) in [
            (Some(-1.0), None, 2.0),
            (None, Some(360.0), 2.0),
            (None, None, -1.0),
        ] {
            fix = measurement();
            fix.speed = speed;
            fix.heading = heading;
            fix.accuracy = accuracy;
            assert!(fix.validate(1_000_000.0, 1_000_100.0, 0.0).is_err());
        }
    }
}
