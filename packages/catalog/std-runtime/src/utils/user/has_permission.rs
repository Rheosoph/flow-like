use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Permission constants matching the API RolePermissions bitflags.
/// These are used to populate the dropdown options.
pub(crate) const PERMISSION_OPTIONS: &[(&str, i64)] = &[
    ("Owner", 0b00000000_00000000_00000000_00000001),
    ("Admin", 0b00000000_00000000_00000000_00000010),
    ("Read Team", 0b00000000_00000000_00000000_00000100),
    ("Read Roles", 0b00000000_00000000_00000000_00001000),
    ("Read Files", 0b00000000_00000000_00000000_00010000),
    ("Write Files", 0b00000000_00000000_00000000_00100000),
    ("Invoke API", 0b00000000_00000000_00000000_01000000),
    ("Write Meta", 0b00000000_00000000_00000000_10000000),
    ("Read Boards", 0b00000000_00000000_00000001_00000000),
    ("Execute Boards", 0b00000000_00000000_00000010_00000000),
    ("Write Boards", 0b00000000_00000000_00000100_00000000),
    ("List Events", 0b00000000_00000000_00001000_00000000),
    ("Read Events", 0b00000000_00000000_00010000_00000000),
    ("Execute Events", 0b00000000_00000000_00100000_00000000),
    ("Write Events", 0b00000000_00000000_01000000_00000000),
    ("Read Logs", 0b00000000_00000000_10000000_00000000),
    ("Read Analytics", 0b00000000_00000001_00000000_00000000),
    ("Read Config", 0b00000000_00000010_00000000_00000000),
    ("Write Config", 0b00000000_00000100_00000000_00000000),
    ("Read Templates", 0b00000000_00001000_00000000_00000000),
    ("Write Templates", 0b00000000_00010000_00000000_00000000),
    ("Read Courses", 0b00000000_00100000_00000000_00000000),
    ("Write Courses", 0b00000000_01000000_00000000_00000000),
    ("Read Widgets", 0b00000000_10000000_00000000_00000000),
    ("Write Widgets", 0b00000001_00000000_00000000_00000000),
    ("Write Routes", 0b00000010_00000000_00000000_00000000),
];

/// Check if the executing user has a specific permission.
/// Admin and Owner permissions implicitly include all other permissions.
#[crate::register_node]
#[derive(Default)]
pub struct HasPermissionNode {}

impl HasPermissionNode {
    pub fn new() -> Self {
        HasPermissionNode {}
    }
}

#[async_trait]
impl NodeLogic for HasPermissionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_user_has_permission",
            "Has Permission",
            "Checks if the executing user has a specific permission. Admin and Owner roles automatically have all permissions. Returns false if no user context is available.",
            "Utils/User",
        );
        node.set_flowscript_name("user", "hasPermission");
        node.add_icon("/flow/icons/shield.svg");

        let permission_names: Vec<String> = PERMISSION_OPTIONS
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();

        node.add_input_pin(
            "permission",
            "Permission",
            "The permission to check for",
            VariableType::String,
        )
        .set_default_value(Some(json!("Read Boards")))
        .set_options(PinOptions::new().set_valid_values(permission_names).build());

        node.add_output_pin(
            "has_permission",
            "Has Permission",
            "True if the user has the specified permission (or is Admin/Owner)",
            VariableType::Boolean,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(8) // Only checks permissions, doesn't expose user data
                .set_security(9) // Important for access control
                .set_performance(10) // Very fast, just bitwise check
                .set_governance(9) // Essential for authorization
                .set_reliability(10) // Always succeeds
                .set_cost(10) // No external calls
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let permission_name: String = context.evaluate_pin("permission").await?;

        // Look up the permission value from the name
        let permission_value = PERMISSION_OPTIONS
            .iter()
            .find(|(name, _)| *name == permission_name)
            .map(|(_, value)| *value);

        let user_context = context.user_context().cloned();

        let has_permission = match permission_value {
            Some(perm) => {
                match user_context {
                    Some(uc) => {
                        // Check if user has the permission
                        // Admin (bit 1) and Owner (bit 0) have all permissions
                        let user_perms = uc.role.as_ref().map(|r| r.permissions).unwrap_or(0);

                        let is_admin = (user_perms & 0b10) != 0; // Admin bit
                        let is_owner = (user_perms & 0b01) != 0; // Owner bit

                        is_owner || is_admin || (user_perms & perm) != 0
                    }
                    None => false,
                }
            }
            None => {
                context.log_message(
                    &format!("Unknown permission: {}", permission_name),
                    LogLevel::Warn,
                );
                false
            }
        };

        context
            .set_pin_value("has_permission", json!(has_permission))
            .await?;

        Ok(())
    }
}
