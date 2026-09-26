use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use flow_like::app::App;
use flow_like::bit::Metadata;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use super::audience;
use super::model::{
    Accent, COLLECTION_ITEMS_MIN, CollectionSource, ItemKind, LayoutDoc, PlacementContent,
    PlacementDoc, PlacementItemDoc, PlacementStatus, Platform, Projection, RailKey,
    SLOT_CATEGORIES, SLOT_COLLECTION, SLOT_FEATURE, SLOT_HERO, SLOT_NOTICE, SLOT_STAT,
    SLOT_UNPLACED, SkipReason, SkippedPlacement, SlotArea, SlotTrace, Tone, Viewer, slot_accepts,
};
use crate::routes::registry::types::PackageSummary;

pub const AUTO_FILL_SLIDES: usize = 3;
pub const COLLECTION_PREVIEW_MAX: usize = 4;

#[derive(Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedItem {
    /// The same `App` and `Metadata` objects `/apps/search` returns (snake_case inside).
    App {
        #[schema(value_type = Object)]
        app: App,
        #[schema(value_type = Object)]
        metadata: Option<Metadata>,
    },
    Package {
        package: PackageSummary,
    },
    Collection {
        collection: CollectionSummary,
    },
}

impl ResolvedItem {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::App { .. } => ItemKind::App,
            Self::Package { .. } => ItemKind::Package,
            Self::Collection { .. } => ItemKind::Collection,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::App { app, .. } => &app.id,
            Self::Package { package } => &package.id,
            Self::Collection { collection } => &collection.id,
        }
    }
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    pub id: String,
    pub title: String,
    pub blurb: Option<String>,
    pub apps: u32,
    pub packages: u32,
    /// At most four apps or packages.
    #[schema(no_recursion)]
    pub preview: Vec<ResolvedItem>,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSlide {
    pub item: ResolvedItem,
    pub headline: Option<String>,
    pub subline: Option<String>,
    pub artwork_url: Option<String>,
    pub accent: Option<Accent>,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSpotlight {
    pub placement_id: String,
    pub rotation_seconds: u8,
    pub slides: Vec<ResolvedSlide>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedAnnouncement {
    pub placement_id: String,
    /// `<placementId>:<12 hex of sha256(title|body|ctaHref)>`; editing the copy re-shows a dismissed announcement.
    pub dismiss_key: String,
    pub tone: Tone,
    pub title: String,
    pub body: String,
    pub cta_label: Option<String>,
    pub cta_href: Option<String>,
    pub image_url: Option<String>,
    pub dismissible: bool,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedFeature {
    pub placement_id: String,
    pub eyebrow: Option<String>,
    pub slide: ResolvedSlide,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedCollection {
    pub placement_id: String,
    pub title: String,
    pub blurb: Option<String>,
    pub source: CollectionSource,
    pub items: Vec<ResolvedItem>,
    pub apps: u32,
    pub packages: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RailStat {
    pub total: u32,
    pub apps: u32,
    pub packages: u32,
    pub free_apps: u32,
    pub paid_apps: u32,
    pub verified_packages: u32,
}

/// `app_category` is the core `AppCategory` serde name (PascalCase).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RailCategory {
    pub app_category: String,
    pub apps: u32,
    pub packages: u32,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedRail {
    pub placement_id: String,
    pub rail: RailKey,
    pub title: Option<String>,
    pub items: Vec<ResolvedItem>,
    pub stat: Option<RailStat>,
    pub categories: Option<Vec<RailCategory>>,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedRow {
    Rail { rail: ResolvedRail },
    Collection { collection: ResolvedCollection },
}

#[derive(Clone, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedGrid {
    pub hero: Option<ResolvedSpotlight>,
    pub notice: Option<ResolvedAnnouncement>,
    pub feature: Option<ResolvedFeature>,
    pub collection: Option<ResolvedCollection>,
    pub stat: Option<ResolvedRail>,
    pub categories: Option<ResolvedRail>,
}

#[derive(Clone, Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedView {
    pub grid: ResolvedGrid,
    pub rows: Vec<ResolvedRow>,
}

/// `all` is always present (the apps-only page for a non-dev viewer); `apps` and `packages` only for dev viewers.
#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedViews {
    pub all: ResolvedView,
    pub apps: Option<ResolvedView>,
    pub packages: Option<ResolvedView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewerEcho {
    pub dev: bool,
    pub signed_in: bool,
    pub platform: Platform,
    pub language: String,
}

impl From<&Viewer> for ViewerEcho {
    fn from(viewer: &Viewer) -> Self {
        Self {
            dev: viewer.dev,
            signed_in: viewer.signed_in,
            platform: viewer.platform,
            language: viewer.language.clone(),
        }
    }
}

/// `packages` is 0 for non-dev viewers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypeCounts {
    pub apps: u64,
    pub packages: u64,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedExplore {
    pub revision: String,
    pub generated_at: DateTime<Utc>,
    pub viewer: ViewerEcho,
    pub type_counts: TypeCounts,
    pub views: ResolvedViews,
}

#[derive(Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExplorePreview {
    pub page: ResolvedExplore,
    /// Selection trace of the `all` view.
    pub trace: Vec<SlotTrace>,
}

/// Rail ids in rank order (apps and packages separately) plus the aggregates of the stat/category rails.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RailData {
    pub apps: Vec<String>,
    pub packages: Vec<String>,
    /// `new_count` only.
    pub stat: Option<RailStat>,
    /// `by_category` only.
    pub categories: Vec<RailCategory>,
}

/// Everything selection may show, fetched in one batch. Only public, active apps and packages belong here;
/// `packages` stays empty for non-dev viewers.
#[derive(Default)]
pub struct Hydrated {
    pub apps: HashMap<String, (App, Option<Metadata>)>,
    pub packages: HashMap<String, PackageSummary>,
    /// Rule-collection results by placement id, in rank order.
    pub rule_items: HashMap<String, Vec<(ItemKind, String)>>,
    pub rails: HashMap<RailKey, RailData>,
}

impl Hydrated {
    pub fn app(&self, id: &str) -> Option<&(App, Option<Metadata>)> {
        self.apps.get(id)
    }

    pub fn package(&self, id: &str) -> Option<&PackageSummary> {
        self.packages.get(id)
    }

    pub fn rule_items(&self, placement_id: &str) -> &[(ItemKind, String)] {
        self.rule_items
            .get(placement_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn rail(&self, key: RailKey) -> Option<&RailData> {
        self.rails.get(&key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SlotEntry {
    Candidate(PlacementDoc),
    Skipped(SkippedPlacement),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlotCandidates {
    pub key: String,
    pub area: SlotArea,
    /// Priority order; placements that are not live or miss the audience are already `Skipped`.
    pub entries: Vec<SlotEntry>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidates {
    /// Grid slots and rows of the layout; `unplaced` is never a candidate slot.
    pub slots: Vec<SlotCandidates>,
    /// COLLECTION placements from any slot that pass [`collection_visible`], by id.
    pub collections: HashMap<String, PlacementDoc>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemIds {
    pub apps: Vec<String>,
    pub packages: Vec<String>,
}

impl Candidates {
    pub fn placements(&self) -> impl Iterator<Item = &PlacementDoc> {
        self.slots
            .iter()
            .flat_map(|slot| slot.entries.iter())
            .filter_map(|entry| match entry {
                SlotEntry::Candidate(placement) => Some(placement),
                SlotEntry::Skipped(_) => None,
            })
    }

    fn slide_collections(&self) -> impl Iterator<Item = &PlacementDoc> {
        self.placements()
            .filter(|placement| matches!(placement.content, PlacementContent::Spotlight { .. }))
            .flat_map(|placement| placement.items.iter())
            .filter(|item| item.kind == ItemKind::Collection)
            .filter_map(|item| self.collections.get(&item.id))
    }

    /// App and package ids to hydrate: every candidate's items plus the items of visible collections that
    /// spotlight slides reference. Rule results and rails are hydrated separately.
    pub fn item_ids(&self) -> ItemIds {
        let mut ids = ItemIds::default();
        let mut seen = HashSet::new();
        for item in self
            .placements()
            .chain(self.slide_collections())
            .flat_map(|placement| placement.items.iter())
        {
            if !seen.insert((item.kind, item.id.as_str())) {
                continue;
            }
            match item.kind {
                ItemKind::App => ids.apps.push(item.id.clone()),
                ItemKind::Package => ids.packages.push(item.id.clone()),
                ItemKind::Collection => {}
            }
        }
        ids
    }

    /// Rule-based collections whose rule query must run: candidates and visible slide targets.
    pub fn rule_collections(&self) -> Vec<&PlacementDoc> {
        let mut seen = HashSet::new();
        self.placements()
            .chain(self.slide_collections())
            .filter(|placement| {
                matches!(
                    placement.content,
                    PlacementContent::Collection {
                        source: CollectionSource::Rule,
                        ..
                    }
                )
            })
            .filter(|placement| seen.insert(placement.id.as_str()))
            .collect()
    }

    /// Rails selection may need: every candidate rail except `suites` (client-rendered), plus `trending` when
    /// a spotlight fills itself.
    pub fn rail_keys(&self) -> Vec<RailKey> {
        let mut keys = Vec::new();
        for placement in self.placements() {
            let key = match &placement.content {
                PlacementContent::Rail { rail, .. } if *rail != RailKey::Suites => Some(*rail),
                PlacementContent::Spotlight {
                    auto_fill: true, ..
                } => Some(RailKey::Trending),
                _ => None,
            };
            if let Some(key) = key
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        keys
    }
}

fn eligibility(
    placement: &PlacementDoc,
    viewer: &Viewer,
    now: DateTime<Utc>,
) -> Option<SkipReason> {
    match placement.status_at(now) {
        PlacementStatus::Draft => Some(SkipReason::Draft),
        PlacementStatus::Scheduled => Some(SkipReason::Scheduled),
        PlacementStatus::Ended => Some(SkipReason::Ended),
        PlacementStatus::Live if !audience::matches(&placement.audience, viewer) => {
            Some(SkipReason::Audience)
        }
        PlacementStatus::Live => None,
    }
}

/// A COLLECTION placement reachable by id (spotlight slide, `?collection=`): enabled, inside its window and
/// matching the audience. The slot is ignored, so unplaced collections qualify.
pub fn collection_visible(placement: &PlacementDoc, viewer: &Viewer, now: DateTime<Utc>) -> bool {
    matches!(placement.content, PlacementContent::Collection { .. })
        && eligibility(placement, viewer, now).is_none()
}

pub fn candidates(layout: &LayoutDoc, viewer: &Viewer, now: DateTime<Utc>) -> Candidates {
    let collections = layout
        .placements()
        .map(|(_, placement)| placement)
        .filter(|placement| collection_visible(placement, viewer, now))
        .map(|placement| (placement.id.clone(), placement.clone()))
        .collect();
    let slots = layout
        .slots
        .iter()
        .filter(|slot| slot.area != SlotArea::Unplaced)
        .map(|slot| SlotCandidates {
            key: slot.key.clone(),
            area: slot.area,
            entries: slot
                .placements
                .iter()
                .map(|placement| match eligibility(placement, viewer, now) {
                    None => SlotEntry::Candidate(placement.clone()),
                    Some(reason) => SlotEntry::Skipped(SkippedPlacement {
                        placement_id: placement.id.clone(),
                        reason,
                    }),
                })
                .collect(),
        })
        .collect();
    Candidates { slots, collections }
}

pub fn dismiss_key(placement_id: &str, title: &str, body: &str, cta_href: Option<&str>) -> String {
    let digest = Sha256::digest(format!("{title}|{body}|{}", cta_href.unwrap_or_default()));
    format!("{placement_id}:{}", &hex::encode(digest)[..12])
}

fn shortfall(shown: usize, gated: usize, total: usize, min: usize) -> Option<SkipReason> {
    if shown >= min {
        None
    } else if total == 0 {
        Some(SkipReason::Empty)
    } else if gated > 0 && shown + gated >= min {
        Some(SkipReason::DevOnly)
    } else {
        Some(SkipReason::TooFewItems)
    }
}

#[derive(Default)]
struct Picked {
    items: Vec<ResolvedItem>,
    gated: usize,
    total: usize,
}

impl Picked {
    fn shortfall(&self, min: usize) -> Option<SkipReason> {
        shortfall(self.items.len(), self.gated, self.total, min)
    }
}

fn kind_counts(items: &[ResolvedItem]) -> (u32, u32) {
    items
        .iter()
        .fold((0, 0), |(apps, packages), item| match item.kind() {
            ItemKind::App => (apps + 1, packages),
            ItemKind::Package => (apps, packages + 1),
            ItemKind::Collection => (apps, packages),
        })
}

fn slide(item: ResolvedItem, doc: Option<&PlacementItemDoc>) -> ResolvedSlide {
    ResolvedSlide {
        item,
        headline: doc.and_then(|doc| doc.headline.clone()),
        subline: doc.and_then(|doc| doc.subline.clone()),
        artwork_url: doc.and_then(|doc| doc.artwork_url.clone()),
        accent: doc.and_then(|doc| doc.accent.clone()),
    }
}

/// Collection members in display order: pinned or hand-picked items, then the rule's results.
fn collection_refs<'a>(
    placement: &'a PlacementDoc,
    hydrated: &'a Hydrated,
) -> Vec<(ItemKind, &'a str)> {
    let mut refs: Vec<(ItemKind, &str)> = placement
        .items
        .iter()
        .filter(|item| item.kind != ItemKind::Collection)
        .map(|item| (item.kind, item.id.as_str()))
        .collect();
    if matches!(
        placement.content,
        PlacementContent::Collection {
            source: CollectionSource::Rule,
            ..
        }
    ) {
        for (kind, id) in hydrated.rule_items(&placement.id) {
            if *kind != ItemKind::Collection && !refs.contains(&(*kind, id.as_str())) {
                refs.push((*kind, id.as_str()));
            }
        }
    }
    refs
}

enum Tile {
    Spotlight(ResolvedSpotlight),
    Announcement(ResolvedAnnouncement),
    Feature(ResolvedFeature),
    Collection(ResolvedCollection),
    Rail(ResolvedRail),
}

struct Selector<'a> {
    hydrated: &'a Hydrated,
    viewer: &'a Viewer,
    projection: Projection,
    collections: &'a HashMap<String, PlacementDoc>,
}

impl Selector<'_> {
    fn shows(&self, kind: ItemKind) -> bool {
        match kind {
            ItemKind::App => self.projection != Projection::Packages,
            ItemKind::Package => self.viewer.dev && self.projection != Projection::Apps,
            ItemKind::Collection => true,
        }
    }

    fn item(&self, kind: ItemKind, id: &str) -> Option<ResolvedItem> {
        match kind {
            ItemKind::App => self
                .hydrated
                .app(id)
                .map(|(app, metadata)| ResolvedItem::App {
                    app: app.clone(),
                    metadata: metadata.clone(),
                }),
            ItemKind::Package => self
                .hydrated
                .package(id)
                .map(|package| ResolvedItem::Package {
                    package: package.clone(),
                }),
            ItemKind::Collection => None,
        }
    }

    fn pick<'r>(&self, refs: impl IntoIterator<Item = (ItemKind, &'r str)>) -> Picked {
        let mut picked = Picked::default();
        for (kind, id) in refs {
            picked.total += 1;
            if !self.shows(kind) {
                picked.gated += 1;
            } else if let Some(item) = self.item(kind, id) {
                picked.items.push(item);
            }
        }
        picked
    }

    fn rail_picked(&self, key: RailKey) -> Picked {
        let Some(data) = self.hydrated.rail(key) else {
            return Picked::default();
        };
        self.pick(
            data.apps
                .iter()
                .map(|id| (ItemKind::App, id.as_str()))
                .chain(
                    data.packages
                        .iter()
                        .map(|id| (ItemKind::Package, id.as_str())),
                ),
        )
    }

    fn collection(
        &self,
        placement: &PlacementDoc,
    ) -> Option<(ResolvedCollection, Option<SkipReason>)> {
        let PlacementContent::Collection {
            title,
            blurb,
            source,
            ..
        } = &placement.content
        else {
            return None;
        };
        let picked = self.pick(collection_refs(placement, self.hydrated));
        let shortfall = picked.shortfall(COLLECTION_ITEMS_MIN);
        let (apps, packages) = kind_counts(&picked.items);
        Some((
            ResolvedCollection {
                placement_id: placement.id.clone(),
                title: title.clone(),
                blurb: blurb.clone(),
                source: *source,
                items: picked.items,
                apps,
                packages,
            },
            shortfall,
        ))
    }

    fn collection_summary(&self, id: &str) -> Option<CollectionSummary> {
        let (collection, shortfall) = self.collection(self.collections.get(id)?)?;
        if shortfall.is_some() {
            return None;
        }
        Some(CollectionSummary {
            id: collection.placement_id,
            title: collection.title,
            blurb: collection.blurb,
            apps: collection.apps,
            packages: collection.packages,
            preview: collection
                .items
                .into_iter()
                .take(COLLECTION_PREVIEW_MAX)
                .collect(),
        })
    }

    fn spotlight(
        &self,
        placement: &PlacementDoc,
        rotation_seconds: u8,
        auto_fill: bool,
    ) -> Result<ResolvedSpotlight, SkipReason> {
        let mut slides: Vec<ResolvedSlide> = Vec::new();
        let mut gated = 0;
        for doc in &placement.items {
            let item = match doc.kind {
                ItemKind::Collection => self
                    .collection_summary(&doc.id)
                    .map(|collection| ResolvedItem::Collection { collection }),
                kind if self.shows(kind) => self.item(kind, &doc.id),
                _ => {
                    gated += 1;
                    None
                }
            };
            if let Some(item) = item {
                slides.push(slide(item, Some(doc)));
            }
        }
        if auto_fill {
            for item in self.rail_picked(RailKey::Trending).items {
                if slides.len() >= AUTO_FILL_SLIDES {
                    break;
                }
                let present = slides
                    .iter()
                    .any(|slide| slide.item.kind() == item.kind() && slide.item.id() == item.id());
                if !present {
                    slides.push(slide(item, None));
                }
            }
        }
        if let Some(reason) = shortfall(slides.len(), gated, placement.items.len(), 1) {
            return Err(reason);
        }
        Ok(ResolvedSpotlight {
            placement_id: placement.id.clone(),
            rotation_seconds,
            slides,
        })
    }

    fn feature(
        &self,
        placement: &PlacementDoc,
        eyebrow: &Option<String>,
    ) -> Result<ResolvedFeature, SkipReason> {
        let doc = placement.items.first();
        let picked = self.pick(doc.map(|doc| (doc.kind, doc.id.as_str())));
        if let Some(reason) = picked.shortfall(1) {
            return Err(reason);
        }
        let item = picked.items.into_iter().next().ok_or(SkipReason::Empty)?;
        Ok(ResolvedFeature {
            placement_id: placement.id.clone(),
            eyebrow: eyebrow.clone(),
            slide: slide(item, doc),
        })
    }

    fn stat(&self) -> Result<RailStat, SkipReason> {
        let full = self
            .hydrated
            .rail(RailKey::NewCount)
            .and_then(|data| data.stat)
            .unwrap_or_default();
        let apps = self.shows(ItemKind::App);
        let packages = self.shows(ItemKind::Package);
        let keep = |shown: bool, value: u32| if shown { value } else { 0 };
        let stat = RailStat {
            apps: keep(apps, full.apps),
            free_apps: keep(apps, full.free_apps),
            paid_apps: keep(apps, full.paid_apps),
            packages: keep(packages, full.packages),
            verified_packages: keep(packages, full.verified_packages),
            total: keep(apps, full.apps) + keep(packages, full.packages),
        };
        if stat.total > 0 {
            Ok(stat)
        } else if full.apps + full.packages > 0 {
            Err(SkipReason::DevOnly)
        } else {
            Err(SkipReason::Empty)
        }
    }

    fn categories(&self) -> Result<Vec<RailCategory>, SkipReason> {
        let all = self
            .hydrated
            .rail(RailKey::ByCategory)
            .map(|data| data.categories.as_slice())
            .unwrap_or_default();
        let apps = self.shows(ItemKind::App);
        let packages = self.shows(ItemKind::Package);
        let visible: Vec<RailCategory> = all
            .iter()
            .map(|category| RailCategory {
                app_category: category.app_category.clone(),
                apps: if apps { category.apps } else { 0 },
                packages: if packages { category.packages } else { 0 },
            })
            .filter(|category| category.apps + category.packages > 0)
            .collect();
        if !visible.is_empty() {
            Ok(visible)
        } else if all
            .iter()
            .any(|category| category.apps + category.packages > 0)
        {
            Err(SkipReason::DevOnly)
        } else {
            Err(SkipReason::Empty)
        }
    }

    fn rail(
        &self,
        placement: &PlacementDoc,
        rail: RailKey,
        title: &Option<String>,
    ) -> Result<ResolvedRail, SkipReason> {
        let mut resolved = ResolvedRail {
            placement_id: placement.id.clone(),
            rail,
            title: title.clone(),
            items: Vec::new(),
            stat: None,
            categories: None,
        };
        match rail {
            RailKey::Suites if self.projection == Projection::Packages => {
                return Err(SkipReason::Empty);
            }
            RailKey::Suites => {}
            RailKey::NewCount => resolved.stat = Some(self.stat()?),
            RailKey::ByCategory => resolved.categories = Some(self.categories()?),
            RailKey::Trending | RailKey::New | RailKey::TopPaid | RailKey::ForBuilders => {
                let picked = self.rail_picked(rail);
                if let Some(reason) = picked.shortfall(1) {
                    return Err(reason);
                }
                resolved.items = picked.items;
            }
        }
        Ok(resolved)
    }

    fn resolve(&self, slot_key: &str, placement: &PlacementDoc) -> Result<Tile, SkipReason> {
        if slot_key == SLOT_UNPLACED || !slot_accepts(slot_key, &placement.content) {
            return Err(SkipReason::Empty);
        }
        match &placement.content {
            PlacementContent::Announcement {
                tone,
                title,
                body,
                cta_label,
                cta_href,
                image_url,
                dismissible,
            } => {
                let cta = cta_label.as_ref().zip(cta_href.as_ref());
                let cta_href = cta.map(|(_, href)| href.clone());
                Ok(Tile::Announcement(ResolvedAnnouncement {
                    placement_id: placement.id.clone(),
                    dismiss_key: dismiss_key(&placement.id, title, body, cta_href.as_deref()),
                    tone: *tone,
                    title: title.clone(),
                    body: body.clone(),
                    cta_label: cta.map(|(label, _)| label.clone()),
                    cta_href,
                    image_url: image_url.clone(),
                    dismissible: *dismissible,
                }))
            }
            PlacementContent::Spotlight {
                rotation_seconds,
                auto_fill,
            } => self
                .spotlight(placement, *rotation_seconds, *auto_fill)
                .map(Tile::Spotlight),
            PlacementContent::Feature { eyebrow } => {
                self.feature(placement, eyebrow).map(Tile::Feature)
            }
            PlacementContent::Collection { .. } => match self.collection(placement) {
                Some((collection, None)) => Ok(Tile::Collection(collection)),
                Some((_, Some(reason))) => Err(reason),
                None => Err(SkipReason::Empty),
            },
            PlacementContent::Rail { rail, title } => {
                self.rail(placement, *rail, title).map(Tile::Rail)
            }
            PlacementContent::Sponsored { .. } => Err(SkipReason::Empty),
        }
    }
}

fn place(view: &mut ResolvedView, slot_key: &str, tile: Tile) {
    match (slot_key, tile) {
        (SLOT_HERO, Tile::Spotlight(spotlight)) => view.grid.hero = Some(spotlight),
        (SLOT_NOTICE, Tile::Announcement(announcement)) => view.grid.notice = Some(announcement),
        (SLOT_FEATURE, Tile::Feature(feature)) => view.grid.feature = Some(feature),
        (SLOT_COLLECTION, Tile::Collection(collection)) => view.grid.collection = Some(collection),
        (SLOT_STAT, Tile::Rail(rail)) => view.grid.stat = Some(rail),
        (SLOT_CATEGORIES, Tile::Rail(rail)) => view.grid.categories = Some(rail),
        (_, Tile::Rail(rail)) => view.rows.push(ResolvedRow::Rail { rail }),
        (_, Tile::Collection(collection)) => view.rows.push(ResolvedRow::Collection { collection }),
        (_, Tile::Spotlight(_) | Tile::Announcement(_) | Tile::Feature(_)) => {}
    }
}

/// Resolves one view: per slot, the first candidate with enough visible items wins. Package items are
/// invisible for non-dev viewers and in the `apps` view, app items in the `packages` view.
pub fn select(
    candidates: &Candidates,
    hydrated: &Hydrated,
    viewer: &Viewer,
    projection: Projection,
) -> (ResolvedView, Vec<SlotTrace>) {
    let selector = Selector {
        hydrated,
        viewer,
        projection,
        collections: &candidates.collections,
    };
    let mut view = ResolvedView::default();
    let mut traces = Vec::with_capacity(candidates.slots.len());
    for slot in &candidates.slots {
        let mut trace = SlotTrace {
            slot_key: slot.key.clone(),
            chosen: None,
            skipped: Vec::new(),
        };
        for entry in &slot.entries {
            match entry {
                SlotEntry::Skipped(skipped) => trace.skipped.push(skipped.clone()),
                SlotEntry::Candidate(_) if trace.chosen.is_some() => {}
                SlotEntry::Candidate(placement) => match selector.resolve(&slot.key, placement) {
                    Ok(tile) => {
                        trace.chosen = Some(placement.id.clone());
                        place(&mut view, &slot.key, tile);
                    }
                    Err(reason) => trace.skipped.push(SkippedPlacement {
                        placement_id: placement.id.clone(),
                        reason,
                    }),
                },
            }
        }
        traces.push(trace);
    }
    (view, traces)
}

/// `all` plus, for dev viewers, the `apps` and `packages` views from the same hydration; the trace is `all`'s.
pub fn resolve_views(
    candidates: &Candidates,
    hydrated: &Hydrated,
    viewer: &Viewer,
) -> (ResolvedViews, Vec<SlotTrace>) {
    let (all, trace) = select(candidates, hydrated, viewer, Projection::All);
    let view = |projection| {
        viewer
            .dev
            .then(|| select(candidates, hydrated, viewer, projection).0)
    };
    (
        ResolvedViews {
            all,
            apps: view(Projection::Apps),
            packages: view(Projection::Packages),
        },
        trace,
    )
}

/// One COLLECTION placement with every item visible in `projection`, regardless of the two-item minimum
/// (Browse's `?collection=` page and collection search hits). `None` for other kinds.
pub fn resolve_collection(
    placement: &PlacementDoc,
    hydrated: &Hydrated,
    viewer: &Viewer,
    projection: Projection,
) -> Option<ResolvedCollection> {
    let collections = HashMap::new();
    Selector {
        hydrated,
        viewer,
        projection,
        collections: &collections,
    }
    .collection(placement)
    .map(|(collection, _)| collection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::explore::model::{
        CollectionRule, ItemOverrides, RuleSort, SlotDoc, slot_area,
    };
    use serde_json::json;

    fn app(id: &str) -> App {
        serde_json::from_value(json!({
            "id": id,
            "status": "Active",
            "visibility": "Public",
            "authors": [],
            "bits": [],
            "boards": [],
            "events": [],
            "templates": [],
            "changelog": null,
            "primary_category": "Finance",
            "secondary_category": null,
            "rating_sum": 0,
            "rating_count": 0,
            "download_count": 0,
            "interactions_count": 0,
            "avg_rating": null,
            "relevance_score": null,
            "execution_mode": "Any",
            "updated_at": {"secs_since_epoch": 0, "nanos_since_epoch": 0},
            "created_at": {"secs_since_epoch": 0, "nanos_since_epoch": 0},
            "version": null,
            "frontend": null,
            "price": 0
        }))
        .unwrap()
    }

    fn package(id: &str) -> PackageSummary {
        serde_json::from_value(json!({
            "id": id,
            "name": id,
            "description": "",
            "latestVersion": "1.0.0",
            "downloadCount": 0,
            "status": "active",
            "keywords": [],
            "verified": false
        }))
        .unwrap()
    }

    fn hydrated(apps: &[&str], packages: &[&str]) -> Hydrated {
        Hydrated {
            apps: apps
                .iter()
                .map(|id| (id.to_string(), (app(id), None)))
                .collect(),
            packages: packages
                .iter()
                .map(|id| (id.to_string(), package(id)))
                .collect(),
            ..Hydrated::default()
        }
    }

    fn with_rail(
        mut hydrated: Hydrated,
        key: RailKey,
        apps: &[&str],
        packages: &[&str],
    ) -> Hydrated {
        hydrated.rails.insert(
            key,
            RailData {
                apps: apps.iter().map(|id| id.to_string()).collect(),
                packages: packages.iter().map(|id| id.to_string()).collect(),
                ..RailData::default()
            },
        );
        hydrated
    }

    fn viewer(dev: bool) -> Viewer {
        Viewer {
            dev,
            signed_in: true,
            platform: Platform::Desktop,
            language: "en".into(),
        }
    }

    fn placement(id: &str, content: PlacementContent, items: &[(ItemKind, &str)]) -> PlacementDoc {
        PlacementDoc {
            id: id.into(),
            kind: content.kind(),
            name: id.into(),
            enabled: true,
            starts_at: None,
            ends_at: None,
            audience: vec![],
            content,
            items: items
                .iter()
                .map(|(kind, id)| {
                    PlacementItemDoc::from_row(*kind, id.to_string(), ItemOverrides::default())
                })
                .collect(),
            status: None,
            updated_at: None,
            created_at: None,
        }
    }

    fn feature(id: &str, items: &[(ItemKind, &str)]) -> PlacementDoc {
        placement(id, PlacementContent::Feature { eyebrow: None }, items)
    }

    fn spotlight(id: &str, auto_fill: bool, items: &[(ItemKind, &str)]) -> PlacementDoc {
        placement(
            id,
            PlacementContent::Spotlight {
                rotation_seconds: 8,
                auto_fill,
            },
            items,
        )
    }

    fn collection(id: &str, items: &[(ItemKind, &str)]) -> PlacementDoc {
        placement(
            id,
            PlacementContent::Collection {
                title: id.into(),
                blurb: None,
                source: CollectionSource::Hand,
                rule: None,
            },
            items,
        )
    }

    fn rail(id: &str, key: RailKey) -> PlacementDoc {
        placement(
            id,
            PlacementContent::Rail {
                rail: key,
                title: None,
            },
            &[],
        )
    }

    fn announcement(id: &str) -> PlacementDoc {
        placement(
            id,
            PlacementContent::Announcement {
                tone: Tone::Info,
                title: format!("{id} title"),
                body: "Body".into(),
                cta_label: None,
                cta_href: None,
                image_url: None,
                dismissible: true,
            },
            &[],
        )
    }

    fn layout(slots: Vec<(&str, Vec<PlacementDoc>)>) -> LayoutDoc {
        LayoutDoc {
            slots: slots
                .into_iter()
                .enumerate()
                .map(|(position, (key, placements))| SlotDoc {
                    key: key.into(),
                    area: slot_area(key).unwrap(),
                    position: position as i32,
                    placements,
                })
                .collect(),
        }
    }

    fn resolve(
        layout: &LayoutDoc,
        hydrated: &Hydrated,
        viewer: &Viewer,
        projection: Projection,
    ) -> (ResolvedView, Vec<SlotTrace>) {
        select(
            &candidates(layout, viewer, Utc::now()),
            hydrated,
            viewer,
            projection,
        )
    }

    fn item_key(item: &ResolvedItem) -> String {
        format!("{}:{}", item.kind().as_str(), item.id())
    }

    fn keys<'a>(items: impl IntoIterator<Item = &'a ResolvedItem>) -> String {
        items
            .into_iter()
            .map(item_key)
            .collect::<Vec<_>>()
            .join(",")
    }

    fn view_ids(view: &ResolvedView) -> Vec<String> {
        let grid = &view.grid;
        let mut ids = Vec::new();
        if let Some(hero) = &grid.hero {
            ids.push(format!(
                "hero:{}[{}]",
                hero.placement_id,
                keys(hero.slides.iter().map(|slide| &slide.item))
            ));
        }
        if let Some(notice) = &grid.notice {
            ids.push(format!("notice:{}", notice.placement_id));
        }
        if let Some(feature) = &grid.feature {
            ids.push(format!(
                "feature:{}[{}]",
                feature.placement_id,
                item_key(&feature.slide.item)
            ));
        }
        if let Some(collection) = &grid.collection {
            ids.push(format!(
                "collection:{}[{}]",
                collection.placement_id,
                keys(&collection.items)
            ));
        }
        if let Some(stat) = &grid.stat {
            ids.push(format!("stat:{}", stat.placement_id));
        }
        if let Some(categories) = &grid.categories {
            ids.push(format!("categories:{}", categories.placement_id));
        }
        for row in &view.rows {
            ids.push(match row {
                ResolvedRow::Rail { rail } => {
                    format!("row:{}[{}]", rail.placement_id, keys(&rail.items))
                }
                ResolvedRow::Collection { collection } => format!(
                    "row:{}[{}]",
                    collection.placement_id,
                    keys(&collection.items)
                ),
            });
        }
        ids
    }

    fn skipped(traces: &[SlotTrace], slot: &str) -> Vec<(String, SkipReason)> {
        traces
            .iter()
            .find(|trace| trace.slot_key == slot)
            .map(|trace| {
                trace
                    .skipped
                    .iter()
                    .map(|skip| (skip.placement_id.clone(), skip.reason))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn dev_only_feature_falls_back_to_the_app_feature() {
        let layout = layout(vec![(
            SLOT_FEATURE,
            vec![
                feature("f-pkg", &[(ItemKind::Package, "p1")]),
                feature("f-app", &[(ItemKind::App, "a1")]),
            ],
        )]);
        let data = hydrated(&["a1"], &["p1"]);
        let (view, traces) = resolve(&layout, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["feature:f-app[app:a1]"]);
        assert_eq!(
            skipped(&traces, SLOT_FEATURE),
            [("f-pkg".into(), SkipReason::DevOnly)]
        );
        assert_eq!(traces[0].chosen.as_deref(), Some("f-app"));
        let (view, traces) = resolve(&layout, &data, &viewer(true), Projection::All);
        assert_eq!(view_ids(&view), ["feature:f-pkg[package:p1]"]);
        assert!(skipped(&traces, SLOT_FEATURE).is_empty());
    }

    #[test]
    fn collection_with_one_visible_item_yields_to_the_next() {
        let layout = layout(vec![(
            SLOT_COLLECTION,
            vec![
                collection("c1", &[(ItemKind::App, "a1"), (ItemKind::App, "gone")]),
                collection("c2", &[(ItemKind::App, "a1"), (ItemKind::App, "a2")]),
            ],
        )]);
        let (view, traces) = resolve(
            &layout,
            &hydrated(&["a1", "a2"], &[]),
            &viewer(false),
            Projection::All,
        );
        assert_eq!(view_ids(&view), ["collection:c2[app:a1,app:a2]"]);
        assert_eq!(
            skipped(&traces, SLOT_COLLECTION),
            [("c1".into(), SkipReason::TooFewItems)]
        );
    }

    #[test]
    fn spotlight_drops_package_slides_for_non_dev_and_yields_when_none_are_left() {
        let layout = layout(vec![(
            SLOT_HERO,
            vec![
                spotlight(
                    "s1",
                    false,
                    &[(ItemKind::Package, "p1"), (ItemKind::Package, "p2")],
                ),
                spotlight(
                    "s2",
                    false,
                    &[(ItemKind::App, "a1"), (ItemKind::Package, "p1")],
                ),
            ],
        )]);
        let data = hydrated(&["a1"], &["p1", "p2"]);
        let (view, traces) = resolve(&layout, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["hero:s2[app:a1]"]);
        assert_eq!(
            skipped(&traces, SLOT_HERO),
            [("s1".into(), SkipReason::DevOnly)]
        );
        let (view, _) = resolve(&layout, &data, &viewer(true), Projection::All);
        assert_eq!(view_ids(&view), ["hero:s1[package:p1,package:p2]"]);
    }

    #[test]
    fn only_the_highest_priority_announcement_shows() {
        let mut first = announcement("n1");
        let layout_both = layout(vec![(SLOT_NOTICE, vec![first.clone(), announcement("n2")])]);
        let data = hydrated(&[], &[]);
        let (view, traces) = resolve(&layout_both, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["notice:n1"]);
        assert!(traces[0].skipped.is_empty());
        let notice = view.grid.notice.unwrap();
        assert!(notice.dismiss_key.starts_with("n1:"));
        assert_eq!(notice.dismiss_key.len(), "n1:".len() + 12);
        first.audience = vec!["signed_out".into()];
        let layout_fallback = layout(vec![(SLOT_NOTICE, vec![first, announcement("n2")])]);
        let (view, traces) = resolve(&layout_fallback, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["notice:n2"]);
        assert_eq!(
            skipped(&traces, SLOT_NOTICE),
            [("n1".into(), SkipReason::Audience)]
        );
    }

    #[test]
    fn dismiss_key_changes_with_the_copy() {
        let key = dismiss_key("n1", "Title", "Body", Some("/store"));
        assert_eq!(key, dismiss_key("n1", "Title", "Body", Some("/store")));
        assert_ne!(key, dismiss_key("n1", "Title", "Body!", Some("/store")));
        assert_ne!(key, dismiss_key("n1", "Title", "Body", None));
        assert!(key[3..].bytes().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn an_announcement_shows_a_cta_only_with_label_and_href() {
        let with_cta = |label: Option<&str>, href: Option<&str>| {
            let mut notice = announcement("n1");
            if let PlacementContent::Announcement {
                cta_label,
                cta_href,
                ..
            } = &mut notice.content
            {
                *cta_label = label.map(str::to_owned);
                *cta_href = href.map(str::to_owned);
            }
            let layout = layout(vec![(SLOT_NOTICE, vec![notice])]);
            let (view, _) = resolve(
                &layout,
                &hydrated(&[], &[]),
                &viewer(false),
                Projection::All,
            );
            let notice = view.grid.notice.unwrap();
            (notice.cta_label, notice.cta_href)
        };
        assert_eq!(
            with_cta(Some("Browse"), Some("/store")),
            (Some("Browse".into()), Some("/store".into()))
        );
        assert_eq!(with_cta(Some("Browse"), None), (None, None));
        assert_eq!(with_cta(None, Some("/store")), (None, None));
    }

    #[test]
    fn unavailable_placements_are_skipped_with_their_reason() {
        let now = Utc::now();
        let hour = chrono::Duration::hours(1);
        let mut scheduled = feature("scheduled", &[(ItemKind::App, "a1")]);
        scheduled.starts_at = Some(now + hour);
        let mut ended = feature("ended", &[(ItemKind::App, "a1")]);
        ended.ends_at = Some(now - hour);
        let mut draft = feature("draft", &[(ItemKind::App, "a1")]);
        draft.enabled = false;
        let mut dev_only = feature("dev-only", &[(ItemKind::App, "a1")]);
        dev_only.audience = vec!["dev".into()];
        let mut later_draft = feature("later-draft", &[(ItemKind::App, "a1")]);
        later_draft.enabled = false;
        let layout = layout(vec![(
            SLOT_FEATURE,
            vec![
                scheduled,
                ended,
                draft,
                dev_only,
                feature("missing", &[(ItemKind::App, "gone")]),
                feature("live", &[(ItemKind::App, "a1")]),
                feature("fallback", &[(ItemKind::App, "a1")]),
                later_draft,
            ],
        )]);
        let (view, traces) = resolve(
            &layout,
            &hydrated(&["a1"], &[]),
            &viewer(false),
            Projection::All,
        );
        assert_eq!(view_ids(&view), ["feature:live[app:a1]"]);
        assert_eq!(
            skipped(&traces, SLOT_FEATURE),
            [
                ("scheduled".into(), SkipReason::Scheduled),
                ("ended".into(), SkipReason::Ended),
                ("draft".into(), SkipReason::Draft),
                ("dev-only".into(), SkipReason::Audience),
                ("missing".into(), SkipReason::TooFewItems),
                ("later-draft".into(), SkipReason::Draft),
            ]
        );
    }

    #[test]
    fn empty_builders_rail_falls_back_to_new() {
        let layout = layout(vec![(
            "row:builders",
            vec![
                rail("builders", RailKey::ForBuilders),
                rail("new", RailKey::New),
            ],
        )]);
        let data = with_rail(
            with_rail(
                hydrated(&["a1"], &["p1"]),
                RailKey::ForBuilders,
                &[],
                &["p1"],
            ),
            RailKey::New,
            &["a1"],
            &[],
        );
        let (view, traces) = resolve(&layout, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["row:new[app:a1]"]);
        assert_eq!(
            skipped(&traces, "row:builders"),
            [("builders".into(), SkipReason::DevOnly)]
        );
        let (view, _) = resolve(&layout, &data, &viewer(true), Projection::All);
        assert_eq!(view_ids(&view), ["row:builders[package:p1]"]);
        let empty = with_rail(data, RailKey::ForBuilders, &[], &[]);
        let (view, traces) = resolve(&layout, &empty, &viewer(true), Projection::All);
        assert_eq!(view_ids(&view), ["row:new[app:a1]"]);
        assert_eq!(
            skipped(&traces, "row:builders"),
            [("builders".into(), SkipReason::Empty)]
        );
    }

    #[test]
    fn unplaced_placements_are_never_tiles() {
        let layout = layout(vec![
            (SLOT_FEATURE, vec![]),
            (
                SLOT_UNPLACED,
                vec![
                    collection("c1", &[(ItemKind::App, "a1"), (ItemKind::App, "a2")]),
                    feature("f1", &[(ItemKind::App, "a1")]),
                ],
            ),
        ]);
        let (view, traces) = resolve(
            &layout,
            &hydrated(&["a1", "a2"], &[]),
            &viewer(true),
            Projection::All,
        );
        assert!(view_ids(&view).is_empty());
        assert!(traces.iter().all(|trace| trace.slot_key != SLOT_UNPLACED));
        assert_eq!(traces.len(), 1);
    }

    #[test]
    fn spotlight_collection_slides_require_a_visible_collection() {
        let now = Utc::now();
        let two = [(ItemKind::App, "a1"), (ItemKind::App, "a2")];
        let mut disabled = collection("disabled", &two);
        disabled.enabled = false;
        let mut scheduled = collection("scheduled", &two);
        scheduled.starts_at = Some(now + chrono::Duration::hours(1));
        let mut dev_only = collection("dev-only", &two);
        dev_only.audience = vec!["dev".into()];
        let unplaced = collection("unplaced", &two);
        let thin = collection("thin", &[(ItemKind::App, "a1"), (ItemKind::App, "gone")]);
        let hero = spotlight(
            "s1",
            false,
            &[
                (ItemKind::Collection, "disabled"),
                (ItemKind::Collection, "scheduled"),
                (ItemKind::Collection, "dev-only"),
                (ItemKind::Collection, "unplaced"),
                (ItemKind::Collection, "thin"),
                (ItemKind::App, "a3"),
            ],
        );
        let layout = layout(vec![
            (SLOT_HERO, vec![hero]),
            (
                SLOT_UNPLACED,
                vec![disabled, scheduled, dev_only, unplaced, thin],
            ),
        ]);
        let data = hydrated(&["a1", "a2", "a3"], &[]);
        let (view, _) = resolve(&layout, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["hero:s1[collection:unplaced,app:a3]"]);
        let Some(ResolvedItem::Collection { collection }) = view
            .grid
            .hero
            .as_ref()
            .map(|hero| hero.slides[0].item.clone())
        else {
            panic!("collection slide expected");
        };
        assert_eq!(keys(&collection.preview), "app:a1,app:a2");
        let dev = viewer(true);
        let candidates = candidates(&layout, &dev, now);
        assert!(!collection_visible(
            layout.find("disabled").unwrap().1,
            &dev,
            now
        ));
        assert!(candidates.collections.contains_key("dev-only"));
        assert!(!candidates.collections.contains_key("scheduled"));
        let (view, _) = select(&candidates, &data, &dev, Projection::All);
        assert_eq!(
            view_ids(&view),
            ["hero:s1[collection:dev-only,collection:unplaced,app:a3]"]
        );
        assert!(!collection_visible(&feature("f", &[]), &dev, now));
    }

    #[test]
    fn projections_resolve_their_own_fallbacks() {
        let layout = layout(vec![
            (
                SLOT_HERO,
                vec![spotlight(
                    "s1",
                    false,
                    &[(ItemKind::App, "a1"), (ItemKind::Package, "p1")],
                )],
            ),
            (
                SLOT_FEATURE,
                vec![
                    feature("f-pkg", &[(ItemKind::Package, "p1")]),
                    feature("f-app", &[(ItemKind::App, "a2")]),
                ],
            ),
            (
                SLOT_COLLECTION,
                vec![
                    collection(
                        "c-mixed",
                        &[(ItemKind::App, "a1"), (ItemKind::Package, "p1")],
                    ),
                    collection("c-apps", &[(ItemKind::App, "a1"), (ItemKind::App, "a2")]),
                ],
            ),
            ("row:suites", vec![rail("suites", RailKey::Suites)]),
            (
                "row:builders",
                vec![
                    rail("builders", RailKey::ForBuilders),
                    rail("new", RailKey::New),
                ],
            ),
        ]);
        let data = with_rail(
            with_rail(
                hydrated(&["a1", "a2"], &["p1"]),
                RailKey::ForBuilders,
                &[],
                &["p1"],
            ),
            RailKey::New,
            &["a2"],
            &[],
        );
        let dev = viewer(true);
        let candidates = candidates(&layout, &dev, Utc::now());
        let (views, trace) = resolve_views(&candidates, &data, &dev);
        assert_eq!(
            view_ids(&views.all),
            [
                "hero:s1[app:a1,package:p1]",
                "feature:f-pkg[package:p1]",
                "collection:c-mixed[app:a1,package:p1]",
                "row:suites[]",
                "row:builders[package:p1]",
            ]
        );
        assert_eq!(trace.len(), 5);
        assert_eq!(
            view_ids(views.apps.as_ref().unwrap()),
            [
                "hero:s1[app:a1]",
                "feature:f-app[app:a2]",
                "collection:c-apps[app:a1,app:a2]",
                "row:suites[]",
                "row:new[app:a2]",
            ]
        );
        assert_eq!(
            view_ids(views.packages.as_ref().unwrap()),
            [
                "hero:s1[package:p1]",
                "feature:f-pkg[package:p1]",
                "row:builders[package:p1]"
            ]
        );
        let non_dev = viewer(false);
        let (views, _) = resolve_views(
            &super::candidates(&layout, &non_dev, Utc::now()),
            &data,
            &non_dev,
        );
        assert!(views.apps.is_none());
        assert!(views.packages.is_none());
        assert_eq!(
            view_ids(&views.all),
            [
                "hero:s1[app:a1]",
                "feature:f-app[app:a2]",
                "collection:c-apps[app:a1,app:a2]",
                "row:suites[]",
                "row:new[app:a2]",
            ]
        );
    }

    #[test]
    fn auto_fill_spotlight_tops_up_from_trending_without_duplicates() {
        let data = with_rail(
            hydrated(&["a1", "a2", "a3", "a4", "a5"], &["p1", "p2"]),
            RailKey::Trending,
            &["a1", "a2", "a3", "a4"],
            &["p1", "p2"],
        );
        let empty = layout(vec![(SLOT_HERO, vec![spotlight("s1", true, &[])])]);
        let (view, _) = resolve(&empty, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["hero:s1[app:a1,app:a2,app:a3]"]);
        let (view, _) = resolve(&empty, &data, &viewer(true), Projection::Packages);
        assert_eq!(view_ids(&view), ["hero:s1[package:p1,package:p2]"]);
        let picked = layout(vec![(
            SLOT_HERO,
            vec![spotlight(
                "s1",
                true,
                &[(ItemKind::App, "a2"), (ItemKind::App, "a5")],
            )],
        )]);
        let (view, _) = resolve(&picked, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["hero:s1[app:a2,app:a5,app:a1]"]);
        let (view, traces) = resolve(&empty, &hydrated(&[], &[]), &viewer(false), Projection::All);
        assert!(view_ids(&view).is_empty());
        assert_eq!(
            skipped(&traces, SLOT_HERO),
            [("s1".into(), SkipReason::Empty)]
        );
        let candidates = candidates(&empty, &viewer(false), Utc::now());
        assert_eq!(candidates.rail_keys(), [RailKey::Trending]);
    }

    #[test]
    fn suites_row_has_no_items_and_skips_the_packages_view() {
        let layout = layout(vec![("row:suites", vec![rail("suites", RailKey::Suites)])]);
        let data = hydrated(&[], &[]);
        for projection in [Projection::All, Projection::Apps] {
            let (view, _) = resolve(&layout, &data, &viewer(true), projection);
            assert_eq!(view_ids(&view), ["row:suites[]"]);
        }
        let (view, _) = resolve(&layout, &data, &viewer(true), Projection::Packages);
        assert!(view.rows.is_empty());
        assert!(
            candidates(&layout, &viewer(true), Utc::now())
                .rail_keys()
                .is_empty()
        );
    }

    #[test]
    fn package_counts_are_zero_where_packages_are_hidden() {
        let items = [
            (ItemKind::App, "a1"),
            (ItemKind::App, "a2"),
            (ItemKind::Package, "p1"),
        ];
        let layout = layout(vec![
            (
                SLOT_HERO,
                vec![spotlight("s1", false, &[(ItemKind::Collection, "c1")])],
            ),
            (SLOT_COLLECTION, vec![collection("c1", &items)]),
        ]);
        let data = hydrated(&["a1", "a2"], &["p1"]);
        let counts = |dev: bool, projection| {
            let (view, _) = resolve(&layout, &data, &viewer(dev), projection);
            let collection = view.grid.collection.as_ref().map(|c| (c.apps, c.packages));
            let summary = view
                .grid
                .hero
                .as_ref()
                .and_then(|hero| match &hero.slides[0].item {
                    ResolvedItem::Collection { collection } => {
                        Some((collection.apps, collection.packages))
                    }
                    _ => None,
                });
            (collection, summary)
        };
        assert_eq!(counts(false, Projection::All), (Some((2, 0)), Some((2, 0))));
        assert_eq!(counts(true, Projection::All), (Some((2, 1)), Some((2, 1))));
        assert_eq!(counts(true, Projection::Apps), (Some((2, 0)), Some((2, 0))));
        assert_eq!(counts(true, Projection::Packages), (None, None));
    }

    #[test]
    fn stat_and_category_rails_zero_the_hidden_kind() {
        let layout = layout(vec![
            (SLOT_STAT, vec![rail("stat", RailKey::NewCount)]),
            (
                SLOT_CATEGORIES,
                vec![rail("categories", RailKey::ByCategory)],
            ),
        ]);
        let mut data = hydrated(&[], &[]);
        data.rails.insert(
            RailKey::NewCount,
            RailData {
                stat: Some(RailStat {
                    total: 5,
                    apps: 3,
                    packages: 2,
                    free_apps: 2,
                    paid_apps: 1,
                    verified_packages: 1,
                }),
                ..RailData::default()
            },
        );
        data.rails.insert(
            RailKey::ByCategory,
            RailData {
                categories: vec![
                    RailCategory {
                        app_category: "Finance".into(),
                        apps: 2,
                        packages: 1,
                    },
                    RailCategory {
                        app_category: "Education".into(),
                        apps: 0,
                        packages: 3,
                    },
                ],
                ..RailData::default()
            },
        );
        let (view, _) = resolve(&layout, &data, &viewer(false), Projection::All);
        let stat = view.grid.stat.as_ref().unwrap().stat.unwrap();
        assert_eq!(
            (stat.total, stat.apps, stat.packages, stat.verified_packages),
            (3, 3, 0, 0)
        );
        let categories = view
            .grid
            .categories
            .as_ref()
            .unwrap()
            .categories
            .clone()
            .unwrap();
        assert_eq!(
            categories,
            [RailCategory {
                app_category: "Finance".into(),
                apps: 2,
                packages: 0
            }]
        );
        let (view, _) = resolve(&layout, &data, &viewer(true), Projection::Packages);
        let stat = view.grid.stat.as_ref().unwrap().stat.unwrap();
        assert_eq!(
            (stat.total, stat.apps, stat.free_apps, stat.packages),
            (2, 0, 0, 2)
        );
        assert_eq!(view.grid.categories.unwrap().categories.unwrap().len(), 2);
        let (view, traces) = resolve(&layout, &hydrated(&[], &[]), &viewer(true), Projection::All);
        assert!(view.grid.stat.is_none());
        assert_eq!(
            skipped(&traces, SLOT_STAT),
            [("stat".into(), SkipReason::Empty)]
        );
    }

    #[test]
    fn rule_collections_append_rule_results_after_pinned_items() {
        let mut rule = collection("c1", &[(ItemKind::App, "a2")]);
        rule.content = PlacementContent::Collection {
            title: "Rule".into(),
            blurb: None,
            source: CollectionSource::Rule,
            rule: Some(CollectionRule {
                item_kind: ItemKind::App,
                app_category: Some("Finance".into()),
                app_type: None,
                package_category: None,
                verified_only: false,
                min_rating: None,
                price: None,
                sort: RuleSort::Installs,
                limit: 4,
            }),
        };
        let layout = layout(vec![(SLOT_COLLECTION, vec![rule])]);
        let mut data = hydrated(&["a1", "a2", "a3"], &[]);
        data.rule_items.insert(
            "c1".into(),
            vec![
                (ItemKind::App, "a1".into()),
                (ItemKind::App, "a2".into()),
                (ItemKind::App, "a3".into()),
            ],
        );
        let (view, _) = resolve(&layout, &data, &viewer(false), Projection::All);
        assert_eq!(view_ids(&view), ["collection:c1[app:a2,app:a1,app:a3]"]);
        let candidates = candidates(&layout, &viewer(false), Utc::now());
        assert_eq!(candidates.rule_collections()[0].id, "c1");
        let page = resolve_collection(
            layout.find("c1").unwrap().1,
            &data,
            &viewer(false),
            Projection::All,
        )
        .unwrap();
        assert_eq!(page.items.len(), 3);
    }

    #[test]
    fn hydration_ids_cover_candidates_and_visible_slide_collections() {
        let mut hidden = collection("hidden", &[(ItemKind::App, "h1"), (ItemKind::App, "h2")]);
        hidden.enabled = false;
        let layout = layout(vec![
            (
                SLOT_HERO,
                vec![spotlight(
                    "s1",
                    false,
                    &[
                        (ItemKind::Collection, "shown"),
                        (ItemKind::Collection, "hidden"),
                        (ItemKind::Package, "p1"),
                    ],
                )],
            ),
            (SLOT_FEATURE, vec![feature("f1", &[(ItemKind::App, "a1")])]),
            (
                SLOT_UNPLACED,
                vec![
                    collection("shown", &[(ItemKind::App, "a1"), (ItemKind::Package, "p2")]),
                    hidden,
                ],
            ),
            (
                "row:trending",
                vec![
                    rail("trending", RailKey::Trending),
                    rail("new", RailKey::New),
                ],
            ),
        ]);
        let candidates = candidates(&layout, &viewer(true), Utc::now());
        assert_eq!(
            candidates.item_ids(),
            ItemIds {
                apps: vec!["a1".into()],
                packages: vec!["p1".into(), "p2".into()],
            }
        );
        assert_eq!(candidates.rail_keys(), [RailKey::Trending, RailKey::New]);
        assert!(candidates.rule_collections().is_empty());
    }
}
