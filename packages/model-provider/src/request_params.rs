//! Provider-native JSON overrides stored on custom model Bits.

use std::sync::Arc;

use anyhow::{Result, ensure};
use serde_json::{Map, Value};

use crate::provider::ModelProvider;

#[derive(Clone, Default)]
pub(crate) struct RequestParams(Arc<Map<String, Value>>);

impl RequestParams {
    pub(crate) fn from_provider(provider: &ModelProvider) -> Result<Self> {
        match provider
            .params
            .as_ref()
            .and_then(|params| params.get("request_params"))
        {
            Some(value) => Self::new(value.clone()),
            None => Ok(Self::default()),
        }
    }

    pub(crate) fn new(value: Value) -> Result<Self> {
        let Value::Object(params) = value else {
            anyhow::bail!("Request parameters must be a JSON object");
        };
        ensure!(
            !params.contains_key("stream"),
            "Request parameters cannot set 'stream'; the app controls streaming mode"
        );
        Ok(Self(Arc::new(params)))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Configured values win over generated fields. Objects merge recursively; arrays,
    /// scalars and null replace the existing value without passing through an SDK schema.
    pub(crate) fn apply(&self, body: &mut Value) {
        if let Some(body) = body.as_object_mut() {
            merge(body, &self.0);
        }
    }
}

fn merge(body: &mut Map<String, Value>, params: &Map<String, Value>) {
    for (key, value) in params {
        match (body.get_mut(key), value) {
            (Some(Value::Object(body)), Value::Object(params)) => merge(body, params),
            _ => {
                body.insert(key.clone(), value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_overrides_merge_objects_and_replace_other_values() {
        let params = RequestParams::new(json!({
            "max_tokens": 8192,
            "generationConfig": {"futureOption": false, "thinkingConfig": {"budget": 4096}},
            "stop": ["END"], "optional": null,
        }))
        .unwrap();
        let mut body = json!({
            "model": "model", "max_tokens": 2048,
            "generationConfig": {"temperature": 0.2, "thinkingConfig": {"budget": 100, "include": true}},
            "stop": ["STOP", "DONE"], "optional": true,
        });
        params.apply(&mut body);
        assert_eq!(
            body,
            json!({
                "model": "model", "max_tokens": 8192,
                "generationConfig": {"temperature": 0.2, "futureOption": false, "thinkingConfig": {"budget": 4096, "include": true}},
                "stop": ["END"], "optional": null,
            })
        );
    }

    #[test]
    fn invalid_objects_and_transport_overrides_are_rejected() {
        for value in [
            Value::Null,
            json!([]),
            json!("{}"),
            json!(1),
            json!({"stream": false}),
        ] {
            assert!(RequestParams::new(value).is_err());
        }
        assert!(RequestParams::new(json!({})).unwrap().is_empty());
    }
}
