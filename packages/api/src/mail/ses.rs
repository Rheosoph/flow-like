use aws_sdk_sesv2::types::{Body, Content, Destination, EmailContent, Message, MessageHeader};
use flow_like::hub::MailConfig;
use flow_like_types::Result;

use super::{AutomationEmailMessage, MailClient};

pub struct SesMailClient {
    client: aws_sdk_sesv2::Client,
    from_email: String,
    from_name: String,
}

impl SesMailClient {
    pub async fn new(config: &MailConfig) -> Result<Self> {
        let aws_config = aws_config::load_from_env().await;
        let client = aws_sdk_sesv2::Client::new(&aws_config);

        Ok(Self {
            client,
            from_email: config.from_email.clone(),
            from_name: config.from_name.clone(),
        })
    }
}

fn build_content(message: &AutomationEmailMessage) -> Result<EmailContent> {
    let headers = message
        .headers()?
        .into_iter()
        .map(|(name, value)| MessageHeader::builder().name(name).value(value).build())
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut body_builder = Body::builder();

    if let Some(html) = &message.body_html {
        body_builder = body_builder.html(Content::builder().data(html).charset("UTF-8").build()?);
    }

    if let Some(text) = &message.body_text {
        body_builder = body_builder.text(Content::builder().data(text).charset("UTF-8").build()?);
    }

    Ok(EmailContent::builder()
        .simple(
            Message::builder()
                .subject(
                    Content::builder()
                        .data(&message.subject)
                        .charset("UTF-8")
                        .build()?,
                )
                .body(body_builder.build())
                .set_headers((!headers.is_empty()).then_some(headers))
                .build(),
        )
        .build())
}

#[async_trait::async_trait]
impl MailClient for SesMailClient {
    async fn send_automation(&self, message: AutomationEmailMessage) -> Result<()> {
        let email_content = build_content(&message)?;
        let from_address = super::display_address(
            message.from_name.as_deref().unwrap_or(&self.from_name),
            message.from_email.as_deref().unwrap_or(&self.from_email),
        );
        // Bounces for event senders go to the platform sender, never back into an event.
        let feedback_address = message
            .from_email
            .is_some()
            .then(|| self.from_email.clone());

        let destination = Destination::builder()
            .set_to_addresses(Some(message.to))
            .set_cc_addresses(Some(message.cc))
            .set_bcc_addresses(Some(message.bcc))
            .build();

        self.client
            .send_email()
            .from_email_address(from_address)
            .set_feedback_forwarding_email_address(feedback_address)
            .destination(destination)
            .set_reply_to_addresses(message.reply_to.map(|address| vec![address]))
            .content(email_content)
            .send()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to send email via SES: {}", e))?;

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
    fn simple_message_retains_thread_headers() {
        let message = crate::mail::tests::reply_message();
        let content = build_content(&message).unwrap();
        let simple = content.simple().unwrap();
        assert_eq!(simple.subject().unwrap().data(), "Re: Hello");
        let headers = simple
            .headers()
            .iter()
            .map(|header| (header.name(), header.value()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(headers["In-Reply-To"], "<parent@example.net>");
        assert_eq!(
            headers["References"],
            "<first@example.net> <parent@example.net>"
        );
        assert_eq!(headers["Auto-Submitted"], "auto-replied");
    }
}
