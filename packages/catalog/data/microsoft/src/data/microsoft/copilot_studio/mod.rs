//! Microsoft Copilot Studio agents, called over Direct-to-Engine (the Power Platform API protocol
//! behind the Microsoft 365 Agents SDK `CopilotStudioClient`) or over Direct Line 3.0.

mod engine;
mod turn;

pub mod agent;
pub mod chat;
pub mod direct_line;
pub mod discovery;
pub mod provider;

pub use agent::CopilotStudioAgent;
pub use direct_line::DirectLineSession;
pub use discovery::{CopilotStudioAgentInfo, PowerPlatformEnvironment};
pub use provider::CopilotStudioProvider;
pub use turn::{BotActivity, CopilotStudioAttachment};

use flow_like::flow::node::NodeScores;
use flow_like_types::reqwest;

pub(crate) const CATEGORY: &str = "Data/Microsoft/Copilot Studio";
pub(crate) const FLOWSCRIPT_NAMESPACE: &str = "microsoft.copilotStudio";
pub(crate) const MODEL_NAME: &str = "microsoft-copilot-studio";

/// Hub OAuth provider for tokens with audience `https://api.powerplatform.com`. It is separate from
/// the Graph provider because Entra issues single-audience tokens.
pub const POWER_PLATFORM_PROVIDER_ID: &str = "microsoft_power_platform";
pub const INVOKE_SCOPE: &str = "https://api.powerplatform.com/CopilotStudio.Copilots.Invoke";
pub const ENVIRONMENTS_READ_SCOPE: &str =
    "https://api.powerplatform.com/EnvironmentManagement.Environments.Read";

/// Hub OAuth provider for Dataverse. It signs in against the Global Discovery Service, and its
/// refresh token is redeemed per environment for `https://{org}.crm.dynamics.com`.
pub const DATAVERSE_PROVIDER_ID: &str = "microsoft_dataverse";
pub const GLOBAL_DISCOVERY_SCOPE: &str = "https://globaldisco.crm.dynamics.com/user_impersonation";

pub(crate) fn scores(privacy: u8, performance: u8, cost: u8) -> NodeScores {
    NodeScores::new()
        .set_privacy(privacy)
        .set_security(8)
        .set_performance(performance)
        .set_governance(8)
        .set_reliability(7)
        .set_cost(cost)
        .build()
}

const MAX_ERROR_BODY_CHARS: usize = 600;

/// Turns a failed Microsoft response into an error message that names the operation, the status,
/// the service's own error text and the most likely cause.
pub(crate) async fn http_failure(operation: &str, response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body = body.trim();
    let excerpt: String = body.chars().take(MAX_ERROR_BODY_CHARS).collect();
    let hint = match status.as_u16() {
        401 => {
            " The token was rejected: sign in again, and check that the account is in the same Entra tenant as the agent."
        }
        403 => {
            " Access denied: the agent must be published and shared with this user, and no DLP or channel policy may block it."
        }
        404 => {
            " Not found: check the agent's schema name (case-sensitive), that it is published, and the conversation id."
        }
        429 => " Rate limited: the environment's Copilot Studio quota was hit; retry later.",
        _ => "",
    };
    if excerpt.is_empty() {
        format!("{operation} failed with HTTP {status}.{hint}")
    } else {
        format!("{operation} failed with HTTP {status}: {excerpt}.{hint}")
    }
}
