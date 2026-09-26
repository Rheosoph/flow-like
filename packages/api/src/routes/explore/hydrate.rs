use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use flow_like::app::App;
use flow_like::bit::Metadata;
use flow_like_storage::Path as FlowPath;
use flow_like_storage::files::store::FlowLikeStore;
use futures::future::{join_all, try_join_all};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use super::audience::DEFAULT_LANGUAGE;
use super::model::{
    CollectionSource, ItemKind, ItemRef, LayoutDoc, PlacementContent, PlacementDoc, Viewer,
    normalize_package_category,
};
use super::query;
use super::rails;
use super::resolve::{Candidates, Hydrated};
use crate::entity::sea_orm_active_enums::{
    Status, Visibility, WasmPackageStatus, WasmPackageVisibility,
};
use crate::entity::{app, meta, wasm_package};
use crate::error::ApiError;
use crate::routes::registry::types::{MetaSummary, PackageSummary, SearchFilters};
use crate::state::AppState;

pub type AppMap = HashMap<String, (App, Option<Metadata>)>;
pub type PackageMap = HashMap<String, PackageSummary>;
pub type RuleItems = HashMap<String, Vec<(ItemKind, String)>>;

/// App and package ids to hydrate, each once, in first-seen order.
#[derive(Debug, Default)]
pub struct Wanted {
    pub apps: Vec<String>,
    pub packages: Vec<String>,
    seen: HashSet<(ItemKind, String)>,
}

impl Wanted {
    pub fn add(&mut self, kind: ItemKind, id: &str) {
        if kind == ItemKind::Collection || !self.seen.insert((kind, id.to_owned())) {
            return;
        }
        match kind {
            ItemKind::App => self.apps.push(id.to_owned()),
            ItemKind::Package => self.packages.push(id.to_owned()),
            ItemKind::Collection => {}
        }
    }

    pub fn add_all<'a>(&mut self, items: impl IntoIterator<Item = (ItemKind, &'a str)>) {
        for (kind, id) in items {
            self.add(kind, id);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty() && self.packages.is_empty()
    }
}

/// The store that signs app and package media, or `None` (unsigned media) when credentials are unavailable.
pub async fn media_store(state: &AppState) -> Option<FlowLikeStore> {
    let store = async { state.master_credentials().await?.to_store(false).await };
    store
        .await
        .inspect_err(|error| tracing::warn!(%error, "Explore media stays unsigned: no media store"))
        .ok()
}

fn app_media(app_id: &str) -> FlowPath {
    FlowPath::from("media").join("apps").join(app_id)
}

/// Per owner, the `Meta` row in `language`, else English, else the first by language code.
fn best_meta<F>(metas: Vec<meta::Model>, language: &str, owner: F) -> HashMap<String, meta::Model>
where
    F: Fn(&meta::Model) -> Option<&String>,
{
    let rank = |lang: &str| {
        if lang == language {
            0
        } else if lang == DEFAULT_LANGUAGE {
            1
        } else {
            2
        }
    };
    let mut best: HashMap<String, meta::Model> = HashMap::new();
    for meta in metas {
        let Some(owner) = owner(&meta).cloned() else {
            continue;
        };
        let better = best
            .get(&owner)
            .is_none_or(|current| rank(&meta.lang) < rank(&current.lang));
        if better {
            best.insert(owner, meta);
        }
    }
    best
}

/// Public, active apps among `ids` with their best metadata and signed media: one app query, one meta query
/// and concurrent presigns.
pub async fn apps<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
    language: &str,
    store: Option<&FlowLikeStore>,
) -> Result<AppMap, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let models = app::Entity::find()
        .filter(app::Column::Id.is_in(ids.to_vec()))
        .filter(query::public_app_condition())
        .all(db)
        .await?;
    if models.is_empty() {
        return Ok(HashMap::new());
    }
    let metas = meta::Entity::find()
        .filter(meta::Column::AppId.is_in(models.iter().map(|model| model.id.clone())))
        .order_by_asc(meta::Column::Lang)
        .all(db)
        .await?;
    let mut best = best_meta(metas, language, |meta| meta.app_id.as_ref());
    let tasks = models.into_iter().map(|model| {
        let meta = best.remove(&model.id);
        async move {
            let metadata = match meta {
                Some(meta) => {
                    let mut metadata = Metadata::from(meta);
                    if let Some(store) = store {
                        metadata.presign(app_media(&model.id), store).await;
                    }
                    Some(metadata)
                }
                None => None,
            };
            (model.id.clone(), (App::from(model), metadata))
        }
    });
    Ok(join_all(tasks).await.into_iter().collect())
}

/// Registry categories come back in the entity's PascalCase; Explore always speaks SCREAMING_SNAKE.
pub fn normalize_categories(package: &mut PackageSummary) {
    package.primary_category = package
        .primary_category
        .as_deref()
        .and_then(normalize_package_category);
    package.secondary_category = package
        .secondary_category
        .as_deref()
        .and_then(normalize_package_category);
}

/// Registry results keyed by id with normalized categories. The registry returns its own order, so callers
/// emit through [`in_id_order`].
pub fn index_packages(found: Vec<PackageSummary>) -> PackageMap {
    found
        .into_iter()
        .map(|mut package| {
            normalize_categories(&mut package);
            (package.id.clone(), package)
        })
        .collect()
}

/// The hydrated values of `ids` in exactly that order; ids without a value are skipped.
pub fn in_id_order<T: Clone>(ids: &[String], found: &HashMap<String, T>) -> Vec<T> {
    ids.iter().filter_map(|id| found.get(id).cloned()).collect()
}

/// Public, active packages among `ids` through the registry. `caller_id` stays `None`, so curation can never
/// surface a private package. Media stays unsigned; see [`presign_packages`].
pub async fn packages(
    state: &AppState,
    ids: &[String],
    language: &str,
) -> Result<PackageMap, ApiError> {
    let Some(registry) = state.wasm_registry.as_ref() else {
        return Ok(HashMap::new());
    };
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    // One past the id count, so the registry never needs its COUNT query.
    let filters = SearchFilters {
        limit: ids.len() + 1,
        sort_desc: true,
        language: Some(language.to_owned()),
        ..SearchFilters::default()
    };
    let found = registry
        .search_with_visibility(&filters, None, false, false, None, Some(ids))
        .await?;
    Ok(index_packages(found.packages))
}

pub async fn presign_packages<'a>(
    store: Option<&FlowLikeStore>,
    packages: impl IntoIterator<Item = &'a mut PackageSummary>,
) {
    let Some(store) = store else {
        return;
    };
    join_all(packages.into_iter().filter_map(|package| {
        let id = package.id.clone();
        package
            .metadata
            .as_mut()
            .map(|metadata| async move { metadata.presign_media(&id, store).await })
    }))
    .await;
}

/// Apps and, for developer viewers, packages behind `wanted`, with media signed by `store`.
pub async fn hydrate_items(
    state: &AppState,
    wanted: &Wanted,
    viewer: &Viewer,
    store: Option<&FlowLikeStore>,
) -> Result<(AppMap, PackageMap), ApiError> {
    let (apps, mut packages) = futures::try_join!(
        apps(&state.db, &wanted.apps, &viewer.language, store),
        async {
            if viewer.dev {
                packages(state, &wanted.packages, &viewer.language).await
            } else {
                Ok(HashMap::new())
            }
        },
    )?;
    presign_packages(store, packages.values_mut()).await;
    Ok((apps, packages))
}

/// Ids selected by the rule collections among `placements`, each capped at the rule's limit times `scale`.
/// Package rules only run for developer viewers.
pub async fn rule_items<C: ConnectionTrait>(
    db: &C,
    placements: &[&PlacementDoc],
    viewer: &Viewer,
    scale: u8,
) -> Result<RuleItems, ApiError> {
    let tasks = placements
        .iter()
        .filter_map(|placement| match &placement.content {
            PlacementContent::Collection {
                source: CollectionSource::Rule,
                rule: Some(rule),
                ..
            } if rule.item_kind != ItemKind::Package || viewer.dev => {
                Some((placement.id.clone(), rule))
            }
            _ => None,
        })
        .map(|(id, rule)| async move {
            let limit = u64::from(rule.limit) * u64::from(scale);
            let items = query::rule_ids(db, rule, &viewer.language, limit).await?;
            Ok::<_, ApiError>((id, items))
        });
    Ok(try_join_all(tasks).await?.into_iter().collect())
}

/// Everything selection may show for `candidates`, in one batch: rule collections and rails first, then one app
/// query, one meta query and one registry query for every id they and the candidates reference.
pub async fn hydrate(
    state: &AppState,
    candidates: &Candidates,
    viewer: &Viewer,
    now: DateTime<Utc>,
) -> Result<Hydrated, ApiError> {
    let rule_collections = candidates.rule_collections();
    let rail_keys = candidates.rail_keys();
    let (rule_items, rails) = futures::try_join!(
        rule_items(&state.db, &rule_collections, viewer, 1),
        rails::load(&state.db, &rail_keys, viewer.dev, now),
    )?;
    let mut wanted = Wanted::default();
    let item_ids = candidates.item_ids();
    wanted.add_all(item_ids.apps.iter().map(|id| (ItemKind::App, id.as_str())));
    wanted.add_all(
        item_ids
            .packages
            .iter()
            .map(|id| (ItemKind::Package, id.as_str())),
    );
    for items in rule_items.values() {
        wanted.add_all(items.iter().map(|(kind, id)| (*kind, id.as_str())));
    }
    for rail in rails.values() {
        wanted.add_all(rail.apps.iter().map(|id| (ItemKind::App, id.as_str())));
        wanted.add_all(
            rail.packages
                .iter()
                .map(|id| (ItemKind::Package, id.as_str())),
        );
    }
    let store = if wanted.is_empty() {
        None
    } else {
        media_store(state).await
    };
    let (apps, packages) = hydrate_items(state, &wanted, viewer, store.as_ref()).await?;
    Ok(Hydrated {
        apps,
        packages,
        rule_items,
        rails,
    })
}

fn missing_ref(kind: ItemKind, id: &str) -> ItemRef {
    ItemRef {
        kind,
        id: id.to_owned(),
        name: id.to_owned(),
        icon_url: None,
        cover_url: None,
        public: false,
        exists: false,
    }
}

async fn app_refs<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
    store: Option<&FlowLikeStore>,
) -> Result<HashMap<String, ItemRef>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .column(app::Column::Visibility)
        .column(app::Column::Status)
        .filter(app::Column::Id.is_in(ids.to_vec()))
        .into_tuple::<(String, Visibility, Status)>();
    let metas = meta::Entity::find()
        .filter(meta::Column::AppId.is_in(ids.to_vec()))
        .order_by_asc(meta::Column::Lang);
    let (rows, metas) = futures::try_join!(rows.all(db), metas.all(db))?;
    let mut best = best_meta(metas, DEFAULT_LANGUAGE, |meta| meta.app_id.as_ref());
    let tasks = rows.into_iter().map(|(id, visibility, status)| {
        let meta = best.remove(&id);
        async move {
            let public = matches!(
                visibility,
                Visibility::Public | Visibility::PublicRequestAccess
            ) && status == Status::Active;
            let (name, icon_url, cover_url) = match meta {
                Some(meta) => {
                    let mut media = Metadata {
                        icon: meta.icon,
                        thumbnail: meta.thumbnail,
                        ..Metadata::default()
                    };
                    if let Some(store) = store {
                        media.presign(app_media(&id), store).await;
                    }
                    (meta.name, media.icon, media.thumbnail)
                }
                None => (id.clone(), None, None),
            };
            let item = ItemRef {
                kind: ItemKind::App,
                id: id.clone(),
                name,
                icon_url,
                cover_url,
                public,
                exists: true,
            };
            (id, item)
        }
    });
    Ok(join_all(tasks).await.into_iter().collect())
}

async fn package_refs<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
    store: Option<&FlowLikeStore>,
) -> Result<HashMap<String, ItemRef>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = wasm_package::Entity::find()
        .select_only()
        .column(wasm_package::Column::Id)
        .column(wasm_package::Column::Name)
        .column(wasm_package::Column::Visibility)
        .column(wasm_package::Column::Status)
        .filter(wasm_package::Column::Id.is_in(ids.to_vec()))
        .into_tuple::<(String, String, WasmPackageVisibility, WasmPackageStatus)>();
    let metas = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.is_in(ids.to_vec()))
        .order_by_asc(meta::Column::Lang);
    let (rows, metas) = futures::try_join!(rows.all(db), metas.all(db))?;
    let mut best = best_meta(metas, DEFAULT_LANGUAGE, |meta| {
        meta.wasm_package_id.as_ref()
    });
    let tasks = rows.into_iter().map(|(id, name, visibility, status)| {
        let meta = best.remove(&id).map(|meta| MetaSummary::from_model(&meta));
        async move {
            let public = matches!(
                visibility,
                WasmPackageVisibility::Public | WasmPackageVisibility::PublicRequestAccess
            ) && status == WasmPackageStatus::Active;
            let (name, icon_url, cover_url) = match meta {
                Some(mut meta) => {
                    if let Some(store) = store {
                        meta.presign_media(&id, store).await;
                    }
                    (meta.name, meta.icon, meta.thumbnail)
                }
                None => (name, None, None),
            };
            let item = ItemRef {
                kind: ItemKind::Package,
                id: id.clone(),
                name,
                icon_url,
                cover_url,
                public,
                exists: true,
            };
            (id, item)
        }
    });
    Ok(join_all(tasks).await.into_iter().collect())
}

fn referenced(layout: &LayoutDoc) -> Vec<(ItemKind, &str)> {
    let mut seen = HashSet::new();
    layout
        .placements()
        .flat_map(|(_, placement)| placement.items.iter())
        .map(|item| (item.kind, item.id.as_str()))
        .filter(|key| seen.insert(*key))
        .collect()
}

/// Every item the layout references, private and deleted ones included, for the editor. A collection item is
/// "public" while its placement is enabled.
pub async fn item_refs(state: &AppState, layout: &LayoutDoc) -> Result<Vec<ItemRef>, ApiError> {
    let items = referenced(layout);
    let ids_of = |kind: ItemKind| -> Vec<String> {
        items
            .iter()
            .filter(|(item_kind, _)| *item_kind == kind)
            .map(|(_, id)| (*id).to_owned())
            .collect()
    };
    let (app_ids, package_ids) = (ids_of(ItemKind::App), ids_of(ItemKind::Package));
    let store = if app_ids.is_empty() && package_ids.is_empty() {
        None
    } else {
        media_store(state).await
    };
    let (apps, packages) = futures::try_join!(
        app_refs(&state.db, &app_ids, store.as_ref()),
        package_refs(&state.db, &package_ids, store.as_ref()),
    )?;
    Ok(items
        .into_iter()
        .map(|(kind, id)| {
            let found = match kind {
                ItemKind::App => apps.get(id).cloned(),
                ItemKind::Package => packages.get(id).cloned(),
                ItemKind::Collection => layout.find(id).and_then(|(_, placement)| match &placement
                    .content
                {
                    PlacementContent::Collection { title, .. } => Some(ItemRef {
                        kind,
                        id: id.to_owned(),
                        name: title.clone(),
                        icon_url: None,
                        cover_url: None,
                        public: placement.enabled,
                        exists: true,
                    }),
                    _ => None,
                }),
            };
            found.unwrap_or_else(|| missing_ref(kind, id))
        })
        .collect())
}

fn kind_label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::App => "App",
        ItemKind::Package => "Package",
        ItemKind::Collection => "Collection",
    }
}

/// One line per placement item that viewers cannot see because it was deleted, is private or is a draft.
pub fn ref_warnings(layout: &LayoutDoc, refs: &[ItemRef]) -> Vec<String> {
    let by_key: HashMap<(ItemKind, &str), &ItemRef> = refs
        .iter()
        .map(|item| ((item.kind, item.id.as_str()), item))
        .collect();
    let mut warnings = Vec::new();
    for (_, placement) in layout.placements() {
        for item in &placement.items {
            let Some(found) = by_key.get(&(item.kind, item.id.as_str())) else {
                continue;
            };
            let problem = match (found.exists, found.public, item.kind) {
                (false, _, _) => "no longer exists",
                (true, false, ItemKind::Collection) => "is a draft, so its slide stays hidden",
                (true, false, _) => "is not public, so viewers do not see it",
                (true, true, _) => continue,
            };
            warnings.push(format!(
                "{} {} in {} {problem}",
                kind_label(item.kind),
                found.name,
                placement.name
            ));
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::explore::defaults::default_layout;
    use crate::routes::explore::model::{ItemOverrides, PlacementItemDoc, SLOT_HERO};
    use serde_json::json;

    fn package(id: &str, primary: &str) -> PackageSummary {
        serde_json::from_value(json!({
            "id": id,
            "name": id,
            "description": "",
            "latestVersion": "1.0.0",
            "downloadCount": 0,
            "status": "active",
            "keywords": [],
            "verified": false,
            "primaryCategory": primary,
            "secondaryCategory": "Insurance"
        }))
        .unwrap()
    }

    #[test]
    fn packages_are_emitted_in_id_order_whatever_the_registry_returns() {
        let found = index_packages(vec![
            package("c", "Education"),
            package("a", "AnalyticsReporting"),
            package("b", "FINANCE_BILLING"),
        ]);
        let ids: Vec<String> = ["a", "missing", "b", "c"].map(String::from).to_vec();
        let ordered: Vec<String> = in_id_order(&ids, &found)
            .into_iter()
            .map(|package| package.id)
            .collect();
        assert_eq!(ordered, ["a", "b", "c"]);
        assert_eq!(
            found["a"].primary_category.as_deref(),
            Some("ANALYTICS_REPORTING")
        );
        assert_eq!(
            found["b"].primary_category.as_deref(),
            Some("FINANCE_BILLING")
        );
        assert_eq!(found["c"].secondary_category.as_deref(), Some("INSURANCE"));
    }

    #[test]
    fn wanted_keeps_the_first_occurrence_of_each_id() {
        let mut wanted = Wanted::default();
        wanted.add_all([
            (ItemKind::App, "a"),
            (ItemKind::Package, "a"),
            (ItemKind::App, "b"),
            (ItemKind::App, "a"),
            (ItemKind::Collection, "c"),
        ]);
        assert_eq!(wanted.apps, ["a", "b"]);
        assert_eq!(wanted.packages, ["a"]);
    }

    fn meta(id: &str, app_id: &str, lang: &str) -> meta::Model {
        serde_json::from_value(json!({
            "id": id,
            "lang": lang,
            "name": id,
            "description": null,
            "long_description": null,
            "release_notes": null,
            "use_case": null,
            "icon": null,
            "thumbnail": null,
            "age_rating": null,
            "website": null,
            "support_url": null,
            "docs_url": null,
            "organization_specific_values": null,
            "app_id": app_id,
            "bit_id": null,
            "course_id": null,
            "template_id": null,
            "widget_id": null,
            "created_at": "2026-09-24T00:00:00Z",
            "updated_at": "2026-09-24T00:00:00Z",
            "wasm_package_id": null,
            "group_id": null,
            "tags": null,
            "preview_media": null
        }))
        .unwrap()
    }

    #[test]
    fn metadata_prefers_the_viewer_language_then_english() {
        let best = best_meta(
            vec![
                meta("a-de", "a", "de"),
                meta("a-en", "a", "en"),
                meta("a-fr", "a", "fr"),
                meta("b-es", "b", "es"),
                meta("b-fr", "b", "fr"),
                meta("c-en", "c", "en"),
            ],
            "fr",
            |meta| meta.app_id.as_ref(),
        );
        assert_eq!(best["a"].id, "a-fr");
        assert_eq!(best["b"].id, "b-fr");
        assert_eq!(best["c"].id, "c-en");
        let english = best_meta(
            vec![meta("a-de", "a", "de"), meta("a-en", "a", "en")],
            "ja",
            |meta| meta.app_id.as_ref(),
        );
        assert_eq!(english["a"].id, "a-en");
    }

    #[test]
    fn warnings_name_missing_private_and_draft_items() {
        let mut layout = default_layout();
        let hero = layout
            .slots
            .iter_mut()
            .find(|slot| slot.key == SLOT_HERO)
            .unwrap();
        hero.placements[0].items = [
            (ItemKind::App, "gone"),
            (ItemKind::App, "private"),
            (ItemKind::Package, "fine"),
            (ItemKind::Collection, "drafted"),
        ]
        .map(|(kind, id)| PlacementItemDoc::from_row(kind, id.into(), ItemOverrides::default()))
        .to_vec();
        let refs = vec![
            missing_ref(ItemKind::App, "gone"),
            ItemRef {
                name: "Private app".into(),
                exists: true,
                ..missing_ref(ItemKind::App, "private")
            },
            ItemRef {
                exists: true,
                public: true,
                ..missing_ref(ItemKind::Package, "fine")
            },
            ItemRef {
                name: "Picks".into(),
                exists: true,
                ..missing_ref(ItemKind::Collection, "drafted")
            },
        ];
        assert_eq!(
            ref_warnings(&layout, &refs),
            [
                "App gone in Popular right now no longer exists",
                "App Private app in Popular right now is not public, so viewers do not see it",
                "Collection Picks in Popular right now is a draft, so its slide stays hidden",
            ]
        );
        assert_eq!(referenced(&layout).len(), 4);
    }
}
