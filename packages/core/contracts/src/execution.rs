use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ExecutionEnvironment {
    #[default]
    Local,
    Desktop,
    Mobile,
    BrowserSandbox,
    Server,
}

impl ExecutionEnvironment {
    pub const ENV_VAR: &'static str = "FLOW_LIKE_EXECUTION_ENVIRONMENT";

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Desktop => "desktop",
            Self::Mobile => "mobile",
            Self::BrowserSandbox => "browser_sandbox",
            Self::Server => "server",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "desktop" => Some(Self::Desktop),
            "mobile" => Some(Self::Mobile),
            "browser_sandbox" | "browser-sandbox" | "browser" | "sandbox" => {
                Some(Self::BrowserSandbox)
            }
            "server" | "remote" => Some(Self::Server),
            _ => None,
        }
    }

    pub fn is_local(self) -> bool {
        !matches!(self, Self::Server)
    }

    /// Enforces the registered node's execution restriction. The flag must
    /// come from the host's catalog, since a board can alter its own metadata.
    pub fn ensure_node_execution(self, node_name: &str, only_offline: bool) -> anyhow::Result<()> {
        if self == Self::Server && only_offline {
            return Err(anyhow::anyhow!(
                "Node '{node_name}' requires local execution and cannot run in server-side execution"
            ));
        }
        Ok(())
    }

    /// The environment a server-side entry point should run under: the
    /// operator override if one is set, otherwise [`Self::Server`].
    pub fn server_default() -> Self {
        Self::from_env().unwrap_or(Self::Server)
    }

    /// Refuses credential modes that resolve the host process's own cloud
    /// identity (env vars, IMDS / metadata server, workload identity, CLI
    /// token caches, application-default credentials) when the flow runs in
    /// the shared server executor. There, those credentials belong to the
    /// platform, never to the flow author — and the host's role typically
    /// reaches every tenant's storage.
    pub fn ensure_no_ambient_credentials(self, provider: &str, mode: &str) -> anyhow::Result<()> {
        if self == Self::Server {
            return Err(anyhow::anyhow!(
                "{provider}: auth mode '{mode}' resolves the host's ambient credentials, which \
                 is not permitted in server-side execution. Provide explicit credentials \
                 (static keys, service-account key, SAS token, bearer token) instead."
            ));
        }
        Ok(())
    }

    /// Refuses flow-supplied host filesystem paths when the flow runs in the
    /// shared server executor, where the process filesystem (env files,
    /// mounted tokens, credential caches) belongs to the platform.
    pub fn ensure_host_filesystem_access(self, what: &str) -> anyhow::Result<()> {
        if self == Self::Server {
            return Err(anyhow::anyhow!(
                "{what}: access to the host filesystem is not permitted in server-side \
                 execution. Use a store-backed path instead."
            ));
        }
        Ok(())
    }

    pub fn from_env() -> Option<Self> {
        match std::env::var(Self::ENV_VAR) {
            Ok(value) => match Self::parse(&value) {
                Some(environment) => Some(environment),
                None => {
                    tracing::warn!(
                        env_var = Self::ENV_VAR,
                        value = %value,
                        "Ignoring invalid execution environment override"
                    );
                    None
                }
            },
            Err(std::env::VarError::NotPresent) => None,
            Err(err) => {
                tracing::warn!(
                    env_var = Self::ENV_VAR,
                    error = %err,
                    "Unable to read execution environment override"
                );
                None
            }
        }
    }
}
