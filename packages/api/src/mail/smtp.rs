use flow_like::hub::{MailConfig, SmtpSettings};
use flow_like_types::Result;
use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    address::Envelope,
    message::{
        Mailbox, MultiPart, SinglePart,
        header::{ContentType, HeaderName, HeaderValue},
    },
    transport::smtp::authentication::Credentials,
};

use super::{AutomationEmailMessage, MailClient};

pub struct SmtpMailClient {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from_email: String,
    from_name: String,
    envelope_sender: String,
}

impl SmtpMailClient {
    pub fn new(config: &MailConfig, smtp: &SmtpSettings) -> Result<Self> {
        let host = std::env::var(&smtp.host_env)
            .map_err(|_| flow_like_types::anyhow!("SMTP host env var {} not set", smtp.host_env))?;
        let port: u16 = std::env::var(&smtp.port_env)
            .map_err(|_| flow_like_types::anyhow!("SMTP port env var {} not set", smtp.port_env))?
            .parse()
            .map_err(|_| flow_like_types::anyhow!("Invalid SMTP port"))?;
        let username = std::env::var(&smtp.username_env).map_err(|_| {
            flow_like_types::anyhow!("SMTP username env var {} not set", smtp.username_env)
        })?;
        let password = std::env::var(&smtp.password_env).map_err(|_| {
            flow_like_types::anyhow!("SMTP password env var {} not set", smtp.password_env)
        })?;

        Self::from_settings(config, &host, port, username, password, false)
    }

    pub(crate) fn from_settings(
        config: &MailConfig,
        host: &str,
        port: u16,
        username: String,
        password: String,
        starttls: bool,
    ) -> Result<Self> {
        let transport = if starttls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::relay(host)
        }
        .map_err(|e| flow_like_types::anyhow!("Failed to create SMTP transport: {}", e))?
        .port(port)
        .credentials(Credentials::new(username, password))
        .build();

        Ok(Self {
            transport,
            from_email: config.from_email.clone(),
            from_name: config.from_name.clone(),
            envelope_sender: config.from_email.clone(),
        })
    }

    /// Bounces go to this address instead of the platform sender.
    pub(crate) fn with_envelope_sender(mut self, address: String) -> Self {
        self.envelope_sender = address;
        self
    }
}

fn parse_address(label: &str, address: &str) -> Result<Address> {
    address
        .parse()
        .map_err(|e| flow_like_types::anyhow!("Invalid {label} address {address:?}: {e}"))
}

fn build_message(
    message: &AutomationEmailMessage,
    from_email: &str,
    from_name: &str,
    envelope_sender: &str,
) -> Result<Message> {
    let from = Mailbox::new(
        Some(message.from_name.as_deref().unwrap_or(from_name).to_owned()),
        parse_address("from", message.from_email.as_deref().unwrap_or(from_email))?,
    );
    message.headers()?;

    // The envelope never uses the event address, so bounces cannot trigger a flow.
    let recipients = message
        .to
        .iter()
        .chain(&message.cc)
        .chain(&message.bcc)
        .map(|address| parse_address("recipient", address))
        .collect::<Result<Vec<_>>>()?;
    let envelope = Envelope::new(
        Some(parse_address("envelope sender", envelope_sender)?),
        recipients,
    )
    .map_err(|e| flow_like_types::anyhow!("Invalid SMTP envelope: {}", e))?;

    let mut email_builder = Message::builder()
        .from(from)
        .envelope(envelope)
        .subject(&message.subject);
    for to in &message.to {
        email_builder = email_builder.to(to.parse()?);
    }
    for cc in &message.cc {
        email_builder = email_builder.cc(cc.parse()?);
    }
    for bcc in &message.bcc {
        email_builder = email_builder.bcc(bcc.parse()?);
    }
    if let Some(reply_to) = &message.reply_to {
        email_builder = email_builder.reply_to(reply_to.parse()?);
    }
    if let Some(in_reply_to) = &message.in_reply_to {
        email_builder = email_builder.in_reply_to(in_reply_to.clone());
    }
    if !message.references.is_empty() {
        email_builder = email_builder.references(message.references.join(" "));
    }
    if let Some(auto_submitted) = message.auto_submitted {
        email_builder = email_builder.raw_header(HeaderValue::new(
            HeaderName::new_from_ascii_str("Auto-Submitted"),
            auto_submitted.as_str().to_owned(),
        ));
    }

    let email = match (&message.body_html, &message.body_text) {
        (Some(html), Some(text)) => email_builder
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(text.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(html.clone()),
                    ),
            )
            .map_err(|e| flow_like_types::anyhow!("Failed to build email: {}", e))?,
        (Some(html), None) => email_builder
            .header(ContentType::TEXT_HTML)
            .body(html.clone())
            .map_err(|e| flow_like_types::anyhow!("Failed to build email: {}", e))?,
        (None, Some(text)) => email_builder
            .header(ContentType::TEXT_PLAIN)
            .body(text.clone())
            .map_err(|e| flow_like_types::anyhow!("Failed to build email: {}", e))?,
        (None, None) => {
            return Err(flow_like_types::anyhow!(
                "Email must have either HTML or text body"
            ));
        }
    };

    Ok(email)
}

#[async_trait::async_trait]
impl MailClient for SmtpMailClient {
    async fn send_automation(&self, message: AutomationEmailMessage) -> Result<()> {
        self.transport
            .send(build_message(
                &message,
                &self.from_email,
                &self.from_name,
                &self.envelope_sender,
            )?)
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to send email via SMTP: {}", e))?;

        Ok(())
    }

    fn from_email(&self) -> &str {
        &self.from_email
    }

    fn from_name(&self) -> &str {
        &self.from_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_reply_preserves_thread_and_keeps_bcc_in_envelope_only() {
        let message = build_message(
            &crate::mail::tests::reply_message(),
            "platform@example.com",
            "Platform",
            "platform@example.com",
        )
        .unwrap();
        let raw = String::from_utf8(message.formatted()).unwrap();
        assert!(raw.contains("From: \"Order Desk\" <event@example.com>\r\n"));
        assert!(raw.contains("In-Reply-To: <parent@example.net>\r\n"));
        assert!(raw.contains("References: <first@example.net> <parent@example.net>\r\n"));
        assert!(raw.contains("Auto-Submitted: auto-replied\r\n"));
        assert!(!raw.contains("Bcc:"));
        assert!(!raw.contains("hidden@example.net"));
        assert!(
            message
                .envelope()
                .to()
                .iter()
                .any(|address| address.to_string() == "hidden@example.net")
        );
    }

    #[test]
    fn envelope_sender_is_never_the_event_address() {
        let message = build_message(
            &crate::mail::tests::reply_message(),
            "no-reply@mail.example.com",
            "Platform",
            "bounces@example.com",
        )
        .unwrap();
        assert_eq!(
            message
                .envelope()
                .from()
                .map(ToString::to_string)
                .as_deref(),
            Some("bounces@example.com")
        );
        assert_eq!(message.envelope().to().len(), 3);

        let mut transactional = crate::mail::tests::reply_message();
        transactional.from_email = None;
        transactional.from_name = None;
        let message = build_message(
            &transactional,
            "no-reply@mail.example.com",
            "Platform",
            "no-reply@mail.example.com",
        )
        .unwrap();
        let raw = String::from_utf8(message.formatted()).unwrap();
        assert!(raw.contains("From: Platform <no-reply@mail.example.com>\r\n"));
    }
}
