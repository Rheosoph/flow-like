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

#[async_trait::async_trait]
pub trait MailClient: Send + Sync {
    async fn send(&self, message: EmailMessage) -> Result<()>;
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

fn runtime_provider_override() -> Result<Option<MailProviderType>> {
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
