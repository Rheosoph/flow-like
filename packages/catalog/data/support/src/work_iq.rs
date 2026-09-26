//! Identity of the Microsoft Work IQ service, shared by the Microsoft data nodes and agent tooling
//! so both resolve the same OAuth token.

pub const WORK_IQ_PROVIDER_ID: &str = "microsoft_workiq";
pub const WORK_IQ_SCOPE: &str = "api://workiq.svc.cloud.microsoft/WorkIQAgent.Ask";
pub const WORK_IQ_BASE_URL: &str = "https://workiq.svc.cloud.microsoft";
pub const WORK_IQ_MCP_URL: &str = "https://workiq.svc.cloud.microsoft/mcp";
