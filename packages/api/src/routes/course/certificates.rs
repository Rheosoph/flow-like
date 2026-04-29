use crate::{
    entity::{certificate, course, meta, user},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::course::{access::ensure_course_readable, progress::required_lessons_completed},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CertificateView {
    pub id: String,
    pub user_id: String,
    pub course_id: String,
    pub issued_at: String,
    pub hash: String,
    pub pdf_url: Option<String>,
    /// Public display name of the recipient, derived from the backend user
    /// profile. Populated by the verify endpoint and the personal certificate
    /// list; the raw model omits it.
    pub recipient_name: Option<String>,
    /// Localized course title at the time of viewing, populated like above.
    pub course_name: Option<String>,
}

impl From<certificate::Model> for CertificateView {
    fn from(c: certificate::Model) -> Self {
        Self {
            id: c.id,
            user_id: c.user_id,
            course_id: c.course_id,
            issued_at: c.issued_at.and_utc().to_rfc3339(),
            hash: c.hash,
            pdf_url: c.pdf_url,
            recipient_name: None,
            course_name: None,
        }
    }
}

fn verified_display_name(user: &user::Model) -> String {
    user.name
        .as_deref()
        .or(user.preferred_username.as_deref())
        .or(user.username.as_deref())
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| "Anonymous".to_string())
}

async fn enrich_certificate(
    state: &AppState,
    cert: certificate::Model,
    language: &str,
) -> Result<CertificateView, ApiError> {
    let recipient_name = user::Entity::find_by_id(&cert.user_id)
        .one(&state.db)
        .await?
        .map(|user| verified_display_name(&user))
        .or_else(|| Some("Anonymous".to_string()));

    let course_name = if let Some(_c) = course::Entity::find_by_id(&cert.course_id)
        .one(&state.db)
        .await?
    {
        let metas = meta::Entity::find()
            .filter(meta::Column::CourseId.eq(&cert.course_id))
            .all(&state.db)
            .await?;
        metas
            .iter()
            .find(|m| m.lang == language)
            .or_else(|| metas.first())
            .map(|m| m.name.clone())
    } else {
        None
    };

    let mut view: CertificateView = cert.into();
    view.recipient_name = recipient_name;
    view.course_name = course_name;
    Ok(view)
}

#[utoipa::path(
    post,
    path = "/courses/{course_id}/certificate",
    tag = "courses",
    params(("course_id" = String, Path, description = "Course identifier")),
    responses(
        (status = 200, description = "Issues a certificate when the course is fully completed", body = CertificateView),
        (status = 400, description = "Course not yet completed")
    )
)]
#[tracing::instrument(name = "POST /courses/{course_id}/certificate", skip(state, user))]
pub async fn issue_certificate(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
) -> Result<Json<CertificateView>, ApiError> {
    let sub = user.sub()?;
    ensure_course_readable(&state, &user, &course_id).await?;

    let existing = certificate::Entity::find()
        .filter(certificate::Column::UserId.eq(&sub))
        .filter(certificate::Column::CourseId.eq(&course_id))
        .one(&state.db)
        .await?;
    if let Some(c) = existing {
        return Ok(Json(enrich_certificate(&state, c, "en").await?));
    }

    if !required_lessons_completed(&state, &sub, &course_id).await? {
        return Err(ApiError::FORBIDDEN);
    }

    let now = chrono::Utc::now().naive_utc();
    let mut hasher = Sha256::new();
    hasher.update(sub.as_bytes());
    hasher.update(b"|");
    hasher.update(course_id.as_bytes());
    hasher.update(b"|");
    hasher.update(now.and_utc().timestamp().to_string().as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    let active = certificate::ActiveModel {
        id: Set(create_id()),
        user_id: Set(sub),
        course_id: Set(course_id),
        issued_at: Set(now),
        hash: Set(hash),
        pdf_url: Set(None),
    };
    let saved = active.insert(&state.db).await?;
    Ok(Json(enrich_certificate(&state, saved, "en").await?))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct LanguageOnlyQuery {
    pub language: Option<String>,
}

#[utoipa::path(
    get,
    path = "/courses/certificates/me",
    tag = "courses",
    params(("language" = Option<String>, Query, description = "Preferred language (default: en)")),
    responses(
        (status = 200, description = "Returns the current user's certificates", body = Vec<CertificateView>)
    )
)]
#[tracing::instrument(name = "GET /courses/certificates/me", skip(state, user, q))]
pub async fn list_my_certificates(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<LanguageOnlyQuery>,
) -> Result<Json<Vec<CertificateView>>, ApiError> {
    let sub = user.sub()?;
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());
    let rows = certificate::Entity::find()
        .filter(certificate::Column::UserId.eq(sub))
        .all(&state.db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(enrich_certificate(&state, row, &language).await?);
    }
    Ok(Json(out))
}

#[utoipa::path(
    get,
    path = "/courses/certificates/verify/{cert_id}",
    tag = "courses",
    params(
        ("cert_id" = String, Path, description = "Certificate identifier or SHA-256 verification hash"),
        ("language" = Option<String>, Query, description = "Preferred language (default: en)")
    ),
    responses(
        (status = 200, description = "Returns certificate details (recipient name, course name, hash) if the certificate exists", body = CertificateView),
        (status = 404, description = "Certificate not found")
    )
)]
#[tracing::instrument(name = "GET /courses/certificates/verify/{cert_id}", skip(state, q))]
pub async fn verify_certificate(
    State(state): State<AppState>,
    Path(cert_id): Path<String>,
    Query(q): Query<LanguageOnlyQuery>,
) -> Result<Json<CertificateView>, ApiError> {
    let row = match certificate::Entity::find_by_id(&cert_id)
        .one(&state.db)
        .await?
    {
        Some(row) => row,
        None => certificate::Entity::find()
            .filter(certificate::Column::Hash.eq(&cert_id))
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?,
    };
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());
    Ok(Json(enrich_certificate(&state, row, &language).await?))
}
