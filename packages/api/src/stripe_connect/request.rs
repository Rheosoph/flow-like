use std::collections::BTreeMap;

use flow_like_types::reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{STRIPE_API_VERSION, gateway::StripeError};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripeScope {
    pub platform_account_id: String,
    pub connected_account_id: Option<String>,
    pub livemode: bool,
}

impl StripeScope {
    pub fn platform(platform_account_id: impl Into<String>, livemode: bool) -> Self {
        Self {
            platform_account_id: platform_account_id.into(),
            connected_account_id: None,
            livemode,
        }
    }

    pub fn connected(&self, account: impl Into<String>) -> Self {
        Self {
            connected_account_id: Some(account.into()),
            ..self.clone()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RequestMethod {
    Get,
    Post,
}

/// Persist this value before executing a mutation. Bearer URLs can occur in parameters.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripeRequest {
    pub method: RequestMethod,
    pub path: String,
    pub scope: StripeScope,
    pub api_version: String,
    pub parameters: Value,
    pub idempotency_key: Option<String>,
}

impl std::fmt::Debug for StripeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StripeRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("scope", &self.scope)
            .field("api_version", &self.api_version)
            .finish_non_exhaustive()
    }
}

impl StripeRequest {
    pub fn get(scope: StripeScope, path: impl Into<String>, parameters: Value) -> Self {
        Self {
            method: RequestMethod::Get,
            path: path.into(),
            scope,
            api_version: STRIPE_API_VERSION.into(),
            parameters,
            idempotency_key: None,
        }
    }

    pub fn post(
        scope: StripeScope,
        path: impl Into<String>,
        parameters: Value,
        idempotency_key: impl Into<String>,
    ) -> Self {
        Self {
            method: RequestMethod::Post,
            path: path.into(),
            scope,
            api_version: STRIPE_API_VERSION.into(),
            parameters,
            idempotency_key: Some(idempotency_key.into()),
        }
    }

    pub fn validate(&self) -> Result<(), StripeError> {
        if !valid_id(&self.scope.platform_account_id, "acct_")
            || self
                .scope
                .connected_account_id
                .as_ref()
                .is_some_and(|id| !valid_id(id, "acct_"))
        {
            return Err(StripeError::invalid("invalid Stripe account scope"));
        }
        if !self.path.starts_with("/v1/")
            || self.path.len() > 512
            || self.path.split('/').skip(2).any(|part| {
                part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
        {
            return Err(StripeError::invalid("invalid Stripe API path"));
        }
        if self.api_version.is_empty()
            || self.api_version.len() > 64
            || !self
                .api_version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
        {
            return Err(StripeError::invalid("invalid Stripe API version"));
        }
        if !self.parameters.is_object() {
            return Err(StripeError::invalid("Stripe parameters must be an object"));
        }
        if self.method == RequestMethod::Post && self.idempotency_key.is_none() {
            return Err(StripeError::invalid(
                "Stripe mutation requires an idempotency key",
            ));
        }
        if let Some(key) = &self.idempotency_key
            && (key.is_empty() || key.len() > 255 || !key.bytes().all(|b| (33..=126).contains(&b)))
        {
            return Err(StripeError::invalid("invalid Stripe idempotency key"));
        }
        self.form_pairs()?;
        Ok(())
    }

    /// Includes account, mode and version so reuse cannot cross a financial boundary.
    pub fn digest(&self) -> Result<String, StripeError> {
        self.validate()?;
        let value = serde_json::to_value(self)
            .map_err(|_| StripeError::invalid("cannot serialize Stripe operation"))?;
        let canonical = canonical_json(&value);
        Ok(hex::encode(Sha256::digest(canonical.as_bytes())))
    }

    pub fn form_pairs(&self) -> Result<Vec<(String, String)>, StripeError> {
        let object = self
            .parameters
            .as_object()
            .ok_or_else(|| StripeError::invalid("Stripe parameters must be an object"))?;
        let mut pairs = Vec::new();
        for (key, value) in object {
            validate_parameter_key(key)?;
            flatten(key, value, &mut pairs, 0)?;
        }
        pairs.sort();
        Ok(pairs)
    }

    pub fn encoded_form(&self) -> Result<String, StripeError> {
        let mut url = Url::parse("https://api.stripe.com/")
            .map_err(|_| StripeError::invalid("invalid Stripe URL"))?;
        url.query_pairs_mut().extend_pairs(self.form_pairs()?);
        Ok(url.query().unwrap_or_default().to_owned())
    }
}

pub fn valid_id(id: &str, prefix: &str) -> bool {
    id.starts_with(prefix)
        && id.len() > prefix.len()
        && id.len() <= 255
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn validate_parameter_key(key: &str) -> Result<(), StripeError> {
    if key.is_empty() || key.bytes().any(|b| matches!(b, b'[' | b']' | 0)) {
        return Err(StripeError::invalid("invalid nested Stripe parameter key"));
    }
    Ok(())
}

fn flatten(
    key: &str,
    value: &Value,
    pairs: &mut Vec<(String, String)>,
    depth: usize,
) -> Result<(), StripeError> {
    if depth > 16 || pairs.len() > 4096 {
        return Err(StripeError::invalid(
            "Stripe parameters exceed nesting or size limit",
        ));
    }
    match value {
        Value::Null => {}
        Value::Bool(v) => pairs.push((key.into(), v.to_string())),
        Value::Number(v) if v.is_u64() || v.is_i64() => {
            pairs.push((key.into(), v.to_string()));
        }
        Value::Number(_) => return Err(StripeError::invalid("Stripe amounts require integers")),
        Value::String(v) => pairs.push((key.into(), v.clone())),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                flatten(&format!("{key}[{index}]"), value, pairs, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for (name, value) in values {
                validate_parameter_key(name)?;
                flatten(&format!("{key}[{name}]"), value, pairs, depth + 1)?;
            }
        }
    }
    Ok(())
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let sorted: BTreeMap<_, _> = object.iter().collect();
            format!(
                "{{{}}}",
                sorted
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("JSON object key"),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(array) => format!(
            "[{}]",
            array
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(parameters: Value) -> StripeRequest {
        StripeRequest::post(
            StripeScope::platform("acct_platform", false),
            "/v1/checkout/sessions",
            parameters,
            "payment:one",
        )
    }

    #[test]
    fn nested_form_preserves_metadata_and_indexes() {
        let request = request(json!({
            "controller": {"fees": {"payer": "account"}},
            "line_items": [{"quantity": 1, "price_data": {"unit_amount": 11900}}],
            "metadata": {"description": "A&B = café"}, "omit": null,
            "automatic_tax": {"enabled": true}
        }));
        let encoded = request.encoded_form().unwrap();
        assert!(encoded.contains("controller%5Bfees%5D%5Bpayer%5D=account"));
        assert!(encoded.contains("line_items%5B0%5D%5Bquantity%5D=1"));
        assert!(encoded.contains("metadata%5Bdescription%5D=A%26B+%3D+caf%C3%A9"));
        assert!(!encoded.contains("omit"));
        assert!(encoded.contains("automatic_tax%5Benabled%5D=true"));
    }

    #[test]
    fn digest_is_canonical_and_scoped() {
        let a = request(json!({"b": {"y": 2, "x": 1}, "a": 10}));
        let b = request(json!({"a": 10, "b": {"x": 1, "y": 2}}));
        assert_eq!(a.digest().unwrap(), b.digest().unwrap());
        let mut different = a.clone();
        different.scope.livemode = true;
        assert_ne!(a.digest().unwrap(), different.digest().unwrap());
        different = a.clone();
        different.scope.connected_account_id = Some("acct_seller".into());
        assert_ne!(a.digest().unwrap(), different.digest().unwrap());
    }

    #[test]
    fn rejects_ambiguous_paths_keys_floats_and_unkeyed_mutations() {
        for path in [
            "https://other.example/v1/refunds",
            "/v1/../refunds",
            "/v1/refunds?x=1",
        ] {
            let mut value = request(json!({}));
            value.path = path.into();
            assert!(value.validate().is_err());
        }
        assert!(request(json!({"amount": 1.1})).validate().is_err());
        assert!(
            request(json!({"metadata[owner]": "other"}))
                .validate()
                .is_err()
        );
        let mut value = request(json!({}));
        value.idempotency_key = None;
        assert!(value.validate().is_err());
    }
}
