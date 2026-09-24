use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::time::Instant;

use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use flow_like_types::Value;
use sea_orm::IsolationLevel;
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use utoipa::{IntoParams, ToSchema};

use crate::entity::sea_orm_active_enums::{Category, WasmPackageCategory as DbPackageCategory};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::routes::explore::audience::{self, SUPPORTED_LOCALES};
use crate::routes::explore::edition::{self, Edition, Loaded};
use crate::routes::explore::hydrate::{self, AppMap, PackageMap, RuleItems, Wanted};
use crate::routes::explore::model::{
    CategoryFilter, FacetKind, ItemKind, LayoutDoc, MatchedVia, PermissionGroup, PlacementContent,
    PlacementDoc, Platform, PriceFilter, Projection, SlotArea, SlotTrace, Viewer,
    app_categories_for, package_categories_for, package_category_to_db, parse_app_category,
    parse_categories, parse_permissions,
};
use crate::routes::explore::query::{
    self, AppCategoryPair, AppFilter, ExploreSort, PACKAGE_WINDOW, PackageCategoryPair,
    PackageFilter,
};
use crate::routes::explore::rails;
use crate::routes::explore::resolve::{
    self, Hydrated, ResolvedCollection, ResolvedExplore, ResolvedItem, ViewerEcho,
};
use crate::routes::registry::types::PackageSummary;
use crate::state::AppState;
use flow_like_wasm_schema::manifest::WasmPackageCategory;

const PAGE_CACHE_PREFIX: &str = "explore:v1";
const QUERY_MAX: usize = 100;
const PAGE_DEFAULT: u64 = 24;
const PAGE_MAX: u64 = 48;
const COLLECTION_HITS_MAX: usize = 3;
const RELATED_MAX: u64 = 4;
/// `?collection=` runs rules at four times their limit and shows at most this many items.
const COLLECTION_PAGE_SCALE: u8 = 4;
const COLLECTION_PAGE_ITEMS: usize = 48;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExploreQuery {
    /// `desktop` or `web` (default).
    pub platform: Option<String>,
    /// Viewer language, one of the hub's locales or a regional variant of one (default `en`).
    pub language: Option<String>,
    /// `true` when the viewer has developer mode on: packages and package rails appear.
    pub dev: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExploreSearchQuery {
    /// Search text, at most 100 characters.
    pub q: Option<String>,
    /// `all` (default), `apps`, `packages` or `collections`.
    #[serde(rename = "type")]
    #[param(rename = "type")]
    pub kind: Option<String>,
    /// Comma-separated category facet values, e.g. `app:Finance,package:EDUCATION` (at most 16).
    pub categories: Option<String>,
    /// `free` or `paid`.
    pub price: Option<String>,
    /// `true` shows verified packages only.
    pub verified: Option<String>,
    /// Comma-separated package permission groups: `none`, `network`, `models`, `storage`.
    pub permissions: Option<String>,
    /// `best` (default), `newest`, `rating`, `installs`, `name` or `updated`.
    pub sort: Option<String>,
    /// Id of a curated collection to open instead of searching.
    pub collection: Option<String>,
    pub apps_offset: Option<u64>,
    /// Apps per page, at most 48 (default 24).
    pub apps_limit: Option<u64>,
    pub packages_offset: Option<u64>,
    /// Packages per page, at most 48 (default 24).
    pub packages_limit: Option<u64>,
    /// `desktop` or `web` (default).
    pub platform: Option<String>,
    /// Viewer language (default `en`).
    pub language: Option<String>,
    /// `true` when the viewer has developer mode on.
    pub dev: Option<String>,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchGroup<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub has_more: bool,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchPackageHit {
    pub package: PackageSummary,
    /// `name` when the package's own name, description or id matches; `collection` when a matching curated
    /// collection lists it.
    pub matched_via: MatchedVia,
    pub collection_title: Option<String>,
}

/// One category facet row. `value` round-trips through the `categories` parameter unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FacetCount {
    #[schema(example = "app:Finance")]
    pub value: String,
    pub kind: FacetKind,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypeFacet {
    pub apps: u64,
    pub packages: u64,
    pub collections: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PriceFacet {
    pub free: u64,
    pub paid: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PermissionFacet {
    pub none: u64,
    pub network: u64,
    pub models: u64,
    pub storage: u64,
}

/// Facet counts; each group is counted over every filter except its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExploreFacets {
    pub types: TypeFacet,
    pub categories: Vec<FacetCount>,
    pub price: PriceFacet,
    pub verified: u64,
    pub permissions: PermissionFacet,
    /// The permission filter only looked at the first 200 matching packages.
    pub packages_capped: bool,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExploreSearchResponse {
    pub query: String,
    pub viewer: ViewerEcho,
    pub collections: Vec<ResolvedCollection>,
    pub apps: SearchGroup<ResolvedItem>,
    pub packages: SearchGroup<SearchPackageHit>,
    pub related: Vec<ResolvedItem>,
    pub facets: ExploreFacets,
}

pub(crate) fn parse_flag(param: &str, raw: Option<&str>) -> Result<bool, ApiError> {
    match raw.map(str::trim) {
        None | Some("" | "false" | "0") => Ok(false),
        Some("true" | "1") => Ok(true),
        Some(other) => Err(ApiError::bad_request(format!(
            "{param} value '{other}' is not supported; expected true or false"
        ))),
    }
}

fn parse_platform(raw: Option<&str>) -> Result<Platform, ApiError> {
    match raw.map(str::trim) {
        None | Some("" | "web") => Ok(Platform::Web),
        Some("desktop") => Ok(Platform::Desktop),
        Some(other) => Err(ApiError::bad_request(format!(
            "platform value '{other}' is not supported; expected desktop or web"
        ))),
    }
}

/// The viewer class a page resolves for. `dev` is a presentation filter over public registry data that
/// protects nothing, so it comes from the caller's hint for everyone; a hub without a registry never has it.
pub(crate) fn viewer(
    state: &AppState,
    signed_in: bool,
    platform: Option<&str>,
    language: Option<&str>,
    dev: Option<&str>,
) -> Result<Viewer, ApiError> {
    Ok(Viewer {
        dev: parse_flag("dev", dev)? && state.wasm_registry.is_some(),
        signed_in,
        platform: parse_platform(platform)?,
        language: audience::parse_language(language)?,
    })
}

pub(crate) fn page_cache_key(revision: &str, viewer: &Viewer) -> String {
    format!(
        "{PAGE_CACHE_PREFIX}:{revision}:{}:{}:{}:{}",
        viewer.dev,
        viewer.signed_in,
        viewer.platform.as_str(),
        viewer.language
    )
}

fn viewer_classes() -> impl Iterator<Item = Viewer> {
    [false, true].into_iter().flat_map(|dev| {
        [false, true].into_iter().flat_map(move |signed_in| {
            [Platform::Desktop, Platform::Web]
                .into_iter()
                .flat_map(move |platform| {
                    SUPPORTED_LOCALES.into_iter().map(move |language| Viewer {
                        dev,
                        signed_in,
                        platform,
                        language: language.to_owned(),
                    })
                })
        })
    })
}

/// Drops this replica's cached pages of `revision`, for every viewer class.
pub(crate) fn forget_pages(state: &AppState, revision: &str) {
    for viewer in viewer_classes() {
        state.invalidate_cache(&page_cache_key(revision, &viewer));
    }
}

async fn timed<F: Future>(stage: &'static str, work: F) -> F::Output {
    let started = Instant::now();
    let output = work
        .instrument(tracing::info_span!("explore_stage", stage))
        .await;
    tracing::debug!(stage, elapsed = ?started.elapsed(), "Explore stage finished");
    output
}

/// An edition's header and rows from one REPEATABLE READ snapshot, so the revision names exactly these rows.
pub(crate) async fn load_edition(state: &AppState, edition: Edition) -> Result<Loaded, ApiError> {
    state
        .transaction_with(IsolationLevel::RepeatableRead, move |txn| {
            Box::pin(edition::load(txn, edition))
        })
        .await
}

/// Resolves `loaded` for `viewer`: one shared hydration, then selection for `all` and, for developer viewers,
/// the `apps` and `packages` views. The trace is the `all` view's.
pub(crate) async fn resolve_page(
    state: &AppState,
    loaded: &Loaded,
    viewer: &Viewer,
    now: DateTime<Utc>,
) -> Result<(ResolvedExplore, Vec<SlotTrace>), ApiError> {
    let candidates = resolve::candidates(&loaded.layout, viewer, now);
    let (hydrated, type_counts) = futures::try_join!(
        timed("hydrate", hydrate::hydrate(state, &candidates, viewer, now)),
        timed("type_counts", rails::type_counts(&state.db, viewer.dev)),
    )?;
    let (views, trace) = timed("select", async {
        resolve::resolve_views(&candidates, &hydrated, viewer)
    })
    .await;
    Ok((
        ResolvedExplore {
            revision: loaded.revision().to_owned(),
            generated_at: now,
            viewer: ViewerEcho::from(viewer),
            type_counts,
            views,
        },
        trace,
    ))
}

#[utoipa::path(
    get,
    path = "/store/explore",
    tag = "store",
    description = "The Explore storefront for your viewer class: the curated grid and rows for all items, plus separate apps-only and packages-only views when developer mode is on. Pages are cached for a minute.",
    params(ExploreQuery),
    responses(
        (status = 200, description = "The resolved Explore page", body = ResolvedExplore),
        (status = 400, description = "Unknown platform, language or dev value"),
        (status = 401, description = "This hub requires signing in to browse")
    )
)]
#[tracing::instrument(name = "GET /store/explore", skip_all)]
pub async fn get_explore(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ExploreQuery>,
) -> Result<Json<Value>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }
    let viewer = viewer(
        &state,
        user.sub().is_ok(),
        query.platform.as_deref(),
        query.language.as_deref(),
        query.dev.as_deref(),
    )?;
    let header = timed("header", edition::header(&state.db, Edition::Live)).await?;
    let revision = header
        .as_ref()
        .map_or(edition::DEFAULT_REVISION, |header| header.revision.as_str());
    if let Some(page) = state.get_cache::<Value>(&page_cache_key(revision, &viewer)) {
        return Ok(Json(page));
    }
    let loaded = timed("load", load_edition(&state, Edition::Live)).await?;
    let (page, _) = resolve_page(&state, &loaded, &viewer, Utc::now()).await?;
    let page = serde_json::to_value(page)?;
    state.set_cache(page_cache_key(loaded.revision(), &viewer), &page);
    Ok(Json(page))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SearchScope {
    #[default]
    All,
    Apps,
    Packages,
    Collections,
}

impl SearchScope {
    fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        match raw.map(str::trim) {
            None | Some("" | "all") => Ok(Self::All),
            Some("apps") => Ok(Self::Apps),
            Some("packages") => Ok(Self::Packages),
            Some("collections") => Ok(Self::Collections),
            Some(other) => Err(ApiError::bad_request(format!(
                "type value '{other}' is not supported; expected all, apps, packages or collections"
            ))),
        }
    }

    fn projection(self) -> Projection {
        match self {
            Self::Apps => Projection::Apps,
            Self::Packages => Projection::Packages,
            Self::All | Self::Collections => Projection::All,
        }
    }

    fn lists_apps(self) -> bool {
        matches!(self, Self::All | Self::Apps)
    }

    fn lists_packages(self) -> bool {
        matches!(self, Self::All | Self::Packages)
    }

    fn lists_collections(self) -> bool {
        matches!(self, Self::All | Self::Collections)
    }
}

/// Which kinds the category and price facets count: apps only for non-developers and `type=apps`, packages
/// only for `type=packages`, both otherwise (and then app rows also count the packages they expand to).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FacetScope {
    apps: bool,
    packages: bool,
}

impl FacetScope {
    fn of(scope: SearchScope, dev: bool) -> Self {
        match (dev, scope) {
            (false, _) | (true, SearchScope::Apps) => Self {
                apps: true,
                packages: false,
            },
            (true, SearchScope::Packages) => Self {
                apps: false,
                packages: true,
            },
            (true, SearchScope::All | SearchScope::Collections) => Self {
                apps: true,
                packages: true,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Paging {
    offset: u64,
    limit: u64,
}

impl Paging {
    fn parse(param: &str, offset: Option<u64>, limit: Option<u64>) -> Result<Self, ApiError> {
        let limit = limit.unwrap_or(PAGE_DEFAULT);
        if limit > PAGE_MAX {
            return Err(ApiError::bad_request(format!(
                "{param}_limit must be at most {PAGE_MAX} (got {limit})"
            )));
        }
        Ok(Self {
            offset: offset.unwrap_or_default(),
            limit,
        })
    }

    fn end(self) -> u64 {
        self.offset.saturating_add(self.limit)
    }

    fn slice<T>(self, items: Vec<T>) -> Vec<T> {
        let to_usize = |value: u64| usize::try_from(value).unwrap_or(usize::MAX);
        items
            .into_iter()
            .skip(to_usize(self.offset))
            .take(to_usize(self.limit))
            .collect()
    }

    fn group<T>(self, items: Vec<T>, total: u64) -> SearchGroup<T> {
        let shown = u64::try_from(items.len()).unwrap_or(u64::MAX);
        SearchGroup {
            has_more: self.offset.saturating_add(shown) < total,
            items,
            total,
        }
    }
}

fn empty_group<T>() -> SearchGroup<T> {
    SearchGroup {
        items: Vec::new(),
        total: 0,
        has_more: false,
    }
}

fn count_of(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug)]
struct SearchRequest {
    text: Option<String>,
    scope: SearchScope,
    categories: Vec<CategoryFilter>,
    price: Option<PriceFilter>,
    verified: bool,
    permissions: Vec<PermissionGroup>,
    sort: ExploreSort,
    collection: Option<String>,
    apps: Paging,
    packages: Paging,
}

fn parse_price(raw: Option<&str>) -> Result<Option<PriceFilter>, ApiError> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some("free") => Ok(Some(PriceFilter::Free)),
        Some("paid") => Ok(Some(PriceFilter::Paid)),
        Some(other) => Err(ApiError::bad_request(format!(
            "price value '{other}' is not supported; expected free or paid"
        ))),
    }
}

impl SearchRequest {
    fn parse(params: &ExploreSearchQuery) -> Result<Self, ApiError> {
        let text = params
            .q
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if let Some(text) = text {
            let length = text.chars().count();
            if length > QUERY_MAX {
                return Err(ApiError::bad_request(format!(
                    "q must be at most {QUERY_MAX} characters (got {length})"
                )));
            }
        }
        Ok(Self {
            text: text.map(str::to_owned),
            scope: SearchScope::parse(params.kind.as_deref())?,
            categories: parse_categories(params.categories.as_deref())?,
            price: parse_price(params.price.as_deref())?,
            verified: parse_flag("verified", params.verified.as_deref())?,
            permissions: parse_permissions(params.permissions.as_deref())?,
            sort: ExploreSort::parse(params.sort.as_deref())?,
            collection: params
                .collection
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned),
            apps: Paging::parse("apps", params.apps_offset, params.apps_limit)?,
            packages: Paging::parse("packages", params.packages_offset, params.packages_limit)?,
        })
    }

    /// Selected `app:` categories; `None` without a category filter, empty when only `package:` values are set.
    fn app_categories(&self) -> Option<Vec<Category>> {
        (!self.categories.is_empty()).then(|| {
            self.categories
                .iter()
                .filter_map(CategoryFilter::app_category)
                .map(Category::from)
                .collect()
        })
    }

    /// Selected `package:` categories plus the package categories every selected `app:` value expands to.
    fn package_categories(&self) -> Option<Vec<DbPackageCategory>> {
        (!self.categories.is_empty()).then(|| {
            let mut categories = Vec::new();
            for filter in &self.categories {
                let expanded: Vec<WasmPackageCategory> = match filter {
                    CategoryFilter::Package(category) => vec![*category],
                    CategoryFilter::App(_) => filter
                        .app_category()
                        .map(|app| package_categories_for(&app).to_vec())
                        .unwrap_or_default(),
                };
                for category in expanded.into_iter().map(package_category_to_db) {
                    if !categories.contains(&category) {
                        categories.push(category);
                    }
                }
            }
            categories
        })
    }

    fn app_filter(&self, language: &str) -> AppFilter {
        AppFilter {
            text: self.text.clone(),
            categories: self.app_categories(),
            price: self.price,
            ..AppFilter::new(language)
        }
    }

    fn package_filter(&self, collection_ids: Vec<String>) -> PackageFilter {
        PackageFilter {
            text: self.text.clone(),
            collection_ids,
            categories: self.package_categories(),
            price: self.price,
            verified_only: self.verified,
            ..PackageFilter::default()
        }
    }

    fn matches_permissions(&self, package: &PackageSummary) -> bool {
        self.permissions
            .iter()
            .any(|group| group.matches(&package.capabilities))
    }
}

/// Category rows from `(primary, secondary)` pair counts: each pair adds its count to every category of the
/// set {primary, secondary}, so primary-or-secondary filters and counts agree. With both kinds in scope, an app
/// row also counts the packages whose categories expand to it, each package once. Selected values keep their
/// row at 0; other empty rows are dropped. App rows come first, each kind by count, then value.
fn category_facets(
    app_pairs: &[AppCategoryPair],
    package_pairs: &[PackageCategoryPair],
    scope: FacetScope,
    selected: &[CategoryFilter],
) -> Vec<FacetCount> {
    let mut apps: BTreeMap<String, u64> = BTreeMap::new();
    let mut packages: BTreeMap<String, u64> = BTreeMap::new();
    if scope.apps {
        for (primary, secondary, count) in app_pairs {
            for name in query::app_pair_names(primary.as_ref(), secondary.as_ref()) {
                *apps.entry(name).or_default() += count;
            }
        }
    }
    if scope.packages {
        for (primary, secondary, count) in package_pairs {
            for name in query::package_pair_names(primary.as_ref(), secondary.as_ref()) {
                *packages.entry(name).or_default() += count;
            }
        }
    }
    if scope.apps && scope.packages {
        for (name, count) in rails::packages_per_app_category(package_pairs) {
            *apps.entry(name.to_owned()).or_default() += count;
        }
    }
    for filter in selected {
        match filter {
            CategoryFilter::App(name) if scope.apps => {
                apps.entry(name.clone()).or_default();
            }
            CategoryFilter::Package(category) if scope.packages => {
                packages.entry(category.to_string()).or_default();
            }
            CategoryFilter::App(_) | CategoryFilter::Package(_) => {}
        }
    }
    let selected: HashSet<String> = selected.iter().map(CategoryFilter::value).collect();
    let rows = |counts: BTreeMap<String, u64>, prefix: &str, kind: FacetKind| {
        let mut rows: Vec<FacetCount> = counts
            .into_iter()
            .map(|(name, count)| FacetCount {
                value: format!("{prefix}{name}"),
                kind,
                count,
            })
            .filter(|row| row.count > 0 || selected.contains(&row.value))
            .collect();
        rows.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
        rows
    };
    let mut facets = rows(apps, "app:", FacetKind::App);
    facets.extend(rows(packages, "package:", FacetKind::Package));
    facets
}

fn permission_facet(window: &[PackageSummary]) -> PermissionFacet {
    let count = |group: PermissionGroup| {
        count_of(
            window
                .iter()
                .filter(|package| group.matches(&package.capabilities))
                .count(),
        )
    };
    PermissionFacet {
        none: count(PermissionGroup::NoPermissions),
        network: count(PermissionGroup::Network),
        models: count(PermissionGroup::Models),
        storage: count(PermissionGroup::Storage),
    }
}

/// The LIVE collection `?collection=` opens: any slot (unplaced included), but only when enabled, inside its
/// window and matching the viewer's audience.
fn collection_target<'a>(
    layout: &'a LayoutDoc,
    id: &str,
    viewer: &Viewer,
    now: DateTime<Utc>,
) -> Result<&'a PlacementDoc, ApiError> {
    layout
        .find(id)
        .map(|(_, placement)| placement)
        .filter(|placement| resolve::collection_visible(placement, viewer, now))
        .ok_or_else(|| ApiError::not_found(format!("Explore collection {id} is not available")))
}

fn collection_mentions(placement: &PlacementDoc, needle: &str) -> bool {
    match &placement.content {
        PlacementContent::Collection { title, blurb, .. } => {
            title.to_lowercase().contains(needle)
                || blurb
                    .as_deref()
                    .is_some_and(|blurb| blurb.to_lowercase().contains(needle))
        }
        _ => false,
    }
}

/// Visible collections on the page (grid or rows) whose title or blurb contains `text`; all of them without text.
fn matching_collections<'a>(
    layout: &'a LayoutDoc,
    viewer: &Viewer,
    now: DateTime<Utc>,
    text: Option<&str>,
) -> Vec<&'a PlacementDoc> {
    let needle = text.map(str::to_lowercase);
    layout
        .slots
        .iter()
        .filter(|slot| slot.area != SlotArea::Unplaced)
        .flat_map(|slot| slot.placements.iter())
        .filter(|placement| resolve::collection_visible(placement, viewer, now))
        .filter(|placement| {
            needle
                .as_deref()
                .is_none_or(|needle| collection_mentions(placement, needle))
        })
        .collect()
}

fn package_mentions(package: &PackageSummary, text: &str) -> bool {
    let needle = text.to_lowercase();
    [&package.name, &package.description, &package.id]
        .into_iter()
        .any(|value| value.to_lowercase().contains(&needle))
}

/// Package id → title of the first collection listing it, hand-picked items before rule results.
fn packages_of_collections(collections: &[&PlacementDoc], rules: &RuleItems) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for placement in collections {
        let PlacementContent::Collection { title, .. } = &placement.content else {
            continue;
        };
        let picked = placement.items.iter().map(|item| (item.kind, &item.id));
        let ruled = rules
            .get(&placement.id)
            .into_iter()
            .flatten()
            .map(|(kind, id)| (*kind, id));
        for (kind, id) in picked.chain(ruled) {
            if kind == ItemKind::Package && seen.insert(id.clone()) {
                out.push((id.clone(), title.clone()));
            }
        }
    }
    out
}

fn package_hit(
    package: PackageSummary,
    text: Option<&str>,
    via_collection: &HashMap<String, String>,
) -> SearchPackageHit {
    let collection_title = text
        .filter(|text| !package_mentions(&package, text))
        .and_then(|_| via_collection.get(&package.id).cloned());
    SearchPackageHit {
        matched_via: if collection_title.is_some() {
            MatchedVia::Collection
        } else {
            MatchedVia::Name
        },
        collection_title,
        package,
    }
}

fn app_items(ids: &[String], apps: &AppMap) -> Vec<ResolvedItem> {
    hydrate::in_id_order(ids, apps)
        .into_iter()
        .map(|(app, metadata)| ResolvedItem::App { app, metadata })
        .collect()
}

#[derive(Default)]
struct AppsPart {
    page: Vec<String>,
    total: u64,
    pairs: Vec<AppCategoryPair>,
    price: (u64, u64),
}

async fn apps_part(
    state: &AppState,
    request: &SearchRequest,
    viewer: &Viewer,
    facets: FacetScope,
) -> Result<AppsPart, ApiError> {
    let db = &state.db;
    let filter = request.app_filter(&viewer.language);
    let without_categories = AppFilter {
        categories: None,
        ..filter.clone()
    };
    let without_price = AppFilter {
        price: None,
        ..filter.clone()
    };
    let (page, total, pairs, price) = futures::try_join!(
        async {
            if request.scope.lists_apps() {
                let select = query::app_ids(&filter, request.sort);
                query::ids(db, select, request.apps.offset, request.apps.limit).await
            } else {
                Ok(Vec::new())
            }
        },
        query::count_apps(db, &filter),
        async {
            if facets.apps {
                query::app_category_pairs(db, &without_categories).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if facets.apps {
                query::app_price_counts(db, &without_price).await
            } else {
                Ok((0, 0))
            }
        },
    )?;
    Ok(AppsPart {
        page,
        total,
        pairs,
        price,
    })
}

#[derive(Default)]
struct PackagesPart {
    page: Vec<PackageSummary>,
    total: u64,
    capped: bool,
    pairs: Vec<PackageCategoryPair>,
    price: (u64, u64),
    verified: u64,
    permissions: PermissionFacet,
}

/// Packages for developer viewers, straight over `WasmPackage` under `public_package_condition()`. Without a
/// permission filter the page, the total and the facets come from the database. Permissions exist only in Rust,
/// so with that filter the first 200 matches are hydrated, filtered and paged here, and `capped` says so.
async fn packages_part(
    state: &AppState,
    request: &SearchRequest,
    viewer: &Viewer,
    facets: FacetScope,
    collection_ids: Vec<String>,
) -> Result<PackagesPart, ApiError> {
    if !viewer.dev {
        return Ok(PackagesPart::default());
    }
    let db = &state.db;
    let filter = request.package_filter(collection_ids);
    let filtered = !request.permissions.is_empty();
    let listed = request.scope.lists_packages();
    let windowed = listed || facets.packages || filtered;
    let without_categories = PackageFilter {
        categories: None,
        ..filter.clone()
    };
    let without_price = PackageFilter {
        price: None,
        ..filter.clone()
    };
    let without_verified = PackageFilter {
        verified_only: false,
        ..filter.clone()
    };
    let (window_ids, count, pairs, price, verified) = futures::try_join!(
        async {
            if windowed {
                let select = query::package_ids(&filter, request.sort);
                query::ids(db, select, 0, PACKAGE_WINDOW).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if filtered {
                Ok(0)
            } else {
                query::count_packages(db, &filter).await
            }
        },
        async {
            if facets.packages {
                query::package_category_pairs(db, &without_categories).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if facets.packages {
                query::package_price_counts(db, &without_price).await
            } else {
                Ok((0, 0))
            }
        },
        async {
            if facets.packages {
                Ok(query::package_verified_counts(db, &without_verified).await?.1)
            } else {
                Ok(0)
            }
        },
    )?;
    let found = hydrate::packages(state, &window_ids, &viewer.language).await?;
    let window = hydrate::in_id_order(&window_ids, &found);
    let permissions = permission_facet(&window);
    let window_full = count_of(window_ids.len()) >= PACKAGE_WINDOW;
    let (total, page) = if filtered {
        let matching: Vec<PackageSummary> = window
            .into_iter()
            .filter(|package| request.matches_permissions(package))
            .collect();
        (count_of(matching.len()), request.packages.slice(matching))
    } else if !listed {
        (count, Vec::new())
    } else if !window_full || request.packages.end() <= count_of(window.len()) {
        (count, request.packages.slice(window))
    } else {
        let select = query::package_ids(&filter, request.sort);
        let page_ids = query::ids(db, select, request.packages.offset, request.packages.limit).await?;
        let found = hydrate::packages(state, &page_ids, &viewer.language).await?;
        (count, hydrate::in_id_order(&page_ids, &found))
    };
    Ok(PackagesPart {
        page: if listed { page } else { Vec::new() },
        total,
        capped: filtered && window_full,
        pairs,
        price,
        verified,
        permissions,
    })
}

/// App categories of the matched items: the apps' own and those the packages' categories map to.
fn related_categories(apps: &[ResolvedItem], packages: &[PackageSummary]) -> Vec<Category> {
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: String| {
        if !names.contains(&name) {
            names.push(name);
        }
    };
    for item in apps {
        if let ResolvedItem::App { app, .. } = item {
            for category in app.primary_category.iter().chain(&app.secondary_category) {
                add(query::app_category_label(&Category::from(category.clone())));
            }
        }
    }
    for package in packages {
        for raw in package.primary_category.iter().chain(&package.secondary_category) {
            if let Some(category) = WasmPackageCategory::from_str_opt(raw) {
                for name in app_categories_for(&category) {
                    add((*name).to_owned());
                }
            }
        }
    }
    names
        .iter()
        .filter_map(|name| parse_app_category(name))
        .map(Category::from)
        .collect()
}

/// "You might also like": the most installed apps in the matched items' categories (or overall when nothing
/// matched), excluding the matches.
async fn related(
    state: &AppState,
    viewer: &Viewer,
    apps: &[ResolvedItem],
    packages: &[PackageSummary],
) -> Result<Vec<ResolvedItem>, ApiError> {
    let categories = related_categories(apps, packages);
    let filter = AppFilter {
        categories: (!categories.is_empty()).then_some(categories),
        exclude: apps.iter().map(|item| item.id().to_owned()).collect(),
        ..AppFilter::new(&viewer.language)
    };
    let ids = query::ids(
        &state.db,
        query::app_ids(&filter, ExploreSort::Installs),
        0,
        RELATED_MAX,
    )
    .await?;
    let store = if ids.is_empty() {
        None
    } else {
        hydrate::media_store(state).await
    };
    let found = hydrate::apps(&state.db, &ids, &viewer.language, store.as_ref()).await?;
    Ok(app_items(&ids, &found))
}

fn resolved_collections(
    placements: &[&PlacementDoc],
    hydrated: &Hydrated,
    viewer: &Viewer,
    projection: Projection,
) -> Vec<ResolvedCollection> {
    placements
        .iter()
        .filter_map(|placement| resolve::resolve_collection(placement, hydrated, viewer, projection))
        .filter(|collection| !collection.items.is_empty())
        .collect()
}

async fn search(
    state: &AppState,
    viewer: &Viewer,
    request: &SearchRequest,
    now: DateTime<Utc>,
) -> Result<ExploreSearchResponse, ApiError> {
    let live = timed("load", load_edition(state, Edition::Live)).await?;
    if let Some(id) = &request.collection {
        let placement = collection_target(&live.layout, id, viewer, now)?;
        return open_collection(state, placement, viewer, request).await;
    }
    let facets = FacetScope::of(request.scope, viewer.dev);
    let matched = matching_collections(&live.layout, viewer, now, request.text.as_deref());
    let featured: Vec<&PlacementDoc> = matched.iter().take(COLLECTION_HITS_MAX).copied().collect();

    let (rules, apps) = futures::try_join!(
        timed("rules", hydrate::rule_items(&state.db, &featured, viewer, 1)),
        timed("apps", apps_part(state, request, viewer, facets)),
    )?;
    let via_collection = if request.text.is_some() && viewer.dev {
        packages_of_collections(&featured, &rules)
    } else {
        Vec::new()
    };
    let collection_ids = via_collection.iter().map(|(id, _)| id.clone()).collect();
    let mut packages = timed(
        "packages",
        packages_part(state, request, viewer, facets, collection_ids),
    )
    .await?;

    let mut wanted = Wanted::default();
    wanted.add_all(apps.page.iter().map(|id| (ItemKind::App, id.as_str())));
    if request.scope.lists_collections() {
        let page_packages: HashSet<&str> = packages.page.iter().map(|package| package.id.as_str()).collect();
        for placement in &featured {
            let picked = placement.items.iter().map(|item| (item.kind, item.id.as_str()));
            let ruled = rules
                .get(&placement.id)
                .into_iter()
                .flatten()
                .map(|(kind, id)| (*kind, id.as_str()));
            wanted.add_all(
                picked
                    .chain(ruled)
                    .filter(|(kind, id)| *kind != ItemKind::Package || !page_packages.contains(id)),
            );
        }
    }
    let store = if wanted.is_empty() && packages.page.is_empty() {
        None
    } else {
        hydrate::media_store(state).await
    };
    let (app_map, mut package_map) = timed(
        "hydrate",
        hydrate::hydrate_items(state, &wanted, viewer, store.as_ref()),
    )
    .await?;
    hydrate::presign_packages(store.as_ref(), packages.page.iter_mut()).await;

    let app_page = app_items(&apps.page, &app_map);
    let apps_group = if request.scope.lists_apps() {
        request.apps.group(app_page.clone(), apps.total)
    } else {
        empty_group()
    };
    let packages_group = if request.scope.lists_packages() && viewer.dev {
        request.packages.group(packages.page.clone(), packages.total)
    } else {
        empty_group()
    };
    let related = if request.text.is_some()
        && request.scope != SearchScope::Collections
        && apps_group.total + packages_group.total < RELATED_MAX
    {
        timed("related", related(state, viewer, &app_page, &packages.page)).await?
    } else {
        Vec::new()
    };

    for package in &packages.page {
        package_map.insert(package.id.clone(), package.clone());
    }
    let hydrated = Hydrated {
        apps: app_map,
        packages: package_map,
        rule_items: rules,
        rails: HashMap::new(),
    };
    let collections = if request.scope.lists_collections() {
        resolved_collections(&featured, &hydrated, viewer, request.scope.projection())
    } else {
        Vec::new()
    };
    let via_collection: HashMap<String, String> = via_collection.into_iter().collect();
    let text = request.text.as_deref();
    let packages_group = SearchGroup {
        items: packages_group
            .items
            .into_iter()
            .map(|package| package_hit(package, text, &via_collection))
            .collect(),
        total: packages_group.total,
        has_more: packages_group.has_more,
    };
    let price = |(app_free, app_paid): (u64, u64), (package_free, package_paid): (u64, u64)| PriceFacet {
        free: app_free + package_free,
        paid: app_paid + package_paid,
    };
    Ok(ExploreSearchResponse {
        query: request.text.clone().unwrap_or_default(),
        viewer: ViewerEcho::from(viewer),
        collections,
        apps: apps_group,
        packages: packages_group,
        related,
        facets: ExploreFacets {
            types: TypeFacet {
                apps: apps.total,
                packages: packages.total,
                collections: count_of(matched.len()),
            },
            categories: category_facets(&apps.pairs, &packages.pairs, facets, &request.categories),
            price: price(apps.price, packages.price),
            verified: packages.verified,
            permissions: packages.permissions,
            packages_capped: packages.capped,
        },
    })
}

/// `?collection=<id>`: the collection alone with up to 48 items; its apps and packages fill the two groups.
async fn open_collection(
    state: &AppState,
    placement: &PlacementDoc,
    viewer: &Viewer,
    request: &SearchRequest,
) -> Result<ExploreSearchResponse, ApiError> {
    let rules = hydrate::rule_items(&state.db, &[placement], viewer, COLLECTION_PAGE_SCALE).await?;
    let mut wanted = Wanted::default();
    wanted.add_all(placement.items.iter().map(|item| (item.kind, item.id.as_str())));
    for items in rules.values() {
        wanted.add_all(items.iter().map(|(kind, id)| (*kind, id.as_str())));
    }
    let store = if wanted.is_empty() {
        None
    } else {
        hydrate::media_store(state).await
    };
    let (apps, packages): (AppMap, PackageMap) =
        hydrate::hydrate_items(state, &wanted, viewer, store.as_ref()).await?;
    let hydrated = Hydrated {
        apps,
        packages,
        rule_items: rules,
        rails: HashMap::new(),
    };
    let mut collection =
        resolve::resolve_collection(placement, &hydrated, viewer, request.scope.projection())
            .ok_or_else(|| {
                ApiError::not_found(format!("Explore collection {} is not available", placement.id))
            })?;
    collection.items.truncate(COLLECTION_PAGE_ITEMS);
    let app_items: Vec<ResolvedItem> = collection
        .items
        .iter()
        .filter(|item| item.kind() == ItemKind::App)
        .cloned()
        .collect();
    let package_hits: Vec<SearchPackageHit> = collection
        .items
        .iter()
        .filter_map(|item| match item {
            ResolvedItem::Package { package } => Some(SearchPackageHit {
                package: package.clone(),
                matched_via: MatchedVia::Collection,
                collection_title: Some(collection.title.clone()),
            }),
            _ => None,
        })
        .collect();
    let (app_total, package_total) = (count_of(app_items.len()), count_of(package_hits.len()));
    Ok(ExploreSearchResponse {
        query: request.text.clone().unwrap_or_default(),
        viewer: ViewerEcho::from(viewer),
        apps: request
            .apps
            .group(request.apps.slice(app_items), app_total),
        packages: request
            .packages
            .group(request.packages.slice(package_hits), package_total),
        collections: vec![collection],
        related: Vec::new(),
        facets: ExploreFacets {
            types: TypeFacet {
                apps: app_total,
                packages: package_total,
                collections: 1,
            },
            ..ExploreFacets::default()
        },
    })
}

#[utoipa::path(
    get,
    path = "/store/explore/search",
    tag = "store",
    description = "Search and browse apps, and in developer mode packages, together with matching curated collections and facet counts. With `collection`, opens that curated collection instead.",
    params(ExploreSearchQuery),
    responses(
        (status = 200, description = "Search results with facets", body = ExploreSearchResponse),
        (status = 400, description = "A parameter has an unknown value, too many entries or an overlong query"),
        (status = 401, description = "This hub requires signing in to browse"),
        (status = 404, description = "The requested collection is not available")
    )
)]
#[tracing::instrument(name = "GET /store/explore/search", skip_all)]
pub async fn search_explore(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ExploreSearchQuery>,
) -> Result<Json<ExploreSearchResponse>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }
    let viewer = viewer(
        &state,
        user.sub().is_ok(),
        query.platform.as_deref(),
        query.language.as_deref(),
        query.dev.as_deref(),
    )?;
    let request = SearchRequest::parse(&query)?;
    Ok(Json(search(&state, &viewer, &request, Utc::now()).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::explore::defaults::default_layout;
    use crate::routes::explore::model::{
        CollectionSource, ItemOverrides, PlacementItemDoc, SLOT_COLLECTION, SLOT_UNPLACED,
    };
    use chrono::Duration;
    use serde_json::json;

    fn message(error: ApiError) -> String {
        error.public_message().unwrap_or_default().to_owned()
    }

    fn viewer(dev: bool) -> Viewer {
        Viewer {
            dev,
            signed_in: false,
            platform: Platform::Web,
            language: "en".into(),
        }
    }

    fn params(uri: &str) -> ExploreSearchQuery {
        Query::<ExploreSearchQuery>::try_from_uri(&uri.parse().unwrap())
            .unwrap()
            .0
    }

    #[test]
    fn search_params_parse_from_comma_lists() {
        let request = SearchRequest::parse(&params(
            "/store/explore/search?q=%20invoice%20&type=apps&categories=app:Finance&permissions=network,models&sort=name&apps_limit=48&verified=true",
        ))
        .unwrap();
        assert_eq!(request.text.as_deref(), Some("invoice"));
        assert_eq!(request.scope, SearchScope::Apps);
        assert_eq!(request.categories, [CategoryFilter::App("Finance".into())]);
        assert_eq!(request.permissions, [PermissionGroup::Network, PermissionGroup::Models]);
        assert_eq!(request.sort, ExploreSort::Name);
        assert!(request.verified);
        assert_eq!(request.apps, Paging { offset: 0, limit: 48 });
        assert_eq!(request.packages, Paging { offset: 0, limit: PAGE_DEFAULT });

        let request = SearchRequest::parse(&params(
            "/store/explore/search?categories=app:Finance,package:EDUCATION",
        ))
        .unwrap();
        assert_eq!(request.categories.len(), 2);
        assert_eq!(request.app_categories(), Some(vec![Category::Finance]));
        assert_eq!(
            request.package_categories(),
            Some(vec![
                DbPackageCategory::FinanceBilling,
                DbPackageCategory::Insurance,
                DbPackageCategory::Education
            ])
        );
        let request = SearchRequest::parse(&params("/?categories=package:EDUCATION")).unwrap();
        assert_eq!(request.app_categories(), Some(Vec::new()));
        assert_eq!(SearchRequest::parse(&params("/")).unwrap().app_categories(), None);
    }

    #[test]
    fn bad_search_params_name_the_parameter_and_value() {
        for (uri, expected) in [
            ("/?type=widgets", "type value 'widgets'"),
            ("/?price=cheap", "price value 'cheap'"),
            ("/?sort=relevance", "sort value 'relevance'"),
            ("/?verified=maybe", "verified value 'maybe'"),
            ("/?apps_limit=49", "apps_limit must be at most 48"),
            ("/?packages_limit=100", "packages_limit must be at most 48"),
            ("/?permissions=foo", "'foo'"),
            ("/?categories=package:Education", "'package:Education'"),
        ] {
            let error = SearchRequest::parse(&params(uri)).unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
            assert!(message(error).contains(expected), "{uri}");
        }
        let long = format!("/?q={}", "a".repeat(QUERY_MAX + 1));
        assert!(message(SearchRequest::parse(&params(&long)).unwrap_err()).contains("q must be at most 100"));
        let exact = format!("/?q={}", "a".repeat(QUERY_MAX));
        assert!(SearchRequest::parse(&params(&exact)).is_ok());
    }

    #[test]
    fn viewer_flags_and_platforms_parse() {
        assert!(!parse_flag("dev", None).unwrap());
        assert!(parse_flag("dev", Some("true")).unwrap());
        assert!(message(parse_flag("dev", Some("yes")).unwrap_err()).contains("dev value 'yes'"));
        assert_eq!(parse_platform(None).unwrap(), Platform::Web);
        assert_eq!(parse_platform(Some("desktop")).unwrap(), Platform::Desktop);
        assert!(parse_platform(Some("ios")).is_err());
    }

    #[test]
    fn cache_keys_cover_every_viewer_class() {
        let key = page_cache_key("rev1", &Viewer {
            dev: true,
            signed_in: false,
            platform: Platform::Desktop,
            language: "pt-BR".into(),
        });
        assert_eq!(key, "explore:v1:rev1:true:false:desktop:pt-BR");
        let keys: HashSet<String> = viewer_classes().map(|viewer| page_cache_key("r", &viewer)).collect();
        assert_eq!(keys.len(), 2 * 2 * 2 * SUPPORTED_LOCALES.len());
    }

    const BOTH: FacetScope = FacetScope {
        apps: true,
        packages: true,
    };

    #[test]
    fn category_facets_fold_primary_and_secondary_pairs() {
        let app_pairs = vec![
            (Some(Category::Business), Some(Category::Finance), 1),
            (Some(Category::Finance), Some(Category::Finance), 2),
            (None, None, 5),
        ];
        let package_pairs = vec![
            (
                Some(DbPackageCategory::FinanceBilling),
                Some(DbPackageCategory::Insurance),
                1,
            ),
            (Some(DbPackageCategory::Legal), None, 4),
        ];
        let facets = category_facets(&app_pairs, &package_pairs, BOTH, &[]);
        assert_eq!(
            facets,
            [
                FacetCount { value: "app:Finance".into(), kind: FacetKind::App, count: 4 },
                FacetCount { value: "app:Business".into(), kind: FacetKind::App, count: 1 },
                FacetCount { value: "package:LEGAL".into(), kind: FacetKind::Package, count: 4 },
                FacetCount { value: "package:FINANCE_BILLING".into(), kind: FacetKind::Package, count: 1 },
                FacetCount { value: "package:INSURANCE".into(), kind: FacetKind::Package, count: 1 },
            ]
        );
        for facet in &facets {
            let parsed = parse_categories(Some(&facet.value)).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].value(), facet.value);
        }
    }

    #[test]
    fn a_secondary_only_category_is_counted() {
        let facets = category_facets(
            &[(Some(Category::Business), Some(Category::Finance), 1)],
            &[],
            FacetScope { apps: true, packages: false },
            &[],
        );
        assert!(facets.contains(&FacetCount {
            value: "app:Finance".into(),
            kind: FacetKind::App,
            count: 1
        }));
    }

    #[test]
    fn facet_rows_follow_the_type_and_keep_selected_values() {
        let app_pairs = vec![(Some(Category::Finance), None, 3)];
        let package_pairs = vec![(Some(DbPackageCategory::Education), None, 2)];
        let selected = vec![
            CategoryFilter::App("Games".into()),
            CategoryFilter::Package(WasmPackageCategory::Legal),
        ];
        let apps_only = category_facets(&app_pairs, &package_pairs, FacetScope::of(SearchScope::Apps, true), &selected);
        assert!(apps_only.iter().all(|facet| facet.kind == FacetKind::App));
        assert!(apps_only.contains(&FacetCount { value: "app:Games".into(), kind: FacetKind::App, count: 0 }));
        assert!(!apps_only.iter().any(|facet| facet.value == "app:Education"));
        let packages_only =
            category_facets(&app_pairs, &package_pairs, FacetScope::of(SearchScope::Packages, true), &selected);
        assert_eq!(
            packages_only,
            [
                FacetCount { value: "package:EDUCATION".into(), kind: FacetKind::Package, count: 2 },
                FacetCount { value: "package:LEGAL".into(), kind: FacetKind::Package, count: 0 },
            ]
        );
        let non_dev = category_facets(&app_pairs, &package_pairs, FacetScope::of(SearchScope::All, false), &[]);
        assert_eq!(non_dev, [FacetCount { value: "app:Finance".into(), kind: FacetKind::App, count: 3 }]);
        let all = category_facets(&app_pairs, &package_pairs, FacetScope::of(SearchScope::All, true), &[]);
        assert!(all.contains(&FacetCount { value: "app:Education".into(), kind: FacetKind::App, count: 2 }));
    }

    fn placement_json(id: &str, content: serde_json::Value, extra: serde_json::Value) -> PlacementDoc {
        let mut doc = json!({
            "id": id,
            "kind": content["kind"],
            "name": id,
            "enabled": true,
            "startsAt": null,
            "endsAt": null,
            "audience": [],
            "content": content,
            "items": [],
        });
        if let (Some(doc), Some(extra)) = (doc.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                doc.insert(key.clone(), value.clone());
            }
        }
        serde_json::from_value(doc).unwrap()
    }

    fn collection(id: &str, title: &str, extra: serde_json::Value) -> PlacementDoc {
        placement_json(
            id,
            json!({"kind": "collection", "title": title, "blurb": "Tools for invoices", "source": "hand", "rule": null}),
            extra,
        )
    }

    fn layout_with(slot: &str, placements: Vec<PlacementDoc>) -> LayoutDoc {
        let mut layout = default_layout();
        layout
            .slots
            .iter_mut()
            .find(|candidate| candidate.key == slot)
            .unwrap()
            .placements
            .extend(placements);
        layout
    }

    #[test]
    fn opening_a_collection_requires_a_visible_collection() {
        let now = Utc::now();
        let later = (now + Duration::days(1)).to_rfc3339();
        let layout = layout_with(
            SLOT_UNPLACED,
            vec![
                collection("live", "Invoicing", json!({})),
                collection("disabled", "Off", json!({"enabled": false})),
                collection("scheduled", "Soon", json!({"startsAt": later})),
                collection("devs", "Dev picks", json!({"audience": ["dev"]})),
            ],
        );
        let viewer = viewer(false);
        assert_eq!(collection_target(&layout, "live", &viewer, now).unwrap().id, "live");
        for id in ["disabled", "scheduled", "devs", "default-trending", "missing"] {
            let error = collection_target(&layout, id, &viewer, now).unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::NOT_FOUND, "{id}");
            assert_eq!(message(error), format!("Explore collection {id} is not available"));
        }
    }

    #[test]
    fn collection_hits_come_from_the_page_and_match_title_or_blurb() {
        let now = Utc::now();
        let mut layout = layout_with(SLOT_COLLECTION, vec![collection("grid", "Finance picks", json!({}))]);
        layout
            .slots
            .iter_mut()
            .find(|slot| slot.key == SLOT_UNPLACED)
            .unwrap()
            .placements
            .push(collection("parked", "Finance extras", json!({})));
        let viewer = viewer(false);
        let ids = |text: Option<&str>| -> Vec<String> {
            matching_collections(&layout, &viewer, now, text)
                .into_iter()
                .map(|placement| placement.id.clone())
                .collect()
        };
        assert_eq!(ids(Some("FINANCE")), ["grid"]);
        assert_eq!(ids(Some("invoices")), ["grid"]);
        assert!(ids(Some("travel")).is_empty());
        assert_eq!(ids(None), ["grid"]);
    }

    fn package(id: &str, name: &str) -> PackageSummary {
        serde_json::from_value(json!({
            "id": id,
            "name": name,
            "description": "",
            "latestVersion": "1.0.0",
            "downloadCount": 0,
            "status": "active",
            "keywords": [],
            "verified": false,
            "capabilities": ["net.http"]
        }))
        .unwrap()
    }

    #[test]
    fn packages_match_by_name_or_through_a_collection() {
        let mut picks = collection("picks", "Invoicing", json!({}));
        picks.items = [(ItemKind::App, "a"), (ItemKind::Package, "pdf"), (ItemKind::Package, "ocr")]
            .map(|(kind, id)| PlacementItemDoc::from_row(kind, id.into(), ItemOverrides::default()))
            .to_vec();
        let mut ruled = collection("ruled", "More", json!({}));
        if let PlacementContent::Collection { source, .. } = &mut ruled.content {
            *source = CollectionSource::Rule;
        }
        let rules: RuleItems = HashMap::from([(
            "ruled".to_owned(),
            vec![(ItemKind::Package, "ocr".to_owned()), (ItemKind::Package, "csv".to_owned())],
        )]);
        let via = packages_of_collections(&[&picks, &ruled], &rules);
        assert_eq!(
            via,
            [
                ("pdf".to_owned(), "Invoicing".to_owned()),
                ("ocr".to_owned(), "Invoicing".to_owned()),
                ("csv".to_owned(), "More".to_owned()),
            ]
        );
        let via: HashMap<String, String> = via.into_iter().collect();
        let by_name = package_hit(package("invoice-kit", "Kit"), Some("Invoice"), &via);
        assert_eq!(by_name.matched_via, MatchedVia::Name);
        assert_eq!(by_name.collection_title, None);
        let by_collection = package_hit(package("pdf", "PDF tools"), Some("invoice"), &via);
        assert_eq!(by_collection.matched_via, MatchedVia::Collection);
        assert_eq!(by_collection.collection_title.as_deref(), Some("Invoicing"));
        assert_eq!(package_hit(package("pdf", "PDF"), None, &via).matched_via, MatchedVia::Name);
    }

    #[test]
    fn paging_slices_and_reports_more() {
        let paging = Paging { offset: 2, limit: 2 };
        assert_eq!(paging.slice(vec![1, 2, 3, 4, 5]), [3, 4]);
        let group = paging.group(vec![3, 4], 5);
        assert!(group.has_more);
        assert!(!paging.group(vec![3, 4], 4).has_more);
        let facet = permission_facet(&[package("a", "A"), package("b", "B")]);
        assert_eq!(facet, PermissionFacet { none: 0, network: 2, models: 0, storage: 0 });
    }

    #[test]
    fn related_categories_combine_apps_and_mapped_packages() {
        let mut pkg = package("p", "P");
        pkg.primary_category = Some("INSURANCE".into());
        pkg.secondary_category = Some("MEDIA_CONTENT".into());
        let categories = related_categories(&[], &[pkg]);
        assert_eq!(
            categories,
            [
                Category::Finance,
                Category::News,
                Category::Photography,
                Category::Music,
                Category::Entertainment
            ]
        );
    }
}
