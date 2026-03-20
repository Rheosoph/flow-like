use crate::error::{Result, SecretError};
use secrecy::{SecretBox, SecretString};
use std::sync::Arc;

#[derive(Clone)]
pub enum SecretValue {
    Text(Arc<SecretString>),
    Binary(Arc<SecretBox<[u8]>>),
}

impl SecretValue {
    pub fn text(secret: SecretString) -> Self {
        Self::Text(Arc::new(secret))
    }

    pub fn binary(secret: SecretBox<[u8]>) -> Self {
        Self::Binary(Arc::new(secret))
    }

    pub fn from_string(value: String) -> Self {
        Self::text(SecretString::new(value.into_boxed_str()))
    }

    pub fn from_bytes(value: Vec<u8>) -> Self {
        Self::binary(SecretBox::new(value.into_boxed_slice()))
    }

    pub fn as_text(&self) -> Result<Arc<SecretString>> {
        match self {
            SecretValue::Text(value) => Ok(Arc::clone(value)),
            SecretValue::Binary(_) => Err(SecretError::SecretValueBinary),
        }
    }

    pub fn as_bytes(&self) -> Result<Arc<SecretBox<[u8]>>> {
        match self {
            SecretValue::Binary(value) => Ok(Arc::clone(value)),
            SecretValue::Text(_) => Err(SecretError::SecretValueText),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must_ok<T, E: std::fmt::Display>(result: std::result::Result<T, E>, context: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{context}: {error}"),
        }
    }

    #[test]
    fn returns_text_variant() {
        use secrecy::ExposeSecret;

        let value = SecretValue::from_string("abc".to_string());
        let secret = must_ok(value.as_text(), "must be text");
        assert_eq!(secret.expose_secret(), "abc");
    }

    #[test]
    fn binary_rejects_text_accessor() {
        let value = SecretValue::from_bytes(vec![1, 2, 3]);
        match value.as_text() {
            Ok(_) => panic!("binary value must reject text accessor"),
            Err(error) => assert_eq!(error, SecretError::SecretValueBinary),
        }
    }
}
