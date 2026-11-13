use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderValue, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, FixedOffset, Utc};
use flow_like::bit::Metadata;
use flow_like_types::tokio::time::timeout;
use reqwest::Url;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::{
    entity::{app, invite_link, membership, meta, sea_orm_active_enums::Visibility, user},
    error::ApiError,
    routes::{app::meta::MetaMode, inbox::links},
    state::AppState,
};

mod page;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/join", get(invitation))
        .route("/join/", get(invitation))
}

#[derive(Debug, Deserialize)]
struct InviteQuery {
    #[serde(rename = "appId")]
    app_id: String,
    token: String,
}

/// A group chat's invite link: `room` is the link's token.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoomQuery {
    room: String,
}

impl InviteQuery {
    fn pairs(&self) -> [(&'static str, &str); 2] {
        [("appId", &self.app_id), ("token", &self.token)]
    }

    fn is_valid(&self) -> bool {
        [&self.app_id, &self.token].into_iter().all(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    }
}

#[derive(FromQueryResult)]
struct InviteAccess {
    expires_at: Option<DateTime<FixedOffset>>,
    count_joined: i64,
    max_uses: i64,
}

#[derive(FromQueryResult)]
struct AppAccess {
    visibility: Visibility,
    price: i64,
    default_role_id: Option<String>,
}

async fn can_preview<C: ConnectionTrait>(
    db: &C,
    query: &InviteQuery,
    max_prototype: i64,
    now: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    if !query.is_valid() {
        return Ok(false);
    }
    let link = invite_link::Entity::find()
        .select_only()
        .column_as(invite_link::Column::ExpiresAt, "expires_at")
        .column_as(invite_link::Column::CountJoined, "count_joined")
        .column_as(invite_link::Column::MaxUses, "max_uses")
        .filter(invite_link::Column::AppId.eq(&query.app_id))
        .filter(invite_link::Column::Token.eq(&query.token))
        .into_model::<InviteAccess>()
        .one(db)
        .await?;
    let Some(link) = link else {
        return Ok(false);
    };
    if link.expires_at.is_some_and(|expires_at| expires_at <= now)
        || (link.max_uses > 0 && link.count_joined >= link.max_uses)
    {
        return Ok(false);
    }

    let app = app::Entity::find_by_id(&query.app_id)
        .select_only()
        .columns([app::Column::Visibility, app::Column::Price])
        .column_as(app::Column::DefaultRoleId, "default_role_id")
        .into_model::<AppAccess>()
        .one(db)
        .await?;
    let Some(app) = app else {
        return Ok(false);
    };
    if matches!(app.visibility, Visibility::Private | Visibility::Offline)
        || app.price > 0
        || app.default_role_id.as_deref().is_none_or(str::is_empty)
    {
        return Ok(false);
    }
    if app.visibility == Visibility::Prototype && max_prototype > 0 {
        let members = membership::Entity::find()
            .select_only()
            .column(membership::Column::Id)
            .filter(membership::Column::AppId.eq(&query.app_id))
            .count(db)
            .await?;
        if members >= max_prototype as u64 {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn invitation_metadata<C: ConnectionTrait>(
    db: &C,
    query: &InviteQuery,
    max_prototype: i64,
) -> Result<Option<Metadata>, DbErr> {
    if !can_preview(db, query, max_prototype, Utc::now().fixed_offset()).await? {
        return Ok(None);
    }

    // Read branding only after the token has granted access to this project.
    let metadata = meta::Entity::find()
        .filter(meta::Column::AppId.eq(&query.app_id))
        .filter(meta::Column::BitId.is_null())
        .filter(meta::Column::TemplateId.is_null())
        .filter(meta::Column::CourseId.is_null())
        .filter(meta::Column::WidgetId.is_null())
        .filter(meta::Column::WasmPackageId.is_null())
        .filter(meta::Column::GroupId.is_null());
    let english = metadata
        .clone()
        .filter(meta::Column::Lang.eq("en"))
        .one(db)
        .await?;
    let record = match english {
        Some(record) => Some(record),
        None => metadata.order_by_asc(meta::Column::Lang).one(db).await?,
    };
    Ok(Some(record.map(Metadata::from).unwrap_or_else(|| {
        Metadata {
            name: "Flow-Like project".into(),
            description: String::new(),
            ..Metadata::default()
        }
    })))
}

fn frontend_url(
    frontend_base: Option<&str>,
    frontend: Option<&str>,
    hub_app: Option<&str>,
    secure: bool,
) -> Result<Url, ApiError> {
    let configured_env = frontend_base
        .filter(|value| !value.trim().is_empty())
        .or_else(|| frontend.filter(|value| !value.trim().is_empty()));
    let configured = configured_env
        .or(hub_app)
        .ok_or_else(|| ApiError::service_unavailable("The web app URL is not configured"))?;
    let configured = if configured_env.is_none() && !configured.contains("://") {
        if configured.starts_with('/') || configured.contains(['?', '#', '@', '\\']) {
            return Err(ApiError::service_unavailable("Invalid web app URL"));
        }
        format!("{}://{configured}", if secure { "https" } else { "http" })
    } else {
        configured.to_owned()
    };
    if !configured.starts_with("https://") && !configured.starts_with("http://") {
        return Err(ApiError::service_unavailable("Invalid web app URL"));
    }
    let url = Url::parse(&configured)
        .map_err(|_| ApiError::service_unavailable("Invalid web app URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || configured.chars().any(char::is_whitespace)
        || configured.contains('\\')
    {
        return Err(ApiError::service_unavailable("Invalid web app URL"));
    }
    Ok(url)
}

fn safe_image(value: Option<String>) -> Option<String> {
    let value = value?;
    let url = Url::parse(&value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && !value.chars().any(char::is_control))
    .then(|| url.to_string())
}

fn api_url(public_api: Option<&str>, domain: &str, secure: bool) -> Result<Url, ApiError> {
    let configured = match public_api {
        Some(value) => value.to_owned(),
        None => {
            if domain.contains(['/', '?', '#', '@', '\\']) {
                return Err(ApiError::service_unavailable("Invalid public API URL"));
            }
            format!("{}://{domain}", if secure { "https" } else { "http" })
        }
    };
    if (!configured.starts_with("https://") && !configured.starts_with("http://"))
        || configured.chars().any(char::is_whitespace)
        || configured.contains('\\')
    {
        return Err(ApiError::service_unavailable("Invalid public API URL"));
    }
    let mut url = Url::parse(&configured)
        .map_err(|_| ApiError::service_unavailable("Invalid public API URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::service_unavailable("Invalid public API URL"));
    }
    let path = url.path().trim_end_matches('/');
    let prefix = path.strip_suffix("/api/v1").unwrap_or(path).to_owned();
    url.set_path(&prefix);
    Ok(url)
}

fn destination_urls(
    mut api: Url,
    mut web: Url,
    pairs: &[(&str, &str)],
) -> (String, String, String) {
    api.set_path(&format!("{}/join", api.path().trim_end_matches('/')));
    api.query_pairs_mut().extend_pairs(pairs);
    web.set_path(&format!("{}/join", web.path().trim_end_matches('/')));
    web.query_pairs_mut().extend_pairs(pairs);
    let mut desktop = Url::parse("flow-like://join").expect("static desktop URL");
    desktop.query_pairs_mut().extend_pairs(pairs);
    (api.to_string(), web.to_string(), desktop.to_string())
}

/// The web app and the public API origins invitation pages link to.
fn public_origins(state: &AppState) -> Result<(Url, Url), ApiError> {
    let configured_base = std::env::var("FRONTEND_BASE_URL").ok();
    let configured = std::env::var("FRONTEND_URL").ok();
    let web = frontend_url(
        configured_base.as_deref(),
        configured.as_deref(),
        state.platform_config.app.as_deref(),
        state.platform_config.secure,
    )?;
    let public_api = std::env::var("PUBLIC_API_URL").ok();
    let api = api_url(
        public_api.as_deref(),
        &state.platform_config.domain,
        state.platform_config.secure,
    )?;
    Ok((api, web))
}

fn bounded_text(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

async fn load_page(state: &AppState, query: &InviteQuery) -> Result<Option<String>, ApiError> {
    let Some(mut metadata) = invitation_metadata(
        &state.db,
        query,
        i64::from(state.platform_config.max_users_prototype.unwrap_or(-1)),
    )
    .await?
    else {
        return Ok(None);
    };
    let (api, web) = public_origins(state)?;
    let (share_url, web_url, desktop_url) = destination_urls(api, web, &query.pairs());

    // Preview media is unused here; avoid signing unrelated screenshots.
    metadata.preview_media.clear();
    if metadata.icon.is_some() || metadata.thumbnail.is_some() {
        let credentials = state.master_credentials().await?;
        let store = credentials.to_store(false).await?;
        metadata
            .presign(
                MetaMode::App(query.app_id.clone()).media_prefix(&query.app_id),
                &store,
            )
            .await;
    }
    let icon = safe_image(metadata.icon);
    let image = safe_image(metadata.thumbnail).or_else(|| icon.clone());
    let name = bounded_text(&metadata.name, 120);
    Ok(Some(page::render_invitation(&page::InvitationPage {
        kind: page::InvitationKind::Project,
        name: if name.is_empty() {
            "Flow-Like project".into()
        } else {
            name
        },
        description: bounded_text(&metadata.description, 280),
        image,
        icon,
        desktop_url,
        web_url,
        share_url,
    })))
}

/// The inviter as the hub's lookup policy lets anyone holding the link see them: a name and an
/// avatar at most, never an email or a login handle.
struct Inviter {
    name: Option<String>,
    avatar: Option<String>,
}

async fn inviter(state: &AppState, user_id: &str) -> Result<Inviter, ApiError> {
    let lookup = &state.platform_config.lookup;
    let Some(row) = user::Entity::find_by_id(user_id).one(&state.db).await? else {
        return Ok(Inviter {
            name: None,
            avatar: None,
        });
    };
    let name = crate::routes::user::identity::public_display_name(
        row.name.as_deref().filter(|_| lookup.name),
        row.preferred_username
            .as_deref()
            .filter(|_| lookup.preferred_username),
        None,
    )
    .map(|name| bounded_text(&name, 80))
    .filter(|name| !name.is_empty());
    let avatar = match row.avatar.as_deref().filter(|_| lookup.avatar) {
        Some(avatar_id) => crate::routes::user::sign_avatar(&row.id, avatar_id, state)
            .await
            .ok()
            .and_then(|url| safe_image(Some(url))),
        None => None,
    };
    Ok(Inviter { name, avatar })
}

fn room_description(inviter: Option<&str>, members: usize) -> String {
    let people = if members == 1 {
        "1 member".to_string()
    } else {
        format!("{members} members")
    };
    match inviter {
        Some(name) => format!("{name} invited you to this group chat. {people}."),
        None => format!("You’re invited to this group chat. {people}."),
    }
}

/// A group chat's invitation. The group and its inviter are read only after the token proved to
/// be a live link.
async fn load_room_page(state: &AppState, query: &RoomQuery) -> Result<Option<String>, ApiError> {
    let Some(live) = links::live_link(&state.db, &query.room, Utc::now().fixed_offset()).await?
    else {
        return Ok(None);
    };
    let (api, web) = public_origins(state)?;
    let (share_url, web_url, desktop_url) =
        destination_urls(api, web, &[("room", query.room.as_str())]);
    let inviter = inviter(state, &live.link.created_by).await?;
    let picture = match live.conversation.image.as_deref() {
        Some(image) => crate::routes::inbox::image::image_url(state, &live.conversation.id, image)
            .await
            .and_then(|url| safe_image(Some(url))),
        None => None,
    };
    let name = live
        .conversation
        .title
        .as_deref()
        .map(|title| bounded_text(title, 120))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Group chat".into());
    Ok(Some(page::render_invitation(&page::InvitationPage {
        kind: page::InvitationKind::Room,
        name,
        description: bounded_text(
            &room_description(inviter.name.as_deref(), live.members),
            280,
        ),
        image: None,
        icon: picture.or(inviter.avatar),
        desktop_url,
        web_url,
        share_url,
    })))
}

fn html_response(status: StatusCode, html: String) -> Response {
    let mut response = (status, Html(html)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "x-robots-tag",
        HeaderValue::from_static("noindex, nofollow, noarchive"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    let script_hash = STANDARD.encode(Sha256::digest(page::SCRIPT.as_bytes()));
    let policy = format!(
        "default-src 'none'; img-src https: http:; style-src 'unsafe-inline'; script-src 'sha256-{script_hash}'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&policy).expect("static CSP with a base64 hash"),
    );
    response
}

async fn invitation(State(state): State<AppState>, uri: Uri) -> Response {
    let load = async {
        if let Ok(Query(room)) = Query::<RoomQuery>::try_from_uri(&uri) {
            return load_room_page(&state, &room).await;
        }
        match Query::<InviteQuery>::try_from_uri(&uri) {
            Ok(Query(query)) => load_page(&state, &query).await,
            Err(_) => Ok(None),
        }
    };
    // Leave time for the outer read deadline to send the neutral HTML response.
    match timeout(Duration::from_secs(8), load).await {
        Ok(Ok(Some(html))) => html_response(StatusCode::OK, html),
        Ok(Ok(None)) => html_response(StatusCode::OK, page::render_unavailable(false)),
        Ok(Err(_)) | Err(_) => {
            // Tokens and project metadata must never enter an error report.
            tracing::warn!("Invitation preview could not be loaded");
            html_response(
                StatusCode::SERVICE_UNAVAILABLE,
                page::render_unavailable(true),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, Schema, Set, sea_query::Table};

    fn query() -> InviteQuery {
        InviteQuery {
            app_id: "project".into(),
            token: "invitation-token".into(),
        }
    }

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        db.execute_unprepared(
            r#"CREATE TABLE public."InviteLink" (
                id TEXT PRIMARY KEY, "appId" TEXT, token TEXT,
                "expiresAt" TEXT, "countJoined" INTEGER, "maxUses" INTEGER
            );
            CREATE TABLE public."App" (
                id TEXT PRIMARY KEY, visibility TEXT, price INTEGER, "defaultRoleId" TEXT
            );
            CREATE TABLE public."Membership" (id TEXT PRIMARY KEY, "appId" TEXT);
            INSERT INTO public."InviteLink" VALUES
                ('link','project','invitation-token',NULL,0,1);
            INSERT INTO public."App" VALUES ('project','PUBLIC',0,'role');"#,
        )
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn rejected_links_never_load_project_metadata() {
        // There is deliberately no Meta table. Every rejected case must stop
        // before a branding query, even with a known app ID and valid token.
        for change in [
            r#"DELETE FROM public."InviteLink""#,
            r#"UPDATE public."InviteLink" SET token='another-token'"#,
            r#"UPDATE public."InviteLink" SET "appId"='another-project'"#,
            r#"UPDATE public."InviteLink" SET "expiresAt"='2000-01-01T00:00:00+00:00'"#,
            r#"UPDATE public."InviteLink" SET "countJoined"=1"#,
            r#"DELETE FROM public."App""#,
            r#"UPDATE public."App" SET visibility='PRIVATE'"#,
            r#"UPDATE public."App" SET visibility='OFFLINE'"#,
            r#"UPDATE public."App" SET price=100"#,
            r#"UPDATE public."App" SET "defaultRoleId"=NULL"#,
            r#"UPDATE public."App" SET "defaultRoleId"=''"#,
            r#"UPDATE public."App" SET visibility='PROTOTYPE';
               INSERT INTO public."Membership" VALUES ('member','project')"#,
        ] {
            let db = database().await;
            db.execute_unprepared(change).await.unwrap();
            assert!(
                invitation_metadata(&db, &query(), 1)
                    .await
                    .unwrap()
                    .is_none(),
                "metadata leaked after {change}"
            );
        }
    }

    #[tokio::test]
    async fn preview_validation_is_read_only_for_all_joinable_visibilities() {
        let db = database().await;
        let now = Utc::now().fixed_offset();
        for visibility in ["PUBLIC", "PUBLIC_REQUEST_ACCESS", "PROTOTYPE"] {
            db.execute_unprepared(&format!(
                r#"UPDATE public."App" SET visibility='{visibility}'"#
            ))
            .await
            .unwrap();
            for _ in 0..3 {
                assert!(can_preview(&db, &query(), 1, now).await.unwrap());
            }
        }
        let row = db
            .query_one_raw(sea_orm::Statement::from_string(
                db.get_database_backend(),
                r#"SELECT "countJoined" AS joined FROM public."InviteLink""#.to_owned(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "joined").unwrap(), 0);
        assert_eq!(
            membership::Entity::find()
                .select_only()
                .column(membership::Column::Id)
                .count(&db)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn exact_expiry_is_unavailable_and_unlimited_links_still_work() {
        let db = database().await;
        db.execute_unprepared(
            r#"UPDATE public."InviteLink" SET "expiresAt"='2030-01-01T00:00:00+00:00',
                "countJoined"=50, "maxUses"=-1"#,
        )
        .await
        .unwrap();
        let expiry = DateTime::parse_from_rfc3339("2030-01-01T00:00:00+00:00").unwrap();
        assert!(
            can_preview(&db, &query(), -1, expiry - chrono::Duration::seconds(1))
                .await
                .unwrap()
        );
        assert!(!can_preview(&db, &query(), -1, expiry).await.unwrap());
    }

    #[tokio::test]
    async fn valid_links_load_only_the_invited_projects_branding() {
        let db = database().await;
        let schema = Schema::new(db.get_database_backend());
        let source = schema.create_table_from_entity(meta::Entity);
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
        for (id, app_id, language, name) in [
            ("other", "other-project", "en", "Unrelated secret"),
            ("de", "project", "de", "German title"),
            ("en", "project", "en", "Invited project"),
        ] {
            meta::Entity::insert(meta::ActiveModel {
                id: Set(id.into()),
                app_id: Set(Some(app_id.into())),
                lang: Set(language.into()),
                name: Set(name.into()),
                description: Set(Some("Project description".into())),
                created_at: Set(Utc::now().fixed_offset()),
                updated_at: Set(Utc::now().fixed_offset()),
                ..Default::default()
            })
            .exec(&db)
            .await
            .unwrap();
        }
        let metadata = invitation_metadata(&db, &query(), -1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(metadata.name, "Invited project");
        assert_eq!(metadata.description, "Project description");
    }

    #[test]
    fn malformed_and_duplicate_parameters_are_rejected() {
        for query_string in [
            "appId=project",
            "token=invitation-token",
            "appId=project&token=a&token=b",
        ] {
            let uri = format!("/join?{query_string}").parse().unwrap();
            assert!(Query::<InviteQuery>::try_from_uri(&uri).is_err());
        }
        for bad_value in ["", "../project", "project token", "<script>", "%00"] {
            let mut malformed = query();
            malformed.token = bad_value.into();
            assert!(!malformed.is_valid());
        }
        let mut oversized = query();
        oversized.token = "a".repeat(129);
        assert!(!oversized.is_valid());
    }

    #[test]
    fn destinations_use_only_the_configured_origin_and_encoded_invitation() {
        let base = frontend_url(None, None, Some("app.example.com"), true).unwrap();
        let api = api_url(None, "api.example.com", true).unwrap();
        let (share, web, desktop) = destination_urls(api, base, &query().pairs());
        assert_eq!(
            share,
            "https://api.example.com/join?appId=project&token=invitation-token"
        );
        assert_eq!(
            web,
            "https://app.example.com/join?appId=project&token=invitation-token"
        );
        assert_eq!(
            desktop,
            "flow-like://join?appId=project&token=invitation-token"
        );
        for invalid in [
            "//evil.test",
            "https://example.com?next=evil",
            "https://example.com#fragment",
            "https://user:pass@example.com",
            "javascript:alert(1)",
            "https://example.com\\evil",
            " https://example.com",
            "https://example.com\n",
        ] {
            assert!(
                frontend_url(Some(invalid), None, Some("safe.example.com"), true).is_err(),
                "{invalid}"
            );
        }
        assert!(frontend_url(None, None, Some("//evil.test"), true).is_err());
        assert!(frontend_url(None, None, None, true).is_err());
        assert!(frontend_url(Some("http://localhost:3000/"), None, None, true).is_ok());
        assert_eq!(
            api_url(
                Some("https://api.example.com/api/v1"),
                "ignored.example.com",
                true
            )
            .unwrap()
            .as_str(),
            "https://api.example.com/"
        );
        assert_eq!(
            api_url(None, "localhost:8080", false).unwrap().as_str(),
            "http://localhost:8080/"
        );
        let prefixed = api_url(
            Some("https://api.example.com/prefix/api/v1"),
            "safe.example.com",
            true,
        )
        .unwrap();
        let (prefixed_share, _, _) = destination_urls(
            prefixed,
            frontend_url(Some("https://app.example.com"), None, None, true).unwrap(),
            &query().pairs(),
        );
        assert_eq!(
            prefixed_share,
            "https://api.example.com/prefix/join?appId=project&token=invitation-token"
        );
        assert!(
            api_url(
                Some("https://secret@api.example.com/api/v1"),
                "safe.example.com",
                true
            )
            .is_err()
        );
        assert!(api_url(None, "api.example.com/other", true).is_err());
    }

    #[test]
    fn local_frontend_settings_preserve_http_and_configured_prefixes() {
        let web = frontend_url(
            Some("http://localhost:3001"),
            Some("https://legacy.example.com"),
            Some("app.example.com"),
            true,
        )
        .unwrap();
        let (_, web, _) = destination_urls(
            api_url(None, "localhost:8080", false).unwrap(),
            web,
            &query().pairs(),
        );
        assert_eq!(
            web,
            "http://localhost:3001/join?appId=project&token=invitation-token"
        );
        assert_eq!(
            frontend_url(None, None, Some("localhost:3001"), false)
                .unwrap()
                .as_str(),
            "http://localhost:3001/"
        );
        assert_eq!(
            frontend_url(
                Some(""),
                Some("http://localhost:3002"),
                Some("ignored.example.com"),
                true
            )
            .unwrap()
            .as_str(),
            "http://localhost:3002/"
        );
        let prefixed =
            frontend_url(Some("https://web.example.com/workspace/"), None, None, true).unwrap();
        let (_, prefixed, _) = destination_urls(
            api_url(None, "api.example.com", true).unwrap(),
            prefixed,
            &query().pairs(),
        );
        assert_eq!(
            prefixed,
            "https://web.example.com/workspace/join?appId=project&token=invitation-token"
        );
    }

    #[test]
    fn images_reject_executable_and_credential_urls() {
        for invalid in [
            "javascript:alert(1)",
            "data:image/svg+xml,<svg/>",
            "file:///private/image.png",
            "//example.com/image.png",
            "https://secret@example.com/image.png",
            "https://example.com/\nimage.png",
        ] {
            assert!(safe_image(Some(invalid.into())).is_none(), "{invalid}");
        }
        assert_eq!(
            safe_image(Some("https://cdn.example.com/image.webp?sig=abc".into())).as_deref(),
            Some("https://cdn.example.com/image.webp?sig=abc")
        );
    }

    #[test]
    fn display_text_is_bounded_without_splitting_unicode() {
        assert_eq!(bounded_text("  東京 project  ", 2), "東京");
        assert_eq!(bounded_text(&"🧩".repeat(1000), 120).chars().count(), 120);
        assert!(bounded_text(" \n\t ", 120).is_empty());
    }

    #[test]
    fn neutral_pages_keep_tokens_out_of_caches_referrers_and_search_indexes() {
        let response = html_response(StatusCode::OK, page::render_unavailable(false));
        let headers = response.headers();
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(headers["x-robots-tag"], "noindex, nofollow, noarchive");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(csp.contains("script-src 'sha256-"));
        assert!(!csp.contains("script-src 'unsafe-inline'"));
        assert!(csp.contains("frame-ancestors 'none'"));
    }
}
