use flow_like::hub::{MailConfig, SendgridSettings};
use flow_like_types::Result;
use reqwest::Client;
use serde::Serialize;

use super::{AutomationEmailMessage, MailClient};

pub struct SendgridMailClient {
    client: Client,
    api_key: String,
    from_email: String,
    from_name: String,
}

#[derive(Serialize)]
struct SendgridEmail {
    personalizations: Vec<Personalization>,
    from: EmailAddress,
    subject: String,
    content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<EmailAddress>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    headers: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize)]
struct Personalization {
    to: Vec<EmailAddress>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<EmailAddress>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<EmailAddress>,
}

#[derive(Serialize)]
struct EmailAddress {
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Serialize)]
struct Content {
    #[serde(rename = "type")]
    content_type: String,
    value: String,
}

impl SendgridMailClient {
    pub fn new(config: &MailConfig, sendgrid: &SendgridSettings) -> Result<Self> {
        let api_key = std::env::var(&sendgrid.api_key_env).map_err(|_| {
            flow_like_types::anyhow!("Sendgrid API key env var {} not set", sendgrid.api_key_env)
        })?;

        Ok(Self {
            client: Client::new(),
            api_key,
            from_email: config.from_email.clone(),
            from_name: config.from_name.clone(),
        })
    }
}

impl SendgridEmail {
    fn new(message: AutomationEmailMessage, from_email: &str, from_name: &str) -> Result<Self> {
        let headers = message.headers()?;
        let mut content = Vec::new();

        if let Some(text) = &message.body_text {
            content.push(Content {
                content_type: "text/plain".to_string(),
                value: text.clone(),
            });
        }

        if let Some(html) = &message.body_html {
            content.push(Content {
                content_type: "text/html".to_string(),
                value: html.clone(),
            });
        }

        if content.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Email must have either HTML or text body"
            ));
        }

        Ok(Self {
            personalizations: vec![Personalization {
                to: message.to.into_iter().map(EmailAddress::from).collect(),
                cc: message.cc.into_iter().map(EmailAddress::from).collect(),
                bcc: message.bcc.into_iter().map(EmailAddress::from).collect(),
            }],
            from: EmailAddress {
                email: message.from_email.unwrap_or_else(|| from_email.to_owned()),
                name: Some(message.from_name.unwrap_or_else(|| from_name.to_owned())),
            },
            subject: message.subject.clone(),
            content,
            reply_to: message.reply_to.map(EmailAddress::from),
            headers,
        })
    }
}

#[async_trait::async_trait]
impl MailClient for SendgridMailClient {
    async fn send_automation(&self, message: AutomationEmailMessage) -> Result<()> {
        let email = SendgridEmail::new(message, &self.from_email, &self.from_name)?;

        let response = self
            .client
            .post("https://api.sendgrid.com/v3/mail/send")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&email)
            .send()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to send email via Sendgrid: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Sendgrid API error: {} - {}",
                status,
                body
            ));
        }

        Ok(())
    }

    fn from_email(&self) -> &str {
        &self.from_email
    }

    fn from_name(&self) -> &str {
        &self.from_name
    }
}

impl From<String> for EmailAddress {
    fn from(email: String) -> Self {
        Self { email, name: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_the_event_sender_thread_and_hidden_recipients() {
        let email = SendgridEmail::new(
            crate::mail::tests::reply_message(),
            "platform@example.com",
            "Platform",
        )
        .unwrap();
        let value = serde_json::to_value(email).unwrap();
        assert_eq!(value["from"]["email"], "event@example.com");
        assert_eq!(value["from"]["name"], "Order Desk");
        assert_eq!(value["headers"]["Auto-Submitted"], "auto-replied");
        assert_eq!(
            value["personalizations"][0]["bcc"][0]["email"],
            "hidden@example.net"
        );
        assert_eq!(value["headers"]["In-Reply-To"], "<parent@example.net>");
        assert_eq!(
            value["headers"]["References"],
            "<first@example.net> <parent@example.net>"
        );
        assert!(value.get("reply_to").is_none());
    }
}
