use super::has_permission::PERMISSION_OPTIONS;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, json::json, reqwest};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::OnceLock};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 100;
const MEMBERSHIP_PAGE_SIZE: u64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct UserRef {
    #[serde(alias = "id")]
    pub user_id: String,
    pub sub: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub preferred_username: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub description: Option<String>,
    pub created_at: Option<String>,
    pub additional_information: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ProjectRole {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub attributes: Option<Vec<String>>,
    pub permissions: i64,
    pub app_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ProjectMembership {
    pub id: String,
    pub user_id: String,
    pub app_id: String,
    pub role_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub joined_via: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct ProjectUser {
    pub user: UserRef,
    pub membership_id: String,
    pub app_id: String,
    pub role_id: String,
    pub role: Option<ProjectRole>,
    pub permissions: i64,
    pub permission_names: Vec<String>,
    pub attributes: Vec<String>,
    pub joined_via: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct CurrentUser {
    pub user: UserRef,
    pub project_user: Option<ProjectUser>,
    pub has_project_user: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct UserRoles {
    pub user: UserRef,
    pub role: Option<ProjectRole>,
    pub roles: Vec<ProjectRole>,
    pub role_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct UserPermissions {
    pub user: UserRef,
    pub permissions: i64,
    pub permission_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct UserAttributes {
    pub user: UserRef,
    pub attributes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct UserBatchLookupBody {
    user_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HubError {
    status_code: i64,
    message: String,
}

impl HubError {
    fn new(status_code: i64, message: impl Into<String>) -> Self {
        Self {
            status_code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct HubClient {
    client: reqwest::Client,
    token: String,
    base_url: String,
}

impl HubClient {
    fn from_context(context: &ExecutionContext) -> Result<Self, HubError> {
        let token = context
            .token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| HubError::new(0, "No runtime token available"))?;

        let base_url = api_base_url(&context.profile.hub, context.profile.secure)
            .ok_or_else(|| HubError::new(0, "No hub URL configured on the execution profile"))?;

        static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

        Ok(Self {
            client: CLIENT.get_or_init(reqwest::Client::new).clone(),
            token: token.to_string(),
            base_url,
        })
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<(T, i64), HubError> {
        let url = self.url(path);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|err| HubError::new(0, format!("Failed to call {path}: {err}")))?;

        parse_response(response, path).await
    }

    async fn post_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<(T, i64), HubError> {
        let url = self.url(path);
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|err| HubError::new(0, format!("Failed to call {path}: {err}")))?;

        parse_response(response, path).await
    }

    async fn current_user(&self) -> Result<(UserRef, i64), HubError> {
        let (value, status) = self.get_json::<Value>("user/info").await?;
        let user = UserRef {
            user_id: string_field(&value, "id"),
            sub: string_field(&value, "id"),
            email: optional_string_field(&value, "email"),
            username: optional_string_field(&value, "username"),
            preferred_username: optional_string_field(&value, "preferred_username"),
            name: optional_string_field(&value, "name"),
            avatar_url: optional_string_field(&value, "avatar"),
            description: optional_string_field(&value, "description"),
            created_at: optional_string_field(&value, "created_at"),
            additional_information: value.get("additional_information").cloned(),
        };

        Ok((user, status))
    }

    async fn roles(
        &self,
        app_id: &str,
    ) -> Result<(Option<String>, Vec<ProjectRole>, i64), HubError> {
        let path = format!("apps/{}/roles", urlencoding::encode(app_id));
        let ((default_role_id, roles), status) = self
            .get_json::<(Option<String>, Vec<ProjectRole>)>(&path)
            .await?;
        Ok((default_role_id, roles, status))
    }

    async fn memberships(
        &self,
        app_id: &str,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<ProjectMembership>, i64), HubError> {
        let path = format!(
            "apps/{}/team?offset={offset}&limit={limit}",
            urlencoding::encode(app_id)
        );
        self.get_json::<Vec<ProjectMembership>>(&path).await
    }

    async fn lookup_users(
        &self,
        user_ids: &[String],
    ) -> Result<(HashMap<String, UserRef>, i64), HubError> {
        let mut users = HashMap::new();
        let mut last_status = 200;

        for chunk in user_ids.chunks(100) {
            if chunk.is_empty() {
                continue;
            }

            let body = UserBatchLookupBody {
                user_ids: chunk.to_vec(),
            };
            let (found, status) = self
                .post_json::<_, Vec<UserRef>>("user/lookup", &body)
                .await?;
            last_status = status;

            for mut user in found {
                if user.sub.is_empty() {
                    user.sub = user.user_id.clone();
                }
                if user.user_id.is_empty() {
                    user.user_id = user.sub.clone();
                }
                users.insert(user.user_id.clone(), user);
            }
        }

        Ok((users, last_status))
    }
}

async fn parse_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    path: &str,
) -> Result<(T, i64), HubError> {
    let status = response.status();
    let status_code = status.as_u16() as i64;

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(HubError::new(
            status_code,
            format!("{path} returned {}: {}", status, truncate_error_body(&body)),
        ));
    }

    response
        .json::<T>()
        .await
        .map(|value| (value, status_code))
        .map_err(|err| HubError::new(status_code, format!("Failed to parse {path}: {err}")))
}

fn api_base_url(hub: &str, secure: bool) -> Option<String> {
    let origin = flow_like::hub::hub_origin(hub, secure)?;

    if origin.ends_with("/api/v1") {
        return Some(origin);
    }

    Some(format!("{origin}/api/v1"))
}

fn truncate_error_body(body: &str) -> String {
    const MAX_CHARS: usize = 500;
    let body = body.trim();
    let mut chars = body.chars();
    let truncated: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_none() {
        return body.to_string();
    }

    format!("{truncated}...")
}

fn string_field(value: &Value, key: &str) -> String {
    optional_string_field(value, key).unwrap_or_default()
}

fn optional_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn app_id_from_context(context: &ExecutionContext, app_id: String) -> Result<String, HubError> {
    let app_id = app_id.trim();
    if !app_id.is_empty() {
        return Ok(app_id.to_string());
    }

    context
        .execution_cache
        .as_ref()
        .map(|cache| cache.app_id.clone())
        .filter(|app_id| !app_id.trim().is_empty())
        .ok_or_else(|| {
            HubError::new(
                0,
                "No app_id provided and no execution app context available",
            )
        })
}

fn normalize_offset(offset: i64) -> u64 {
    offset.max(0) as u64
}

fn normalize_limit(limit: i64) -> u64 {
    limit.clamp(1, MAX_LIMIT) as u64
}

async fn eval_string_pin(context: &mut ExecutionContext, pin: &str, default_value: &str) -> String {
    context
        .evaluate_pin::<String>(pin)
        .await
        .unwrap_or_else(|_| default_value.to_string())
}

async fn eval_i64_pin(context: &mut ExecutionContext, pin: &str, default_value: i64) -> i64 {
    context
        .evaluate_pin::<i64>(pin)
        .await
        .unwrap_or(default_value)
}

fn role_map(roles: Vec<ProjectRole>) -> HashMap<String, ProjectRole> {
    roles
        .into_iter()
        .map(|role| (role.id.clone(), role))
        .collect()
}

fn user_from_membership(
    membership: ProjectMembership,
    users: &HashMap<String, UserRef>,
    roles: &HashMap<String, ProjectRole>,
) -> ProjectUser {
    let user = users
        .get(&membership.user_id)
        .cloned()
        .unwrap_or_else(|| UserRef {
            user_id: membership.user_id.clone(),
            sub: membership.user_id.clone(),
            ..Default::default()
        });

    let role = roles.get(&membership.role_id).cloned();
    let permissions = role
        .as_ref()
        .map(|role| role.permissions)
        .unwrap_or_default();
    let attributes = role
        .as_ref()
        .and_then(|role| role.attributes.clone())
        .unwrap_or_default();

    ProjectUser {
        user,
        membership_id: membership.id,
        app_id: membership.app_id,
        role_id: membership.role_id,
        role,
        permissions,
        permission_names: effective_permission_names(permissions),
        attributes,
        joined_via: membership.joined_via,
        created_at: membership.created_at,
        updated_at: membership.updated_at,
    }
}

async fn hydrate_project_users(
    client: &HubClient,
    memberships: Vec<ProjectMembership>,
    roles: &HashMap<String, ProjectRole>,
) -> Result<(Vec<ProjectUser>, i64), HubError> {
    let user_ids = memberships
        .iter()
        .map(|membership| membership.user_id.clone())
        .collect::<Vec<_>>();
    let (users, status) = client.lookup_users(&user_ids).await?;

    Ok((
        memberships
            .into_iter()
            .map(|membership| user_from_membership(membership, &users, roles))
            .collect(),
        status,
    ))
}

async fn get_project_user_by_id(
    client: &HubClient,
    app_id: &str,
    user_id: &str,
    roles: &HashMap<String, ProjectRole>,
) -> Result<Option<(ProjectUser, i64)>, HubError> {
    let mut offset = 0;

    loop {
        let (memberships, status) = client
            .memberships(app_id, offset, MEMBERSHIP_PAGE_SIZE)
            .await?;

        if memberships.is_empty() {
            return Ok(None);
        }

        if let Some(membership) = memberships
            .iter()
            .find(|membership| membership.user_id == user_id)
            .cloned()
        {
            let (mut users, lookup_status) = client
                .lookup_users(std::slice::from_ref(&membership.user_id))
                .await?;
            let user = users
                .remove(&membership.user_id)
                .unwrap_or_else(|| UserRef {
                    user_id: membership.user_id.clone(),
                    sub: membership.user_id.clone(),
                    ..Default::default()
                });
            let mut user_map = HashMap::new();
            user_map.insert(user.user_id.clone(), user);
            return Ok(Some((
                user_from_membership(membership, &user_map, roles),
                lookup_status,
            )));
        }

        if memberships.len() < MEMBERSHIP_PAGE_SIZE as usize {
            return Ok(None);
        }

        offset += MEMBERSHIP_PAGE_SIZE;
        if status == 0 {
            return Ok(None);
        }
    }
}

async fn find_project_user_by_email(
    client: &HubClient,
    app_id: &str,
    email: &str,
    roles: &HashMap<String, ProjectRole>,
) -> Result<Option<(ProjectUser, i64)>, HubError> {
    let email = email.trim().to_ascii_lowercase();
    if email.is_empty() {
        return Ok(None);
    }

    let mut offset = 0;

    loop {
        let (memberships, _) = client
            .memberships(app_id, offset, MEMBERSHIP_PAGE_SIZE)
            .await?;
        if memberships.is_empty() {
            return Ok(None);
        }

        let len = memberships.len();
        let (users, status) = hydrate_project_users(client, memberships, roles).await?;
        if let Some(user) = users.into_iter().find(|user| {
            user.user
                .email
                .as_deref()
                .map(|candidate| candidate.eq_ignore_ascii_case(&email))
                .unwrap_or(false)
        }) {
            return Ok(Some((user, status)));
        }

        if len < MEMBERSHIP_PAGE_SIZE as usize {
            return Ok(None);
        }

        offset += MEMBERSHIP_PAGE_SIZE;
    }
}

async fn search_project_users(
    client: &HubClient,
    app_id: &str,
    roles: &HashMap<String, ProjectRole>,
    query: &str,
    offset: u64,
    limit: u64,
) -> Result<(Vec<ProjectUser>, bool, i64), HubError> {
    if query.trim().is_empty() {
        let (memberships, status) = client.memberships(app_id, offset, limit).await?;
        let has_more = memberships.len() == limit as usize;
        let (users, lookup_status) = hydrate_project_users(client, memberships, roles).await?;
        return Ok((users, has_more, lookup_status.max(status)));
    }

    let mut membership_offset = 0;
    let mut skipped = 0_u64;
    let mut results = Vec::new();
    let mut has_more = false;
    let mut last_status: i64;

    'scan: loop {
        let (memberships, status) = client
            .memberships(app_id, membership_offset, MEMBERSHIP_PAGE_SIZE)
            .await?;
        last_status = status;

        if memberships.is_empty() {
            break;
        }

        let len = memberships.len();
        let (users, lookup_status) = hydrate_project_users(client, memberships, roles).await?;
        last_status = lookup_status;

        for user in users {
            if !matches_project_user(&user, query) {
                continue;
            }

            if skipped < offset {
                skipped += 1;
                continue;
            }

            if results.len() < limit as usize {
                results.push(user);
            } else {
                has_more = true;
                break 'scan;
            }
        }

        if len < MEMBERSHIP_PAGE_SIZE as usize {
            break;
        }

        membership_offset += MEMBERSHIP_PAGE_SIZE;
    }

    Ok((results, has_more, last_status))
}

async fn filter_project_users<F>(
    client: &HubClient,
    app_id: &str,
    roles: &HashMap<String, ProjectRole>,
    offset: u64,
    limit: u64,
    mut predicate: F,
) -> Result<(Vec<ProjectUser>, bool, i64), HubError>
where
    F: FnMut(&ProjectUser) -> bool,
{
    let mut membership_offset = 0;
    let mut skipped = 0_u64;
    let mut results = Vec::new();
    let mut has_more = false;
    let mut last_status: i64;

    'scan: loop {
        let (memberships, status) = client
            .memberships(app_id, membership_offset, MEMBERSHIP_PAGE_SIZE)
            .await?;
        last_status = status;

        if memberships.is_empty() {
            break;
        }

        let len = memberships.len();
        let (users, lookup_status) = hydrate_project_users(client, memberships, roles).await?;
        last_status = lookup_status;

        for user in users {
            if !predicate(&user) {
                continue;
            }

            if skipped < offset {
                skipped += 1;
                continue;
            }

            if results.len() < limit as usize {
                results.push(user);
            } else {
                has_more = true;
                break 'scan;
            }
        }

        if len < MEMBERSHIP_PAGE_SIZE as usize {
            break;
        }

        membership_offset += MEMBERSHIP_PAGE_SIZE;
    }

    Ok((results, has_more, last_status))
}

fn matches_project_user(user: &ProjectUser, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }

    let fields = [
        Some(user.user.user_id.as_str()),
        Some(user.user.sub.as_str()),
        user.user.email.as_deref(),
        user.user.username.as_deref(),
        user.user.preferred_username.as_deref(),
        user.user.name.as_deref(),
        user.role.as_ref().map(|role| role.name.as_str()),
    ];

    fields
        .into_iter()
        .flatten()
        .any(|value| value.to_ascii_lowercase().contains(query.as_str()))
}

fn role_matches(role: Option<&ProjectRole>, role_query: &str) -> bool {
    let role_query = role_query.trim();
    if role_query.is_empty() {
        return true;
    }

    role.map(|role| {
        role.id.eq_ignore_ascii_case(role_query) || role.name.eq_ignore_ascii_case(role_query)
    })
    .unwrap_or(false)
}

fn has_attribute(user: &ProjectUser, attribute: &str) -> bool {
    let attribute = attribute.trim();
    if attribute.is_empty() {
        return false;
    }

    user.attributes
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(attribute))
}

fn permission_from_name(name: &str) -> Option<i64> {
    let normalized = normalize_label(name);
    PERMISSION_OPTIONS
        .iter()
        .find(|(candidate, _)| normalize_label(candidate) == normalized)
        .map(|(_, value)| *value)
        .or_else(|| name.trim().parse::<i64>().ok())
}

fn normalize_label(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn has_effective_permission(permissions: i64, permission: i64) -> bool {
    let is_owner = (permissions & PERMISSION_OPTIONS[0].1) != 0;
    let is_admin = (permissions & PERMISSION_OPTIONS[1].1) != 0;
    is_owner || is_admin || (permissions & permission) != 0
}

fn effective_permission_names(permissions: i64) -> Vec<String> {
    let is_owner = (permissions & PERMISSION_OPTIONS[0].1) != 0;
    let is_admin = (permissions & PERMISSION_OPTIONS[1].1) != 0;

    PERMISSION_OPTIONS
        .iter()
        .filter(|(_, value)| is_owner || is_admin || (permissions & *value) != 0)
        .map(|(name, _)| (*name).to_string())
        .collect()
}

async fn set_common_outputs(
    context: &mut ExecutionContext,
    success: bool,
    status_code: i64,
    error: &str,
) -> flow_like_types::Result<()> {
    context.set_pin_value("success", json!(success)).await?;
    context
        .set_pin_value("status_code", json!(status_code))
        .await?;
    context.set_pin_value("error", json!(error)).await?;
    Ok(())
}

fn base_node(id: &str, name: &str, description: &str) -> Node {
    let mut node = Node::new(id, name, description, "Utils/User");
    node.add_icon("/flow/icons/user.svg");
    node.set_scores(
        NodeScores::new()
            .set_privacy(6)
            .set_security(8)
            .set_performance(6)
            .set_governance(8)
            .set_reliability(7)
            .set_cost(9)
            .build(),
    );
    node
}

fn add_app_pin(node: &mut Node) {
    node.add_input_pin(
        "app_id",
        "App ID",
        "Project/app ID. Leave empty to use the current execution app.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_user_id_pin(node: &mut Node) {
    node.add_input_pin(
        "user_id",
        "User ID",
        "User subject / user ID within the project.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_pagination_pins(node: &mut Node) {
    node.add_input_pin(
        "offset",
        "Offset",
        "Number of matching users to skip.",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(0)));
    node.add_input_pin(
        "limit",
        "Limit",
        "Maximum number of users to return, capped at 100.",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(DEFAULT_LIMIT)));
}

fn add_common_outputs(node: &mut Node) {
    node.add_output_pin(
        "success",
        "Success",
        "True when the read operation completed successfully.",
        VariableType::Boolean,
    );
    node.add_output_pin(
        "status_code",
        "Status Code",
        "HTTP status code returned by the hub, or 0 if no request was made.",
        VariableType::Integer,
    );
    node.add_output_pin(
        "error",
        "Error",
        "Error message when the read operation could not complete.",
        VariableType::String,
    );
}

fn add_project_user_output(node: &mut Node) {
    node.add_output_pin(
        "project_user",
        "Project User",
        "Project membership, sanitized user ref, role, effective permissions, and attributes.",
        VariableType::Struct,
    )
    .set_schema::<ProjectUser>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "found",
        "Found",
        "True when a matching project user was found.",
        VariableType::Boolean,
    );
}

fn add_users_output(node: &mut Node) {
    node.add_output_pin(
        "users",
        "Users",
        "Matching project users.",
        VariableType::Struct,
    )
    .set_value_type(ValueType::Array)
    .set_schema::<ProjectUser>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "count",
        "Count",
        "Number of users returned.",
        VariableType::Integer,
    );
    node.add_output_pin(
        "next_offset",
        "Next Offset",
        "Offset to use for the next page.",
        VariableType::Integer,
    );
    node.add_output_pin(
        "has_more",
        "Has More",
        "True when another page may contain more matching users.",
        VariableType::Boolean,
    );
}

fn permission_names() -> Vec<String> {
    PERMISSION_OPTIONS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect()
}

async fn load_roles(
    client: &HubClient,
    app_id: &str,
) -> Result<(HashMap<String, ProjectRole>, i64), HubError> {
    let (_, roles, status) = client.roles(app_id).await?;
    Ok((role_map(roles), status))
}

pub mod check_user_has_role;
pub mod check_user_permission;
pub mod get_current_user;
pub mod get_effective_user_permissions;
pub mod get_project_user;
pub mod get_user_attribute;
pub mod get_user_attributes;
pub mod get_user_roles;
pub mod list_project_users;
pub mod list_users_with_attribute;
pub mod list_users_with_role;
pub mod resolve_user;
pub mod search_users;

pub use check_user_has_role::*;
pub use check_user_permission::*;
pub use get_current_user::*;
pub use get_effective_user_permissions::*;
pub use get_project_user::*;
pub use get_user_attribute::*;
pub use get_user_attributes::*;
pub use get_user_roles::*;
pub use list_project_users::*;
pub use list_users_with_attribute::*;
pub use list_users_with_role::*;
pub use resolve_user::*;
pub use search_users::*;

async fn load_single_project_user(
    context: &mut ExecutionContext,
) -> Result<Option<(ProjectUser, i64)>, HubError> {
    let app_id_input = eval_string_pin(context, "app_id", "").await;
    let user_id = eval_string_pin(context, "user_id", "").await;
    let client = HubClient::from_context(context)?;
    let app_id = app_id_from_context(context, app_id_input)?;
    let (roles, role_status) = load_roles(&client, &app_id).await?;

    get_project_user_by_id(&client, &app_id, user_id.trim(), &roles)
        .await
        .map(|found| found.map(|(user, status)| (user, status.max(role_status))))
}

async fn run_single_project_user_node(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<()> {
    match load_single_project_user(context).await {
        Ok(Some((user, status))) => {
            context.set_pin_value("project_user", json!(user)).await?;
            context.set_pin_value("found", json!(true)).await?;
            set_common_outputs(context, true, status, "").await?;
        }
        Ok(None) => {
            context.set_pin_value("project_user", Value::Null).await?;
            context.set_pin_value("found", json!(false)).await?;
            set_common_outputs(context, true, 200, "").await?;
        }
        Err(err) => {
            context.log_message(&err.message, LogLevel::Warn);
            context.set_pin_value("project_user", Value::Null).await?;
            context.set_pin_value("found", json!(false)).await?;
            set_common_outputs(context, false, err.status_code, &err.message).await?;
        }
    }
    Ok(())
}

async fn run_user_list_node<'a, F>(
    context: &'a mut ExecutionContext,
    handler: F,
) -> flow_like_types::Result<()>
where
    F: for<'b> FnOnce(
        &'b HubClient,
        &'b str,
        &'b HashMap<String, ProjectRole>,
        u64,
        u64,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(Vec<ProjectUser>, bool, i64), HubError>>
                + Send
                + 'b,
        >,
    >,
{
    let app_id_input = eval_string_pin(context, "app_id", "").await;
    let offset = normalize_offset(eval_i64_pin(context, "offset", 0).await);
    let limit = normalize_limit(eval_i64_pin(context, "limit", DEFAULT_LIMIT).await);

    let client = match HubClient::from_context(context) {
        Ok(client) => client,
        Err(err) => {
            context.set_pin_value("users", json!([])).await?;
            context.set_pin_value("count", json!(0)).await?;
            context
                .set_pin_value("next_offset", json!(offset as i64))
                .await?;
            context.set_pin_value("has_more", json!(false)).await?;
            set_common_outputs(context, false, err.status_code, &err.message).await?;
            return Ok(());
        }
    };

    let app_id = match app_id_from_context(context, app_id_input) {
        Ok(app_id) => app_id,
        Err(err) => {
            context.set_pin_value("users", json!([])).await?;
            context.set_pin_value("count", json!(0)).await?;
            context
                .set_pin_value("next_offset", json!(offset as i64))
                .await?;
            context.set_pin_value("has_more", json!(false)).await?;
            set_common_outputs(context, false, err.status_code, &err.message).await?;
            return Ok(());
        }
    };

    let result = async {
        let (roles, role_status) = load_roles(&client, &app_id).await?;
        let (users, has_more, status) = handler(&client, &app_id, &roles, offset, limit).await?;
        Ok::<_, HubError>((users, has_more, status.max(role_status)))
    }
    .await;

    match result {
        Ok((users, has_more, status)) => {
            let count = users.len() as i64;
            let next_offset = offset as i64 + count;
            context.set_pin_value("users", json!(users)).await?;
            context.set_pin_value("count", json!(count)).await?;
            context
                .set_pin_value("next_offset", json!(next_offset))
                .await?;
            context.set_pin_value("has_more", json!(has_more)).await?;
            set_common_outputs(context, true, status, "").await?;
        }
        Err(err) => {
            context.log_message(&err.message, LogLevel::Warn);
            context.set_pin_value("users", json!([])).await?;
            context.set_pin_value("count", json!(0)).await?;
            context
                .set_pin_value("next_offset", json!(offset as i64))
                .await?;
            context.set_pin_value("has_more", json!(false)).await?;
            set_common_outputs(context, false, err.status_code, &err.message).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        api_base_url, effective_permission_names, has_effective_permission, permission_from_name,
    };

    #[test]
    fn builds_api_base_url() {
        assert_eq!(
            api_base_url("https://hub.flow-like.com/", true).as_deref(),
            Some("https://hub.flow-like.com/api/v1")
        );
        assert_eq!(
            api_base_url("https://hub.flow-like.com/api/v1", true).as_deref(),
            Some("https://hub.flow-like.com/api/v1")
        );
        assert_eq!(
            api_base_url("localhost:8080", false).as_deref(),
            Some("http://localhost:8080/api/v1")
        );
        assert_eq!(api_base_url(" ", true), None);
    }

    #[test]
    fn resolves_permissions_by_name_and_alias_shape() {
        assert_eq!(permission_from_name("Read Boards"), Some(256));
        assert_eq!(permission_from_name("read_boards"), Some(256));
        assert_eq!(permission_from_name("read-boards"), Some(256));
        assert_eq!(permission_from_name("256"), Some(256));
        assert_eq!(permission_from_name("missing"), None);
    }

    #[test]
    fn expands_owner_and_admin_permissions() {
        assert!(has_effective_permission(1, 256));
        assert!(has_effective_permission(2, 256));
        assert!(effective_permission_names(1).contains(&"Read Boards".to_string()));
        assert!(effective_permission_names(256).contains(&"Read Boards".to_string()));
    }
}
