use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Subject reported by runs that carry no authenticated caller. It never
/// identifies a stored account, so consumers must resolve it to the current
/// viewer instead of looking it up.
pub const LOCAL_USER_SUB: &str = "local";

/// How the caller of a run authenticated. `is_technical_user` keeps the coarse
/// human/machine split; this names which machine principal it was, so an app
/// calling through an app connection is no longer indistinguishable from an
/// API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionPrincipal {
    /// A human account: an OIDC session or a personal access token.
    #[default]
    User,
    /// An app-scoped API key. Carries no human subject of its own.
    ApiKey,
    /// Another app calling through an app connection.
    ConnectedApp,
}

/// Represents the user context during execution.
/// Contains information about the user who triggered the execution,
/// their role, permissions, and any custom attributes.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserExecutionContext {
    /// User subject identifier (e.g., OIDC sub claim)
    /// Empty for technical users (API keys, app connections)
    pub sub: String,
    /// Role information
    pub role: Option<RoleContext>,
    /// Whether this is a technical user (API key, app connection) rather than a human user
    #[serde(default)]
    pub is_technical_user: bool,
    /// For API keys, the key identifier. Never set for other principals.
    #[serde(default)]
    pub key_id: Option<String>,
    /// Which kind of principal started the run.
    #[serde(default)]
    pub principal: ExecutionPrincipal,
    /// For `ConnectedApp`, the app that made the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_app_id: Option<String>,
    /// Subject the calling principal reported as the initiator: the API key's
    /// creator, or the user an app connection passed through. Attribution only
    /// — the run must never treat it as an identity it may act as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<String>,
}

/// Role context containing role metadata and permissions
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct RoleContext {
    /// Role ID
    pub id: String,
    /// Role name
    pub name: String,
    /// Role permissions as a bitfield
    pub permissions: i64,
    /// Custom attributes assigned to the role
    pub attributes: Vec<String>,
    /// Custom key-value attributes that can be set by users
    #[serde(default)]
    pub custom_attributes: HashMap<String, String>,
}

impl UserExecutionContext {
    /// Create a new user execution context
    pub fn new(sub: impl Into<String>) -> Self {
        Self {
            sub: sub.into(),
            role: None,
            is_technical_user: false,
            key_id: None,
            principal: ExecutionPrincipal::User,
            origin_app_id: None,
            on_behalf_of: None,
        }
    }

    /// Create an offline/local user context with admin privileges
    pub fn offline() -> Self {
        Self {
            sub: LOCAL_USER_SUB.to_string(),
            role: Some(RoleContext::admin()),
            is_technical_user: false,
            key_id: None,
            principal: ExecutionPrincipal::User,
            origin_app_id: None,
            on_behalf_of: None,
        }
    }

    /// Create a context for a run of an app that has no server-side role to
    /// consult — an offline app, or a signed-out desktop session. Those runs
    /// are owner-equivalent by definition: the machine owns the app. An
    /// authenticated run keeps the caller's subject so nodes and surfaces
    /// resolve the real user instead of the placeholder.
    ///
    /// A hosted app executed locally must NOT use this: its role lives on the
    /// hub and has to be resolved there, or the same board answers
    /// `Has Permission` differently on the desktop than in the cloud.
    pub fn local(sub: impl Into<String>) -> Self {
        let sub = sub.into();
        if sub.is_empty() || sub == LOCAL_USER_SUB {
            return Self::offline();
        }

        Self::new(sub).with_role(RoleContext::admin())
    }

    /// Create a context for technical users (API keys)
    pub fn technical(
        key_id: impl Into<String>,
        role_id: impl Into<String>,
        role_name: impl Into<String>,
        permissions: i64,
        attributes: Vec<String>,
        custom_attributes: HashMap<String, String>,
    ) -> Self {
        Self {
            sub: String::new(),
            role: Some(RoleContext {
                id: role_id.into(),
                name: role_name.into(),
                permissions,
                attributes,
                custom_attributes,
            }),
            is_technical_user: true,
            key_id: Some(key_id.into()),
            principal: ExecutionPrincipal::ApiKey,
            origin_app_id: None,
            on_behalf_of: None,
        }
    }

    /// Create a context for an app calling through an app connection. The
    /// connection's role bounds the run; the passed-through subject is
    /// attribution only, so it lands in `on_behalf_of` rather than `sub`.
    pub fn connected_app(origin_app_id: impl Into<String>, role: RoleContext) -> Self {
        Self {
            sub: String::new(),
            role: Some(role),
            is_technical_user: true,
            key_id: None,
            principal: ExecutionPrincipal::ConnectedApp,
            origin_app_id: Some(origin_app_id.into()),
            on_behalf_of: None,
        }
    }

    /// Record the subject the calling principal reported as the initiator.
    pub fn with_on_behalf_of(mut self, sub: Option<String>) -> Self {
        self.on_behalf_of = sub.filter(|sub| !sub.is_empty());
        self
    }

    /// Set the role context
    pub fn with_role(mut self, role: RoleContext) -> Self {
        self.role = Some(role);
        self
    }

    /// Check if the user has a specific permission
    pub fn has_permission(&self, permission: i64) -> bool {
        self.role
            .as_ref()
            .map(|r| r.has_permission(permission))
            .unwrap_or(false)
    }

    /// Check if this is an offline/local context
    pub fn is_offline(&self) -> bool {
        self.sub == LOCAL_USER_SUB
    }

    /// Check if this is a technical user (API key or app connection)
    pub fn is_technical(&self) -> bool {
        self.is_technical_user
    }

    /// Check if another app is calling through an app connection
    pub fn is_connected_app(&self) -> bool {
        self.principal == ExecutionPrincipal::ConnectedApp
    }

    /// Get the key ID for API keys
    pub fn get_key_id(&self) -> Option<&str> {
        self.key_id.as_deref()
    }

    /// The app that made the call, for app-connection principals
    pub fn origin_app_id(&self) -> Option<&str> {
        self.origin_app_id.as_deref()
    }

    /// The subject the calling principal reported as the initiator. Attribution
    /// only — never authorize against this.
    pub fn on_behalf_of(&self) -> Option<&str> {
        self.on_behalf_of.as_deref()
    }

    /// Get an attribute value by key
    pub fn get_attribute(&self, key: &str) -> Option<&str> {
        self.role
            .as_ref()
            .and_then(|r| r.custom_attributes.get(key).map(|s| s.as_str()))
    }

    /// Check if a simple attribute (tag) exists
    pub fn has_attribute(&self, attribute: &str) -> bool {
        self.role
            .as_ref()
            .map(|r| r.attributes.contains(&attribute.to_string()))
            .unwrap_or(false)
    }
}

impl RoleContext {
    /// Create an admin role context (for offline/local execution)
    pub fn admin() -> Self {
        Self {
            id: "local-admin".to_string(),
            name: "Admin".to_string(),
            permissions: Self::OWNER_PERMISSION,
            attributes: vec!["admin".to_string()],
            custom_attributes: HashMap::new(),
        }
    }

    /// Owner permission bitflag (all permissions)
    pub const OWNER_PERMISSION: i64 = 0b00000000_00000000_00000000_00000001;
    /// Admin permission bitflag
    pub const ADMIN_PERMISSION: i64 = 0b00000000_00000000_00000000_00000010;

    /// Check if the role has a specific permission
    pub fn has_permission(&self, permission: i64) -> bool {
        // Owner and Admin have all permissions
        if self.permissions & Self::OWNER_PERMISSION != 0
            || self.permissions & Self::ADMIN_PERMISSION != 0
        {
            return true;
        }
        self.permissions & permission != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offline_context() {
        let ctx = UserExecutionContext::offline();
        assert_eq!(ctx.sub, "local");
        assert!(ctx.is_offline());
        assert!(ctx.role.is_some());
        assert!(ctx.has_permission(RoleContext::OWNER_PERMISSION));
    }

    #[test]
    fn test_local_context_keeps_authenticated_sub() {
        let ctx = UserExecutionContext::local("user-123");
        assert_eq!(ctx.sub, "user-123");
        assert!(!ctx.is_offline());
        assert!(ctx.has_permission(RoleContext::OWNER_PERMISSION));
    }

    #[test]
    fn test_local_context_falls_back_to_offline() {
        for sub in ["", LOCAL_USER_SUB] {
            let ctx = UserExecutionContext::local(sub);
            assert_eq!(ctx.sub, LOCAL_USER_SUB);
            assert!(ctx.is_offline());
            assert!(ctx.has_permission(RoleContext::OWNER_PERMISSION));
        }
    }

    #[test]
    fn test_context_with_role() {
        let role = RoleContext {
            id: "role-123".to_string(),
            name: "Editor".to_string(),
            permissions: 0b00001000,
            attributes: vec!["editor".to_string()],
            custom_attributes: HashMap::from([(
                "department".to_string(),
                "engineering".to_string(),
            )]),
        };

        let ctx = UserExecutionContext::new("user-123").with_role(role);

        assert_eq!(ctx.sub, "user-123");
        assert!(!ctx.is_offline());
        assert!(ctx.has_attribute("editor"));
        assert_eq!(ctx.get_attribute("department"), Some("engineering"));
    }

    #[test]
    fn test_connected_app_is_not_an_api_key() {
        let ctx = UserExecutionContext::connected_app("origin-app", RoleContext::admin())
            .with_on_behalf_of(Some("user-123".to_string()));

        assert_eq!(ctx.principal, ExecutionPrincipal::ConnectedApp);
        assert!(ctx.is_connected_app());
        assert!(ctx.is_technical());
        assert_eq!(ctx.get_key_id(), None);
        assert_eq!(ctx.origin_app_id(), Some("origin-app"));
        // The passed-through subject is attribution, never an identity to act as.
        assert_eq!(ctx.sub, "");
        assert_eq!(ctx.on_behalf_of(), Some("user-123"));
    }

    #[test]
    fn test_api_key_principal() {
        let ctx = UserExecutionContext::technical(
            "key-1",
            "role-1",
            "Runner",
            0b1000,
            vec![],
            HashMap::new(),
        );

        assert_eq!(ctx.principal, ExecutionPrincipal::ApiKey);
        assert!(!ctx.is_connected_app());
        assert_eq!(ctx.get_key_id(), Some("key-1"));
        assert_eq!(ctx.origin_app_id(), None);
    }

    #[test]
    fn test_human_principals_default_to_user() {
        for ctx in [
            UserExecutionContext::new("user-123"),
            UserExecutionContext::offline(),
            UserExecutionContext::local("user-123"),
        ] {
            assert_eq!(ctx.principal, ExecutionPrincipal::User);
            assert!(!ctx.is_technical());
            assert!(ctx.on_behalf_of().is_none());
        }
    }

    #[test]
    fn test_permission_check() {
        let role = RoleContext {
            id: "role-123".to_string(),
            name: "Viewer".to_string(),
            permissions: 0b00001000,
            attributes: vec![],
            custom_attributes: HashMap::new(),
        };

        let ctx = UserExecutionContext::new("user-123").with_role(role);

        assert!(ctx.has_permission(0b00001000));
        assert!(!ctx.has_permission(0b00010000));
    }
}
