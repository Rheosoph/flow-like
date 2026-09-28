use std::sync::Arc;

use flow_like::hub::{MailConfig, MailProviderType};
use flow_like_types::Result;

#[cfg(feature = "acs-email")]
mod azure_communication_services;
#[cfg(feature = "sendgrid")]
mod sendgrid;
#[cfg(feature = "ses")]
mod ses;
#[cfg(feature = "smtp")]
mod smtp;
pub mod templates;

#[cfg(feature = "acs-email")]
pub use azure_communication_services::AzureCommunicationServicesMailClient;
#[cfg(feature = "sendgrid")]
pub use sendgrid::SendgridMailClient;
#[cfg(feature = "ses")]
pub use ses::SesMailClient;
#[cfg(feature = "smtp")]
pub use smtp::SmtpMailClient;

#[derive(Clone)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
}

/// RFC 3834 marker that keeps automated mail out of auto-responder loops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSubmitted {
    Generated,
    Replied,
}

impl AutoSubmitted {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Generated => "auto-generated",
            Self::Replied => "auto-replied",
        }
    }
}

#[derive(Clone)]
pub struct AutomationEmailMessage {
    /// Resolved by the API from the authorized event's active address.
    pub from_email: Option<String>,
    /// Display name for `from_email`; providers fall back to their configured name.
    pub from_name: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub reply_to: Option<String>,
    pub subject: String,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub auto_submitted: Option<AutoSubmitted>,
}

impl AutomationEmailMessage {
    /// Custom headers every provider must set: threading and the RFC 3834 marker.
    pub(super) fn headers(&self) -> Result<std::collections::BTreeMap<String, String>> {
        let mut headers: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        if let Some(value) = &self.in_reply_to {
            headers.insert("In-Reply-To".into(), value.clone());
        }
        if !self.references.is_empty() {
            headers.insert("References".into(), self.references.join(" "));
        }
        if let Some(value) = self.auto_submitted {
            headers.insert("Auto-Submitted".into(), value.as_str().into());
        }
        if let Some((name, _)) = headers.iter().find(|(name, value)| {
            value.is_empty()
                || name.len() + value.len() > 996
                || !value.is_ascii()
                || value.bytes().any(|byte| byte.is_ascii_control())
        }) {
            return Err(flow_like_types::anyhow!("Invalid email header {name}"));
        }
        Ok(headers)
    }
}

/// RFC 5322 mailbox with a quoted or RFC 2047 encoded display name.
#[cfg(any(feature = "ses", test))]
pub(crate) fn display_address(name: &str, email: &str) -> String {
    use base64::Engine;
    let name = name.chars().filter(|c| !c.is_control()).collect::<String>();
    let name = name.trim();
    if name.is_empty() {
        return email.to_owned();
    }
    if name.is_ascii() {
        let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
        return format!("\"{quoted}\" <{email}>");
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in name.chars() {
        if chunk.len() + c.len_utf8() > 45 {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    words.push(chunk);
    let encoded = words
        .iter()
        .map(|word| {
            format!(
                "=?UTF-8?B?{}?=",
                base64::engine::general_purpose::STANDARD.encode(word)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("{encoded} <{email}>")
}

impl From<EmailMessage> for AutomationEmailMessage {
    fn from(message: EmailMessage) -> Self {
        Self {
            from_email: None,
            from_name: None,
            to: vec![message.to],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: None,
            subject: message.subject,
            body_html: message.body_html,
            body_text: message.body_text,
            in_reply_to: None,
            references: Vec::new(),
            auto_submitted: None,
        }
    }
}

#[async_trait::async_trait]
#[allow(clippy::wrong_self_convention)] // "from" is the RFC5322 From: header/display name, not a constructor prefix
pub trait MailClient: Send + Sync {
    async fn send(&self, message: EmailMessage) -> Result<()> {
        self.send_automation(message.into()).await
    }
    async fn send_automation(&self, message: AutomationEmailMessage) -> Result<()>;
    fn from_email(&self) -> &str;
    fn from_name(&self) -> &str;
}

pub type DynMailClient = Arc<dyn MailClient>;

pub async fn create_mail_client(config: &MailConfig) -> Result<DynMailClient> {
    let provider = runtime_provider_override()?.unwrap_or(config.provider);

    match provider {
        MailProviderType::Ses => {
            #[cfg(feature = "ses")]
            {
                let client = SesMailClient::new(config).await?;
                Ok(Arc::new(client))
            }
            #[cfg(not(feature = "ses"))]
            {
                Err(flow_like_types::anyhow!("SES feature not enabled"))
            }
        }
        MailProviderType::Smtp => {
            #[cfg(feature = "smtp")]
            {
                let smtp_settings = config.smtp.as_ref().ok_or_else(|| {
                    flow_like_types::anyhow!("SMTP settings required for SMTP provider")
                })?;
                let client = SmtpMailClient::new(config, smtp_settings)?;
                Ok(Arc::new(client))
            }
            #[cfg(not(feature = "smtp"))]
            {
                Err(flow_like_types::anyhow!("SMTP feature not enabled"))
            }
        }
        MailProviderType::Sendgrid => {
            #[cfg(feature = "sendgrid")]
            {
                let sendgrid_settings = config.sendgrid.as_ref().ok_or_else(|| {
                    flow_like_types::anyhow!("Sendgrid settings required for Sendgrid provider")
                })?;
                let client = SendgridMailClient::new(config, sendgrid_settings)?;
                Ok(Arc::new(client))
            }
            #[cfg(not(feature = "sendgrid"))]
            {
                Err(flow_like_types::anyhow!("Sendgrid feature not enabled"))
            }
        }
        MailProviderType::AzureCommunicationServices => {
            #[cfg(feature = "acs-email")]
            {
                let client = AzureCommunicationServicesMailClient::new(config)?;
                Ok(Arc::new(client))
            }
            #[cfg(not(feature = "acs-email"))]
            {
                Err(flow_like_types::anyhow!(
                    "Azure Communication Services Email feature not enabled"
                ))
            }
        }
    }
}

pub(crate) fn runtime_provider_override() -> Result<Option<MailProviderType>> {
    let Ok(raw_provider) = std::env::var("MAIL_PROVIDER") else {
        return Ok(None);
    };

    let provider = match raw_provider.trim().to_ascii_lowercase().as_str() {
        "ses" => MailProviderType::Ses,
        "smtp" => MailProviderType::Smtp,
        "sendgrid" => MailProviderType::Sendgrid,
        "azure_communication_services" | "acs_email" => {
            MailProviderType::AzureCommunicationServices
        }
        _ => {
            return Err(flow_like_types::anyhow!(
                "unsupported MAIL_PROVIDER; expected ses, smtp, sendgrid, or azure_communication_services"
            ));
        }
    };

    Ok(Some(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn reply_message() -> AutomationEmailMessage {
        AutomationEmailMessage {
            from_email: Some("event@example.com".into()),
            from_name: Some("Order Desk".into()),
            to: vec!["author@example.net".into()],
            cc: vec!["copy@example.net".into()],
            bcc: vec!["hidden@example.net".into()],
            reply_to: None,
            subject: "Re: Hello".into(),
            body_html: None,
            body_text: Some("Reply body".into()),
            in_reply_to: Some("<parent@example.net>".into()),
            references: vec!["<first@example.net>".into(), "<parent@example.net>".into()],
            auto_submitted: Some(AutoSubmitted::Replied),
        }
    }

    #[test]
    fn transactional_mail_keeps_its_configured_sender_and_has_no_thread_headers() {
        let message: AutomationEmailMessage = EmailMessage {
            to: "recipient@example.net".into(),
            subject: "Notice".into(),
            body_html: None,
            body_text: Some("Body".into()),
        }
        .into();
        assert!(message.from_email.is_none() && message.from_name.is_none());
        assert!(message.headers().unwrap().is_empty());
        assert!(message.cc.is_empty() && message.bcc.is_empty());
    }

    #[test]
    fn reply_headers_reject_injection_and_ses_length_overflow() {
        let mut message = reply_message();
        let headers = message.headers().unwrap();
        assert_eq!(
            headers["References"],
            "<first@example.net> <parent@example.net>"
        );
        assert_eq!(headers["Auto-Submitted"], "auto-replied");
        message.in_reply_to = Some("<parent@example.net>\r\nBcc: other@example.com".into());
        assert!(message.headers().is_err());
        message.in_reply_to = None;
        message.references = vec!["x".repeat(996 - "References".len())];
        assert!(message.headers().is_ok());
        message.references[0].push('x');
        assert!(message.headers().is_err());
    }

    #[test]
    fn automation_marks_every_message_as_auto_submitted() {
        let mut message = reply_message();
        message.auto_submitted = Some(AutoSubmitted::Generated);
        assert_eq!(
            message.headers().unwrap()["Auto-Submitted"],
            "auto-generated"
        );
    }

    #[test]
    fn display_names_are_quoted_or_encoded_and_never_inject_headers() {
        assert_eq!(
            display_address("Order Desk", "event@example.com"),
            "\"Order Desk\" <event@example.com>"
        );
        assert_eq!(
            display_address(
                "Say \"hi\" \\ bye\r\nBcc: x@example.com",
                "event@example.com"
            ),
            "\"Say \\\"hi\\\" \\\\ byeBcc: x@example.com\" <event@example.com>"
        );
        assert_eq!(
            display_address("  ", "event@example.com"),
            "event@example.com"
        );
        let encoded = display_address(&"Bestellungen ü".repeat(6), "event@example.com");
        assert!(encoded.ends_with(" <event@example.com>"));
        assert!(encoded.is_ascii());
        assert!(
            encoded
                .trim_end_matches(" <event@example.com>")
                .split(' ')
                .all(|word| word.starts_with("=?UTF-8?B?") && word.len() <= 75)
        );
    }
}
