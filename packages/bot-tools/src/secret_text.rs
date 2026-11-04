use std::collections::BTreeSet;

use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult};

const OPEN: &str = "{{secret:";
const CLOSE: &str = "}}";
const MIN_ENCODED_BYTES: usize = 8;

fn percent_encoded(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub fn placeholder(name: &str) -> String {
    format!("{OPEN}{name}{CLOSE}")
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// The secret names a text asks for, in order of appearance.
pub fn secret_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else { break };
        let name = &after[..end];
        if valid_name(name) && !names.iter().any(|known| known == name) {
            names.push(name.to_string());
        }
        rest = &after[end + CLOSE.len()..];
    }
    names
}

/// Secrets a tool call put into the world, so every text it returns can be cleaned of them.
#[derive(Debug, Default, Clone)]
pub struct Scrubber {
    values: Vec<(String, String)>,
}

impl Scrubber {
    pub async fn load(env: &dyn ToolEnv, names: &BTreeSet<String>) -> Self {
        let mut scrubber = Self::default();
        for name in names {
            if let Ok(Some(value)) = env.secret(name).await {
                scrubber.add(name, value);
            }
        }
        scrubber
    }

    /// Learns `value` and, when it is long enough not to match by chance, the URL-encoded and
    /// base64 forms a program prints it in.
    pub fn add(&mut self, name: &str, value: String) {
        if value.is_empty() {
            return;
        }
        let mut forms = Vec::with_capacity(3);
        if value.len() >= MIN_ENCODED_BYTES {
            forms.push(percent_encoded(&value));
            forms.push(STANDARD.encode(value.as_bytes()));
        }
        forms.push(value);
        for form in forms {
            if !self.values.iter().any(|(_, known)| *known == form) {
                self.values.push((name.to_string(), form));
            }
        }
        self.values
            .sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn scrub(&self, text: &str) -> String {
        let mut clean = text.to_string();
        for (name, value) in &self.values {
            if clean.contains(value.as_str()) {
                clean = clean.replace(value.as_str(), &placeholder(name));
            }
        }
        clean
    }

    pub fn scrub_value(&self, value: &mut Value) {
        if self.is_empty() {
            return;
        }
        match value {
            Value::String(text) => *text = self.scrub(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.scrub_value(item)),
            Value::Object(fields) => fields.values_mut().for_each(|item| self.scrub_value(item)),
            _ => {}
        }
    }

    pub fn scrub_error(&self, mut error: ToolError) -> ToolError {
        error.message = self.scrub(&error.message);
        if let Some(details) = error.details.as_mut() {
            self.scrub_value(details);
        }
        error
    }
}

/// Replaces `{{secret:NAME}}` with the bot's secret. The value goes to the page or the keyboard
/// and nowhere else: the scrubber learns it so no output repeats it.
pub async fn fill_secrets(
    env: &dyn ToolEnv,
    text: &str,
    scrubber: &mut Scrubber,
    used: &mut BTreeSet<String>,
) -> ToolResult<String> {
    let mut filled = text.to_string();
    for name in secret_names(text) {
        let value = env
            .secret(&name)
            .await
            .map_err(ToolError::from)?
            .ok_or_else(|| {
                ToolError::not_found(format!(
                    "The bot has no secret named '{name}'; call secret_list for the stored names, or secret_request to ask the owner for it"
                ))
            })?;
        filled = filled.replace(&placeholder(&name), &value);
        scrubber.add(&name, value);
        used.insert(name);
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_read_in_order_and_once() {
        assert_eq!(
            secret_names("{{secret:A}} x {{secret:B-1}} {{secret:A}} {{secret:bad name}}"),
            vec!["A".to_string(), "B-1".to_string()]
        );
    }

    #[test]
    fn longer_secrets_are_replaced_before_their_prefixes() {
        let mut scrubber = Scrubber::default();
        scrubber.add("short", "pass".into());
        scrubber.add("long", "pass-word".into());
        assert_eq!(
            scrubber.scrub("pass-word and pass"),
            "{{secret:long}} and {{secret:short}}"
        );
    }

    #[test]
    fn encoded_forms_of_a_long_secret_are_scrubbed_too() {
        let mut scrubber = Scrubber::default();
        scrubber.add("key", "s3cr3t/p@ss".into());
        let encoded = percent_encoded("s3cr3t/p@ss");
        let base64 = STANDARD.encode("s3cr3t/p@ss");
        assert_eq!(encoded, "s3cr3t%2Fp%40ss");
        assert_eq!(
            scrubber.scrub(&format!("{encoded} {base64} s3cr3t/p@ss")),
            "{{secret:key}} {{secret:key}} {{secret:key}}"
        );
        let mut short = Scrubber::default();
        short.add("pin", "1234".into());
        assert_eq!(short.scrub("MTIzNA== 1234"), "MTIzNA== {{secret:pin}}");
    }

    #[test]
    fn an_empty_secret_is_ignored() {
        let mut scrubber = Scrubber::default();
        scrubber.add("none", String::new());
        assert!(scrubber.is_empty());
        assert_eq!(scrubber.scrub("text"), "text");
    }
}
