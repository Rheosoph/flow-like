use std::sync::{Arc, Mutex, PoisonError};

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        db::{ScopeParams, resolve_connection},
        page::get_page::if_none_match_matches,
        wasm_catalog::app_wasm_nodes_cached,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use flow_like::{flow::node::Node, state::FlowNodeRegistryInner};
use flow_like_catalog::wire::{NodeIds, catalog_document, encode_section};
use flow_like_storage::databases::graph::lancegraph::{
    self, GraphOverlayDef, RemoteOntologyImportDef,
};
use flow_like_types::anyhow;
use serde::Deserialize;

/// What an app adds depends on the caller's role and on the app's packages, so only the
/// caller's own cache may keep it, and it confirms the copy with the hub before each reuse.
/// That check compares against the tag computed for whoever asks, which is why the answer does
/// not vary on `Authorization`: a rotated token must not cost a download of unchanged content.
const CATALOG_CACHE: &str = "private, no-cache";

/// A built-in catalog requested by its digest can never change under that address.
const PINNED_CATALOG_CACHE: &str = "private, max-age=31536000, immutable";

#[derive(Debug, Default, Deserialize)]
pub struct CatalogQuery {
    /// `lean` selects the sectioned catalog. Anything else returns the plain node list.
    pub format: Option<String>,
    /// The built-in catalog the caller expects, as named by an app's lean catalog.
    pub digest: Option<String>,
}

impl CatalogQuery {
    fn lean(&self) -> bool {
        self.format.as_deref() == Some("lean")
    }
}

/// The built-in catalog as it goes over the wire. It only changes with a deploy, so it is
/// encoded once per process instead of once per request, and it is the same for every app:
/// an app's catalog names it by digest instead of repeating it.
struct BuiltInCatalog {
    fingerprint: [u8; 32],
    digest: String,
    body: String,
}

impl BuiltInCatalog {
    fn of(fingerprint: [u8; 32], section: &str) -> Self {
        let digest = blake3::hash(section.as_bytes()).to_hex().to_string();
        let etag = format!("\"{digest}\"");
        let body = catalog_document(&digest, Some(&etag), section);
        Self {
            fingerprint,
            digest,
            body,
        }
    }

    fn etag(&self) -> String {
        format!("\"{}\"", self.digest)
    }
}

static BUILT_IN_CATALOG: Mutex<Option<Arc<BuiltInCatalog>>> = Mutex::new(None);

async fn built_in_catalog(
    registry: &Arc<FlowNodeRegistryInner>,
) -> Result<Arc<BuiltInCatalog>, ApiError> {
    let fingerprint = registry.fingerprint();
    let cached = BUILT_IN_CATALOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(cached) = cached
        && cached.fingerprint == fingerprint
    {
        return Ok(cached);
    }

    let registry = registry.clone();
    let section = flow_like_types::tokio::task::spawn_blocking(move || {
        let nodes = registry.get_nodes_shared();
        let section = encode_section(nodes.as_slice(), NodeIds::Unstable)?;
        serde_json::to_string(&section)
    })
    .await
    .map_err(|error| {
        ApiError::internal_error(anyhow!(
            "encoding the node catalog was interrupted: {error}"
        ))
    })??;

    let built_in = Arc::new(BuiltInCatalog::of(fingerprint, &section));
    *BUILT_IN_CATALOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(built_in.clone());
    Ok(built_in)
}

fn catalog_response(etag: &str, cache: &str, body: String) -> Response {
    (
        [
            (header::ETAG, etag),
            (header::CACHE_CONTROL, cache),
            (header::CONTENT_TYPE, "application/json"),
        ],
        body,
    )
        .into_response()
}

fn not_modified(etag: &str) -> Response {
    (
        StatusCode::NOT_MODIFIED,
        [(header::ETAG, etag), (header::CACHE_CONTROL, CATALOG_CACHE)],
    )
        .into_response()
}

/// The built-in catalog on its own. Asked for by digest it is cacheable for good; a digest
/// this process does not serve is refused, so a caller never pairs an app's nodes with a
/// built-in catalog they were not derived from (two builds answer side by side in a deploy).
fn lean_built_in(
    headers: &HeaderMap,
    wanted: Option<&str>,
    built_in: &BuiltInCatalog,
) -> Result<Response, ApiError> {
    let etag = built_in.etag();
    match wanted {
        Some(wanted) if wanted == built_in.digest => Ok(catalog_response(
            &etag,
            PINNED_CATALOG_CACHE,
            built_in.body.clone(),
        )),
        Some(wanted) => Err(ApiError::not_found(format!(
            "This hub serves built-in node catalog {}, not {wanted}",
            built_in.digest
        ))),
        None if if_none_match_matches(headers, &etag) => Ok(not_modified(&etag)),
        None => Ok(catalog_response(
            &etag,
            CATALOG_CACHE,
            built_in.body.clone(),
        )),
    }
}

/// What an app adds to the built-in catalog, plus the digest of that catalog. The tag covers
/// both, so neither a package install nor a deploy is ever answered with a 304.
fn lean_app_catalog(
    headers: &HeaderMap,
    built_in: &BuiltInCatalog,
    extras: &[Node],
) -> Result<Response, ApiError> {
    let extras = serde_json::to_string(&encode_section(extras, NodeIds::Stable)?)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(built_in.digest.as_bytes());
    hasher.update(extras.as_bytes());
    let etag = format!("\"{}\"", hasher.finalize().to_hex());
    if if_none_match_matches(headers, &etag) {
        return Ok(not_modified(&etag));
    }
    let body = catalog_document(&built_in.digest, Some(&etag), &extras);
    Ok(catalog_response(&etag, CATALOG_CACHE, body))
}

fn local_presets(definitions: Vec<GraphOverlayDef>, built_in: &[Node]) -> Vec<Node> {
    let ontologies = definitions
        .into_iter()
        .map(crate::routes::app::graph::list_overlays::def_to_overlay)
        .collect::<Vec<_>>();
    flow_like_catalog_core::ontology_binding_nodes(&ontologies, built_in)
}

fn remote_presets(
    app_id: &str,
    definitions: Vec<RemoteOntologyImportDef>,
    built_in: &[Node],
) -> Vec<Node> {
    let imports = definitions
        .into_iter()
        .map(crate::routes::app::graph::list_imports::def_to_import)
        .collect::<Result<Vec<_>, _>>();
    match imports {
        Ok(imports) => flow_like_catalog_core::remote_ontology_binding_nodes(&imports, built_in),
        Err(error) => {
            tracing::warn!(
                app_id,
                %error,
                "Could not decode remote Data Studio bindings"
            );
            Vec::new()
        }
    }
}

/// The presets an app's Data Studio ontologies add to the palette. A project database that
/// cannot be read leaves the catalog without them instead of failing it.
async fn data_studio_presets(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    built_in: &[Node],
) -> Vec<Node> {
    let scope = ScopeParams { scope: None };
    let connection = match resolve_connection(state, user, app_id, &scope).await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(
                app_id,
                %error,
                "Could not open the project database for Data Studio bindings"
            );
            return Vec::new();
        }
    };
    let (ontologies, imports) = flow_like_types::tokio::join!(
        lancegraph::list_overlays(&connection),
        lancegraph::list_ontology_imports(&connection)
    );

    let mut presets = match ontologies {
        Ok(definitions) => local_presets(definitions, built_in),
        Err(error) => {
            tracing::warn!(
                app_id,
                %error,
                "Could not load Data Studio bindings; returning the base catalog"
            );
            Vec::new()
        }
    };
    match imports {
        Ok(definitions) => presets.extend(remote_presets(app_id, definitions, built_in)),
        Err(error) => tracing::warn!(
            app_id,
            %error,
            "Could not load remote Data Studio bindings"
        ),
    }
    presets
}

/// What the app adds to the built-in catalog: Data Studio presets, then its package nodes.
async fn app_nodes(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    with_bindings: bool,
    built_in: &[Node],
) -> Result<Vec<Node>, ApiError> {
    let mut nodes = if with_bindings {
        data_studio_presets(state, user, app_id, built_in).await
    } else {
        Vec::new()
    };
    let wasm = app_wasm_nodes_cached(state, app_id).await?;
    nodes.extend(wasm.nodes.iter().cloned());
    Ok(nodes)
}

#[utoipa::path(
    get,
    path = "/apps/nodes",
    tag = "apps",
    params(
        ("format" = Option<String>, Query, description = "Set to `lean` for the compact built-in catalog, which carries an ETag and answers 304 when unchanged"),
        ("digest" = Option<String>, Query, description = "With `format=lean`: the built-in catalog an app's catalog named. The answer can then be cached for good")
    ),
    responses(
        (status = 200, description = "List of available nodes", body = Vec<Object>),
        (status = 304, description = "The compact catalog the caller already holds is current"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "This hub serves a different built-in catalog than the digest asked for")
    )
)]
#[tracing::instrument(name = "GET /apps/nodes", skip(state, user, query, headers))]
pub async fn get_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<CatalogQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    user.sub()?;

    if query.lean() {
        let built_in = built_in_catalog(&state.registry).await?;
        return lean_built_in(&headers, query.digest.as_deref(), &built_in);
    }

    Ok(Json(state.registry.get_nodes()).into_response())
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/nodes",
    tag = "apps",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("format" = Option<String>, Query, description = "Set to `lean` for the app's own nodes in compact form plus the digest of the built-in catalog they extend. Carries an ETag and answers 304 when unchanged")
    ),
    responses(
        (status = 200, description = "List of available nodes for this app", body = Vec<Object>),
        (status = 304, description = "The compact catalog the caller already holds is current"),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/nodes", skip(state, user, query, headers))]
pub async fn get_app_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<CatalogQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let with_bindings = permission.has_permission(RolePermissions::ReadDatabase)
        || permission.has_permission(RolePermissions::ReadFiles);

    let built_in = state.registry.get_nodes_shared();
    let extras = app_nodes(&state, &user, &app_id, with_bindings, &built_in).await?;

    if query.lean() {
        let encoded = built_in_catalog(&state.registry).await?;
        return lean_app_catalog(&headers, &encoded, &extras);
    }

    let mut nodes = built_in.as_ref().clone();
    nodes.extend(extras);
    Ok(Json(nodes).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use flow_like::flow::variable::VariableType;
    use serde_json::Value;

    fn node(name: &str, id: &str) -> Node {
        let mut node = Node::new(name, name, "A node used by the catalog tests", "Tests");
        node.id = id.to_string();
        node.add_input_pin("exec_in", "In", "Starts the node", VariableType::Execution);
        node
    }

    fn built_in(nodes: &[Node]) -> BuiltInCatalog {
        let section = encode_section(nodes, NodeIds::Unstable).expect("built-in section");
        let section = serde_json::to_string(&section).expect("built-in json");
        BuiltInCatalog::of([0; 32], &section)
    }

    fn header_of(response: &Response, name: header::HeaderName) -> String {
        response.headers()[name]
            .to_str()
            .expect("header text")
            .to_string()
    }

    fn etag_of(response: &Response) -> String {
        header_of(response, header::ETAG)
    }

    fn asking_with(etag: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag.parse().expect("header value"));
        headers
    }

    async fn body_of(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&body).expect("json body")
    }

    #[tokio::test]
    async fn an_apps_catalog_names_the_built_in_one_instead_of_carrying_it() {
        let base = built_in(&[node("alpha", "random-1")]);
        let extras = [node("alpha", "preset_one")];
        let response = lean_app_catalog(&HeaderMap::new(), &base, &extras).expect("catalog");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header_of(&response, header::CACHE_CONTROL), CATALOG_CACHE);
        let etag = etag_of(&response);
        let body = body_of(response).await;
        assert_eq!(body["v"], flow_like_catalog::wire::CATALOG_WIRE_VERSION);
        assert_eq!(body["built_in"], base.digest.as_str());
        assert_eq!(body["etag"], etag.as_str());
        let sections = body["sections"].as_array().expect("sections");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0]["nodes"][0]["id"], "preset_one");

        let unchanged = lean_app_catalog(&asking_with(&etag), &base, &extras).expect("catalog");
        assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(etag_of(&unchanged), etag);
    }

    #[test]
    fn an_apps_validator_follows_its_own_nodes_and_the_built_in_catalog() {
        let base = built_in(&[node("alpha", "random-1")]);
        let none = lean_app_catalog(&HeaderMap::new(), &base, &[]).expect("catalog");
        let preset = lean_app_catalog(&HeaderMap::new(), &base, &[node("alpha", "preset_one")])
            .expect("catalog");
        let other_base = built_in(&[node("beta", "random-1")]);
        let other = lean_app_catalog(&HeaderMap::new(), &other_base, &[]).expect("catalog");

        assert_ne!(etag_of(&none), etag_of(&preset));
        assert_ne!(etag_of(&none), etag_of(&other));
    }

    #[tokio::test]
    async fn the_built_in_catalog_is_pinned_when_asked_for_by_digest() {
        let base = built_in(&[node("alpha", "random-1")]);

        let pinned = lean_built_in(&HeaderMap::new(), Some(&base.digest), &base).expect("pinned");
        assert_eq!(
            header_of(&pinned, header::CACHE_CONTROL),
            PINNED_CATALOG_CACHE
        );
        let body = body_of(pinned).await;
        assert_eq!(body["built_in"], base.digest.as_str());
        assert_eq!(body["etag"], base.etag().as_str());
        assert!(body["sections"][0]["nodes"][0].get("id").is_none());

        let current = lean_built_in(&HeaderMap::new(), None, &base).expect("current");
        assert_eq!(header_of(&current, header::CACHE_CONTROL), CATALOG_CACHE);
        let unchanged = lean_built_in(&asking_with(&base.etag()), None, &base).expect("unchanged");
        assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn a_digest_this_hub_does_not_serve_is_refused() {
        let base = built_in(&[node("alpha", "random-1")]);
        let refused = lean_built_in(&HeaderMap::new(), Some("0000"), &base)
            .expect_err("another build's catalog");
        assert_eq!(refused.into_response().status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn the_same_built_in_catalog_has_one_digest_in_every_process() {
        // A second process builds the same nodes with different random ids.
        let here = built_in(&[node("alpha", "random-1"), node("beta", "random-2")]);
        let there = built_in(&[node("beta", "random-9"), node("alpha", "random-8")]);
        assert_eq!(here.digest, there.digest);
        assert_eq!(here.body, there.body);
    }
}
