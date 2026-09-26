use crate::{Ed25519PublicKey, ManagementScope, ProtocolError, Result, validate_management_id};
use serde::{Deserialize, Serialize};

pub const MAX_INVENTORY_PLAINTEXT: usize = 60 * 1024;

/// Routing and revision are authenticated with the retained observation ciphertext.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryBinding {
    pub issuer: String,
    pub api_origin: String,
    pub account_id: String,
    pub device_id: String,
    pub controller_key: Ed25519PublicKey,
    pub scope: ManagementScope,
    pub revision: u64,
}

impl InventoryBinding {
    pub fn validate(&self) -> Result<()> {
        validate_management_id(&self.device_id)?;
        self.controller_key.to_bytes()?;
        inventory_scope_key(&self.scope)?;
        if self.revision == 0
            || self.revision > 9_007_199_254_740_991
            || [&self.issuer, &self.api_origin, &self.account_id]
                .iter()
                .any(|v| v.is_empty() || v.len() > 2048 || v.chars().any(char::is_control))
        {
            return Err(ProtocolError::Invalid("invalid inventory binding"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptedInventory {
    pub binding: InventoryBinding,
    pub ciphertext: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryView {
    /// Current Status scopes. Stored observations outside these scopes are withheld.
    pub scopes: Vec<ManagementScope>,
    pub observations: Vec<EncryptedInventory>,
}

pub fn inventory_scope_key(scope: &ManagementScope) -> Result<String> {
    match scope {
        ManagementScope::Device => Ok("device".into()),
        ManagementScope::Project { project_id } => {
            validate_management_id(project_id)?;
            Ok(format!("project/{project_id}"))
        }
        ManagementScope::Placement {
            project_id,
            placement_id,
        } => {
            validate_management_id(project_id)?;
            validate_management_id(placement_id)?;
            Ok(format!("placement/{project_id}/{placement_id}"))
        }
    }
}

pub fn inventory_scope_contains(allowed: &ManagementScope, requested: &ManagementScope) -> bool {
    match (allowed, requested) {
        (ManagementScope::Device, _) => true,
        (
            ManagementScope::Project { project_id: a },
            ManagementScope::Project { project_id: b }
            | ManagementScope::Placement { project_id: b, .. },
        ) => a == b,
        (
            ManagementScope::Placement {
                project_id: a,
                placement_id: x,
            },
            ManagementScope::Placement {
                project_id: b,
                placement_id: y,
            },
        ) => a == b && x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn narrowed_scope_cannot_retrieve_broader_observations() {
        let project = ManagementScope::Project {
            project_id: "p".into(),
        };
        let placement = ManagementScope::Placement {
            project_id: "p".into(),
            placement_id: "x".into(),
        };
        assert!(inventory_scope_contains(&project, &placement));
        assert!(!inventory_scope_contains(&placement, &project));
        assert!(!inventory_scope_contains(
            &project,
            &ManagementScope::Device
        ));
    }
}
