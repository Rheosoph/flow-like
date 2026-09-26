use std::collections::HashSet;

use chrono::{DateTime, Utc};
use flow_like::app::{AppCategory, AppType};
use flow_like_types::Value;
use flow_like_wasm_schema::manifest::WasmPackageCategory;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::openapi::{ObjectBuilder, RefOr, Schema, Type};

use super::audience;
use crate::entity::sea_orm_active_enums::{
    ExploreItemKind, ExplorePlacementKind, ExploreSlotArea,
    WasmPackageCategory as DbWasmPackageCategory,
};
use crate::error::ApiError;

pub const NAME_MAX: usize = 80;
pub const ANNOUNCEMENT_TITLE_MAX: usize = 60;
pub const ANNOUNCEMENT_BODY_MAX: usize = 140;
pub const CTA_LABEL_MAX: usize = 24;
pub const HEADLINE_MAX: usize = 60;
pub const SUBLINE_MAX: usize = 140;
pub const COLLECTION_TITLE_MAX: usize = 60;
pub const BLURB_MAX: usize = 140;
pub const RAIL_TITLE_MAX: usize = 40;
pub const ADVERTISER_MAX: usize = 60;
pub const EYEBROW_MAX: usize = 40;
pub const URL_MAX: usize = 2048;
pub const ITEM_ID_MAX: usize = 200;
pub const SPOTLIGHT_ITEMS_MAX: usize = 6;
pub const COLLECTION_ITEMS_MIN: usize = 2;
pub const COLLECTION_ITEMS_MAX: usize = 12;
pub const RULE_PINNED_MAX: usize = 4;
pub const PLACEMENTS_MAX: usize = 60;
pub const ROWS_MAX: usize = 12;
pub const ROW_SLUG_MAX: usize = 40;
pub const ROTATION_MIN: u8 = 5;
pub const ROTATION_MAX: u8 = 20;
pub const ROTATION_DEFAULT: u8 = 8;
pub const RULE_LIMIT_MIN: u8 = 2;
pub const RULE_LIMIT_MAX: u8 = 12;
pub const RATING_MAX: f32 = 5.0;
pub const SEARCH_CATEGORIES_MAX: usize = 16;
pub const SEARCH_PERMISSIONS_MAX: usize = 4;

pub const SLOT_HERO: &str = "hero";
pub const SLOT_NOTICE: &str = "notice";
pub const SLOT_FEATURE: &str = "feature";
pub const SLOT_COLLECTION: &str = "collection";
pub const SLOT_STAT: &str = "stat";
pub const SLOT_CATEGORIES: &str = "categories";
pub const SLOT_UNPLACED: &str = "unplaced";
pub const ROW_PREFIX: &str = "row:";
pub const GRID_SLOTS: [&str; 6] = [
    SLOT_HERO,
    SLOT_NOTICE,
    SLOT_FEATURE,
    SLOT_COLLECTION,
    SLOT_STAT,
    SLOT_CATEGORIES,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlacementKind {
    Announcement,
    Spotlight,
    Feature,
    Collection,
    Rail,
    Sponsored,
}

impl PlacementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Announcement => "announcement",
            Self::Spotlight => "spotlight",
            Self::Feature => "feature",
            Self::Collection => "collection",
            Self::Rail => "rail",
            Self::Sponsored => "sponsored",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RailKey {
    Trending,
    New,
    NewCount,
    TopPaid,
    ForBuilders,
    ByCategory,
    Suites,
}

impl RailKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trending => "trending",
            Self::New => "new",
            Self::NewCount => "new_count",
            Self::TopPaid => "top_paid",
            Self::ForBuilders => "for_builders",
            Self::ByCategory => "by_category",
            Self::Suites => "suites",
        }
    }

    fn fits_row(self) -> bool {
        matches!(
            self,
            Self::Trending | Self::New | Self::TopPaid | Self::ForBuilders | Self::Suites
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    All,
    Apps,
    Packages,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    App,
    Package,
    Collection,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Package => "package",
            Self::Collection => "collection",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Info,
    Launch,
    Maintenance,
    Warning,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Desktop,
    #[default]
    Web,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Web => "web",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlacementStatus {
    Draft,
    Scheduled,
    Live,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CollectionSource {
    Hand,
    Rule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleSort {
    Installs,
    Rating,
    Newest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PriceFilter {
    Free,
    Paid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SlotArea {
    Grid,
    Row,
    Unplaced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    Moved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchedVia {
    Name,
    Collection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FacetKind {
    App,
    Package,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Draft,
    Scheduled,
    Ended,
    Audience,
    DevOnly,
    TooFewItems,
    Empty,
}

/// Look of an item: `Auto` keeps the item's own category color, `Category(name)` borrows the color of a core
/// `AppCategory` (serde name, e.g. "Finance"). The wire form is one string: "auto" or "category:<AppCategory>".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Accent {
    Auto,
    Category(String),
}

impl TryFrom<String> for Accent {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == "auto" {
            return Ok(Self::Auto);
        }
        match value.strip_prefix("category:") {
            Some(name) if parse_app_category(name).is_some() => Ok(Self::Category(name.to_owned())),
            _ => Err(format!(
                "Unknown accent '{value}'; expected 'auto' or 'category:<AppCategory>'"
            )),
        }
    }
}

impl From<Accent> for String {
    fn from(value: Accent) -> Self {
        match value {
            Accent::Auto => "auto".to_owned(),
            Accent::Category(name) => format!("category:{name}"),
        }
    }
}

impl utoipa::PartialSchema for Accent {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .description(Some(
                "\"auto\" keeps the item's own category color; \"category:<AppCategory>\" uses that app category's color.",
            ))
            .examples(["category:Finance"])
            .into()
    }
}

impl ToSchema for Accent {}

fn default_rotation() -> u8 {
    ROTATION_DEFAULT
}

/// Kind-specific payload of a placement. utoipa-gen 5.4 ignores `rename_all_fields`, so every struct variant
/// carries its own `rename_all`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlacementContent {
    #[serde(rename_all = "camelCase")]
    Announcement {
        tone: Tone,
        title: String,
        body: String,
        cta_label: Option<String>,
        cta_href: Option<String>,
        image_url: Option<String>,
        dismissible: bool,
    },
    #[serde(rename_all = "camelCase")]
    Spotlight {
        #[serde(default = "default_rotation")]
        rotation_seconds: u8,
        /// Tops the spotlight up to three slides from the view's trending rail.
        #[serde(default)]
        auto_fill: bool,
    },
    #[serde(rename_all = "camelCase")]
    Feature { eyebrow: Option<String> },
    #[serde(rename_all = "camelCase")]
    Collection {
        title: String,
        blurb: Option<String>,
        source: CollectionSource,
        rule: Option<CollectionRule>,
    },
    #[serde(rename_all = "camelCase")]
    Rail {
        rail: RailKey,
        title: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Sponsored { advertiser: String },
}

impl PlacementContent {
    pub fn kind(&self) -> PlacementKind {
        match self {
            Self::Announcement { .. } => PlacementKind::Announcement,
            Self::Spotlight { .. } => PlacementKind::Spotlight,
            Self::Feature { .. } => PlacementKind::Feature,
            Self::Collection { .. } => PlacementKind::Collection,
            Self::Rail { .. } => PlacementKind::Rail,
            Self::Sponsored { .. } => PlacementKind::Sponsored,
        }
    }

    pub fn rail(&self) -> Option<RailKey> {
        match self {
            Self::Rail { rail, .. } => Some(*rail),
            _ => None,
        }
    }

    /// "rail/trending" for rails, the kind name otherwise; used in validation messages.
    pub fn label(&self) -> String {
        match self.rail() {
            Some(rail) => format!("rail/{}", rail.as_str()),
            None => self.kind().as_str().to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionRule {
    pub item_kind: ItemKind,
    /// Core `AppCategory` serde name, e.g. "Business".
    pub app_category: Option<String>,
    /// Core `AppType` serde name, e.g. "Agent".
    pub app_type: Option<String>,
    /// Manifest package category, e.g. "ANALYTICS_REPORTING".
    pub package_category: Option<String>,
    #[serde(default)]
    pub verified_only: bool,
    pub min_rating: Option<f32>,
    pub price: Option<PriceFilter>,
    pub sort: RuleSort,
    pub limit: u8,
}

impl CollectionRule {
    /// Trims the filters and drops the ones the rule's item kind ignores, as the admin client's
    /// `normalizeContent` does.
    fn normalize(&mut self) {
        trim_option(&mut self.app_category);
        trim_option(&mut self.app_type);
        trim_option(&mut self.package_category);
        match self.item_kind {
            ItemKind::App => {
                self.package_category = None;
                self.verified_only = false;
            }
            ItemKind::Package => {
                self.app_category = None;
                self.app_type = None;
            }
            ItemKind::Collection => {}
        }
    }
}

/// Storage shape of the `ExplorePlacementItem.overrides` column; never part of an API body.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ItemOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<Accent>,
}

/// One item of a placement with its admin copy and look overrides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlacementItemDoc {
    pub kind: ItemKind,
    pub id: String,
    #[serde(default)]
    pub headline: Option<String>,
    #[serde(default)]
    pub subline: Option<String>,
    #[serde(default)]
    pub artwork_url: Option<String>,
    #[serde(default)]
    pub accent: Option<Accent>,
}

impl PlacementItemDoc {
    pub fn overrides(&self) -> ItemOverrides {
        ItemOverrides {
            headline: self.headline.clone(),
            subline: self.subline.clone(),
            artwork_url: self.artwork_url.clone(),
            accent: self.accent.clone(),
        }
    }

    pub fn from_row(kind: ItemKind, id: String, overrides: ItemOverrides) -> Self {
        Self {
            kind,
            id,
            headline: overrides.headline,
            subline: overrides.subline,
            artwork_url: overrides.artwork_url,
            accent: overrides.accent,
        }
    }

    fn key(&self) -> (ItemKind, &str) {
        (self.kind, self.id.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlacementDoc {
    pub id: String,
    /// Always `content.kind`.
    pub kind: PlacementKind,
    pub name: String,
    pub enabled: bool,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub audience: Vec<String>,
    pub content: PlacementContent,
    pub items: Vec<PlacementItemDoc>,
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub status: Option<PlacementStatus>,
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    /// Storage only: kept when an edition is copied, never on the wire.
    #[serde(skip)]
    pub created_at: Option<DateTime<Utc>>,
}

impl PlacementDoc {
    pub fn status_at(&self, now: DateTime<Utc>) -> PlacementStatus {
        placement_status(self.enabled, self.starts_at, self.ends_at, now)
    }

    pub fn to_input(&self) -> PlacementInput {
        PlacementInput {
            name: self.name.clone(),
            enabled: self.enabled,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            audience: self.audience.clone(),
            content: self.content.clone(),
            items: self.items.clone(),
        }
    }

    /// Same curated content, ignoring where the placement sits and the output-only fields.
    pub fn same_content(&self, other: &Self) -> bool {
        self.name == other.name
            && self.enabled == other.enabled
            && self.starts_at == other.starts_at
            && self.ends_at == other.ends_at
            && self.audience == other.audience
            && self.content == other.content
            && self.items == other.items
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlotDoc {
    pub key: String,
    pub area: SlotArea,
    pub position: i32,
    /// Priority order: the first placement that qualifies for a viewer wins.
    pub placements: Vec<PlacementDoc>,
}

/// Grid slots first (fixed order), then rows by position, then `unplaced`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LayoutDoc {
    pub slots: Vec<SlotDoc>,
}

impl LayoutDoc {
    pub fn slot(&self, key: &str) -> Option<&SlotDoc> {
        self.slots.iter().find(|slot| slot.key == key)
    }

    pub fn placements(&self) -> impl Iterator<Item = (&SlotDoc, &PlacementDoc)> {
        self.slots.iter().flat_map(|slot| {
            slot.placements
                .iter()
                .map(move |placement| (slot, placement))
        })
    }

    pub fn find(&self, id: &str) -> Option<(&SlotDoc, &PlacementDoc)> {
        self.placements().find(|(_, placement)| placement.id == id)
    }

    pub fn placement_count(&self) -> usize {
        self.slots.iter().map(|slot| slot.placements.len()).sum()
    }

    pub fn rows(&self) -> impl Iterator<Item = &SlotDoc> {
        self.slots.iter().filter(|slot| slot.area == SlotArea::Row)
    }

    pub fn collection_ids(&self) -> HashSet<String> {
        self.placements()
            .filter(|(_, placement)| placement.kind == PlacementKind::Collection)
            .map(|(_, placement)| placement.id.clone())
            .collect()
    }

    /// Placements whose items point at the COLLECTION placement `collection_id`.
    pub fn referencing(&self, collection_id: &str) -> Vec<&PlacementDoc> {
        self.placements()
            .map(|(_, placement)| placement)
            .filter(|placement| {
                placement
                    .items
                    .iter()
                    .any(|item| item.kind == ItemKind::Collection && item.id == collection_id)
            })
            .collect()
    }

    pub fn ensure_room_for_placement(&self) -> Result<(), ApiError> {
        if self.placement_count() >= PLACEMENTS_MAX {
            return Err(ApiError::bad_request(format!(
                "An Explore layout holds at most {PLACEMENTS_MAX} placements; delete one first"
            )));
        }
        Ok(())
    }

    /// A slot a new or edited placement may go into: one this layout has (rows are created through the
    /// order endpoint first) and whose §2.3 rules accept `content`.
    pub fn check_target(&self, slot_key: &str, content: &PlacementContent) -> Result<(), ApiError> {
        check_slot(slot_key, content)?;
        if self.slot(slot_key).is_none() {
            return Err(ApiError::bad_request(format!(
                "Slot {slot_key} is not part of the layout; create the row first"
            )));
        }
        Ok(())
    }

    pub fn fill_statuses(&mut self, now: DateTime<Utc>) {
        for slot in &mut self.slots {
            for placement in &mut slot.placements {
                placement.status = Some(placement.status_at(now));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewer {
    pub dev: bool,
    pub signed_in: bool,
    pub platform: Platform,
    pub language: String,
}

/// Create/update body of a placement. There is no `kind`: it is always `content.kind()`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlacementInput {
    pub name: String,
    pub enabled: bool,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub audience: Vec<String>,
    pub content: PlacementContent,
    pub items: Vec<PlacementItemDoc>,
}

impl PlacementInput {
    /// Trims the copy, drops empty optional fields and checks every §3.2 rule except slot fit.
    /// `collection_ids` are the COLLECTION placements of the edition that spotlight items may reference.
    pub fn validated(mut self, collection_ids: &HashSet<String>) -> Result<Self, ApiError> {
        self.normalize();
        check_text(&self.name, "name", NAME_MAX, true)?;
        audience::validate(&self.audience)?;
        if let (Some(starts_at), Some(ends_at)) = (self.starts_at, self.ends_at)
            && starts_at >= ends_at
        {
            return Err(ApiError::bad_request(format!(
                "startsAt ({starts_at}) must be before endsAt ({ends_at}) in {}",
                self.name
            )));
        }
        self.check_content()?;
        self.check_items(collection_ids)?;
        Ok(self)
    }

    pub fn into_doc(self, id: String) -> PlacementDoc {
        PlacementDoc {
            id,
            kind: self.content.kind(),
            name: self.name,
            enabled: self.enabled,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            audience: self.audience,
            content: self.content,
            items: self.items,
            status: None,
            updated_at: None,
            created_at: None,
        }
    }

    fn normalize(&mut self) {
        trim(&mut self.name);
        audience::normalize(&mut self.audience);
        for item in &mut self.items {
            trim(&mut item.id);
            trim_option(&mut item.headline);
            trim_option(&mut item.subline);
            trim_option(&mut item.artwork_url);
        }
        match &mut self.content {
            PlacementContent::Announcement {
                title,
                body,
                cta_label,
                cta_href,
                image_url,
                ..
            } => {
                trim(title);
                trim(body);
                trim_option(cta_label);
                trim_option(cta_href);
                trim_option(image_url);
            }
            PlacementContent::Feature { eyebrow } => trim_option(eyebrow),
            PlacementContent::Collection {
                title,
                blurb,
                source,
                rule,
            } => {
                trim(title);
                trim_option(blurb);
                if *source == CollectionSource::Hand {
                    *rule = None;
                }
                if let Some(rule) = rule {
                    rule.normalize();
                }
            }
            PlacementContent::Rail { title, .. } => trim_option(title),
            PlacementContent::Sponsored { advertiser } => trim(advertiser),
            PlacementContent::Spotlight { .. } => {}
        }
    }

    fn check_content(&self) -> Result<(), ApiError> {
        match &self.content {
            PlacementContent::Announcement {
                title,
                body,
                cta_label,
                cta_href,
                image_url,
                ..
            } => {
                check_text(title, "title", ANNOUNCEMENT_TITLE_MAX, true)?;
                check_text(body, "body", ANNOUNCEMENT_BODY_MAX, false)?;
                check_optional_text(cta_label, "ctaLabel", CTA_LABEL_MAX)?;
                if cta_label.is_some() != cta_href.is_some() {
                    return Err(ApiError::bad_request(format!(
                        "ctaLabel and ctaHref must be set together in {}",
                        self.name
                    )));
                }
                if let Some(href) = cta_href {
                    check_cta_href(href)?;
                }
                if let Some(url) = image_url {
                    check_https_url(url, "imageUrl")?;
                }
            }
            PlacementContent::Spotlight {
                rotation_seconds, ..
            } => {
                if !(ROTATION_MIN..=ROTATION_MAX).contains(rotation_seconds) {
                    return Err(ApiError::bad_request(format!(
                        "rotationSeconds must be between {ROTATION_MIN} and {ROTATION_MAX} (got {rotation_seconds})"
                    )));
                }
            }
            PlacementContent::Feature { eyebrow } => {
                check_optional_text(eyebrow, "eyebrow", EYEBROW_MAX)?
            }
            PlacementContent::Collection {
                title,
                blurb,
                source,
                rule,
            } => {
                check_text(title, "title", COLLECTION_TITLE_MAX, true)?;
                check_optional_text(blurb, "blurb", BLURB_MAX)?;
                match (source, rule) {
                    (CollectionSource::Rule, Some(rule)) => check_rule(rule)?,
                    (CollectionSource::Rule, None) => {
                        return Err(ApiError::bad_request(
                            "A rule-based collection needs a rule",
                        ));
                    }
                    (CollectionSource::Hand, _) => {}
                }
            }
            PlacementContent::Rail { title, .. } => {
                check_optional_text(title, "title", RAIL_TITLE_MAX)?
            }
            PlacementContent::Sponsored { advertiser } => {
                check_text(advertiser, "advertiser", ADVERTISER_MAX, true)?;
                if self.enabled {
                    return Err(ApiError::bad_request(
                        "Sponsored placements cannot be published yet",
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_items(&self, collection_ids: &HashSet<String>) -> Result<(), ApiError> {
        const PICKS: &[ItemKind] = &[ItemKind::App, ItemKind::Package];
        const SLIDES: &[ItemKind] = &[ItemKind::App, ItemKind::Package, ItemKind::Collection];
        let (min, max, allowed): (usize, usize, &[ItemKind]) = match &self.content {
            PlacementContent::Spotlight { auto_fill, .. } => {
                (usize::from(!auto_fill), SPOTLIGHT_ITEMS_MAX, SLIDES)
            }
            PlacementContent::Feature { .. } | PlacementContent::Sponsored { .. } => (1, 1, PICKS),
            PlacementContent::Collection {
                source: CollectionSource::Hand,
                ..
            } => (COLLECTION_ITEMS_MIN, COLLECTION_ITEMS_MAX, PICKS),
            PlacementContent::Collection {
                source: CollectionSource::Rule,
                ..
            } => (0, RULE_PINNED_MAX, PICKS),
            PlacementContent::Announcement { .. } | PlacementContent::Rail { .. } => (0, 0, PICKS),
        };
        let count = self.items.len();
        if !(min..=max).contains(&count) {
            let expected = if min == max {
                format!("exactly {min}")
            } else {
                format!("between {min} and {max}")
            };
            return Err(ApiError::bad_request(format!(
                "A {} placement takes {expected} items (got {count}) in {}",
                self.content.label(),
                self.name
            )));
        }
        let mut seen = HashSet::new();
        for item in &self.items {
            if !allowed.contains(&item.kind) {
                return Err(ApiError::bad_request(format!(
                    "A {} placement cannot hold {} items ({})",
                    self.content.label(),
                    item.kind.as_str(),
                    item.id
                )));
            }
            if item.id.is_empty()
                || item.id.chars().count() > ITEM_ID_MAX
                || item.id.chars().any(char::is_control)
            {
                return Err(ApiError::bad_request(format!(
                    "Item ids must contain 1 to {ITEM_ID_MAX} characters (got '{}')",
                    item.id
                )));
            }
            if !seen.insert(item.key()) {
                return Err(ApiError::bad_request(format!(
                    "Item {}:{} appears twice in {}",
                    item.kind.as_str(),
                    item.id,
                    self.name
                )));
            }
            if item.kind == ItemKind::Collection && !collection_ids.contains(&item.id) {
                return Err(ApiError::bad_request(format!(
                    "Item collection:{} does not name a collection placement of this layout",
                    item.id
                )));
            }
            check_optional_text(&item.headline, "headline", HEADLINE_MAX)?;
            check_optional_text(&item.subline, "subline", SUBLINE_MAX)?;
            if let Some(url) = &item.artwork_url {
                check_https_url(url, "artworkUrl")?;
            }
        }
        Ok(())
    }
}

fn check_rule(rule: &CollectionRule) -> Result<(), ApiError> {
    if rule.item_kind == ItemKind::Collection {
        return Err(ApiError::bad_request(
            "Collection rules select apps or packages, not collections",
        ));
    }
    if let Some(name) = &rule.app_category
        && parse_app_category(name).is_none()
    {
        return Err(ApiError::bad_request(format!(
            "appCategory '{name}' is not a known app category"
        )));
    }
    if let Some(name) = &rule.app_type
        && serde_json::from_value::<AppType>(Value::String(name.clone())).is_err()
    {
        return Err(ApiError::bad_request(format!(
            "appType '{name}' is not a known app type"
        )));
    }
    if let Some(name) = &rule.package_category
        && WasmPackageCategory::from_str_opt(name).is_none()
    {
        return Err(ApiError::bad_request(format!(
            "packageCategory '{name}' is not a known package category"
        )));
    }
    if let Some(rating) = rule.min_rating
        && !(0.0..=RATING_MAX).contains(&rating)
    {
        return Err(ApiError::bad_request(format!(
            "minRating must be between 0 and {RATING_MAX} (got {rating})"
        )));
    }
    if !(RULE_LIMIT_MIN..=RULE_LIMIT_MAX).contains(&rule.limit) {
        return Err(ApiError::bad_request(format!(
            "The rule limit must be between {RULE_LIMIT_MIN} and {RULE_LIMIT_MAX} (got {})",
            rule.limit
        )));
    }
    Ok(())
}

fn trim(value: &mut String) {
    let trimmed = value.trim();
    if trimmed.len() != value.len() {
        *value = trimmed.to_owned();
    }
}

fn trim_option(value: &mut Option<String>) {
    if let Some(text) = value {
        trim(text);
        if text.is_empty() {
            *value = None;
        }
    }
}

fn check_text(value: &str, label: &str, max: usize, required: bool) -> Result<(), ApiError> {
    if required && value.is_empty() {
        return Err(ApiError::bad_request(format!("{label} is required")));
    }
    let count = value.chars().count();
    if count > max {
        return Err(ApiError::bad_request(format!(
            "{label} must be at most {max} characters (got {count})"
        )));
    }
    if value.contains('\0') {
        return Err(ApiError::bad_request(format!(
            "{label} must not contain NUL characters"
        )));
    }
    Ok(())
}

fn check_optional_text(value: &Option<String>, label: &str, max: usize) -> Result<(), ApiError> {
    value
        .as_deref()
        .map_or(Ok(()), |text| check_text(text, label, max, false))
}

fn is_https_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| !host.is_empty())
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn check_https_url(value: &str, label: &str) -> Result<(), ApiError> {
    if value.chars().count() > URL_MAX
        || value.chars().any(char::is_control)
        || !is_https_url(value)
    {
        return Err(ApiError::bad_request(format!(
            "{label} must be an https:// URL without credentials and at most {URL_MAX} characters (got '{value}')"
        )));
    }
    Ok(())
}

fn check_cta_href(value: &str) -> Result<(), ApiError> {
    let relative = value.starts_with('/')
        && !value.starts_with("//")
        && !value.starts_with("/\\")
        && !value.chars().any(char::is_whitespace);
    if value.chars().count() > URL_MAX
        || value.chars().any(char::is_control)
        || !(relative || is_https_url(value))
    {
        return Err(ApiError::bad_request(format!(
            "ctaHref must be an app path starting with / (not //) or an https:// URL without credentials, at most {URL_MAX} characters (got '{value}')"
        )));
    }
    Ok(())
}

pub fn placement_status(
    enabled: bool,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> PlacementStatus {
    if !enabled {
        PlacementStatus::Draft
    } else if starts_at.is_some_and(|starts_at| now < starts_at) {
        PlacementStatus::Scheduled
    } else if ends_at.is_some_and(|ends_at| ends_at <= now) {
        PlacementStatus::Ended
    } else {
        PlacementStatus::Live
    }
}

pub fn is_row_key(key: &str) -> bool {
    key.strip_prefix(ROW_PREFIX).is_some_and(|slug| {
        (1..=ROW_SLUG_MAX).contains(&slug.len())
            && slug
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    })
}

pub fn slot_area(key: &str) -> Option<SlotArea> {
    if GRID_SLOTS.contains(&key) {
        Some(SlotArea::Grid)
    } else if key == SLOT_UNPLACED {
        Some(SlotArea::Unplaced)
    } else if is_row_key(key) {
        Some(SlotArea::Row)
    } else {
        None
    }
}

/// §2.3 compatibility: decided by the content kind and, for rails, the rail key.
pub fn slot_accepts(slot_key: &str, content: &PlacementContent) -> bool {
    match slot_key {
        SLOT_HERO => matches!(content, PlacementContent::Spotlight { .. }),
        SLOT_NOTICE => matches!(content, PlacementContent::Announcement { .. }),
        SLOT_FEATURE => matches!(content, PlacementContent::Feature { .. }),
        SLOT_COLLECTION => matches!(content, PlacementContent::Collection { .. }),
        SLOT_STAT => content.rail() == Some(RailKey::NewCount),
        SLOT_CATEGORIES => content.rail() == Some(RailKey::ByCategory),
        SLOT_UNPLACED => true,
        key if is_row_key(key) => match content {
            PlacementContent::Collection { .. } => true,
            PlacementContent::Rail { rail, .. } => rail.fits_row(),
            _ => false,
        },
        _ => false,
    }
}

pub fn check_slot(slot_key: &str, content: &PlacementContent) -> Result<(), ApiError> {
    if slot_area(slot_key).is_none() {
        return Err(ApiError::bad_request(format!(
            "Unknown Explore slot '{slot_key}'"
        )));
    }
    if !slot_accepts(slot_key, content) {
        return Err(ApiError::bad_request(format!(
            "{slot_key} does not accept {}",
            content.label()
        )));
    }
    Ok(())
}

pub fn ensure_kind_unchanged(
    stored: PlacementKind,
    content: &PlacementContent,
) -> Result<(), ApiError> {
    let got = content.kind();
    if got != stored {
        return Err(ApiError::bad_request(format!(
            "Placement kind cannot change (stored {}, got {}); duplicate it instead",
            stored.as_str(),
            got.as_str()
        )));
    }
    Ok(())
}

/// Every rule a publish enforces over a whole edition: slot keys, slot fit, stored kinds, counts and each
/// placement's own validation.
pub fn validate_layout(layout: &LayoutDoc) -> Result<(), ApiError> {
    let mut slot_keys = HashSet::new();
    for slot in &layout.slots {
        if slot_area(&slot.key) != Some(slot.area) {
            return Err(ApiError::bad_request(format!(
                "Slot '{}' is not a valid {:?} slot",
                slot.key, slot.area
            )));
        }
        if !slot_keys.insert(slot.key.as_str()) {
            return Err(ApiError::bad_request(format!(
                "Slot {} appears twice",
                slot.key
            )));
        }
    }
    let rows = layout.rows().count();
    if rows > ROWS_MAX {
        return Err(ApiError::bad_request(format!(
            "An Explore layout holds at most {ROWS_MAX} rows (got {rows})"
        )));
    }
    let count = layout.placement_count();
    if count > PLACEMENTS_MAX {
        return Err(ApiError::bad_request(format!(
            "An Explore layout holds at most {PLACEMENTS_MAX} placements (got {count})"
        )));
    }
    let collection_ids = layout.collection_ids();
    let mut ids = HashSet::new();
    for (slot, placement) in layout.placements() {
        if !ids.insert(placement.id.as_str()) {
            return Err(ApiError::bad_request(format!(
                "Placement {} appears twice",
                placement.id
            )));
        }
        ensure_kind_unchanged(placement.kind, &placement.content)?;
        check_slot(&slot.key, &placement.content)?;
        placement.to_input().validated(&collection_ids)?;
    }
    Ok(())
}

/// Change of the editor's draft relative to the live edition, one per placement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExploreChange {
    pub placement_id: String,
    pub name: String,
    pub change: ChangeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemRef {
    pub kind: ItemKind,
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub cover_url: Option<String>,
    pub public: bool,
    pub exists: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExploreEditorState {
    /// "default" until the first draft write.
    pub draft_revision: String,
    pub live_revision: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
    pub layout: LayoutDoc,
    pub refs: Vec<ItemRef>,
    pub changes: Vec<ExploreChange>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkippedPlacement {
    pub placement_id: String,
    pub reason: SkipReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlotTrace {
    pub slot_key: String,
    pub chosen: Option<String>,
    pub skipped: Vec<SkippedPlacement>,
}

/// `PUT /admin/explore/order`: the complete row order plus the full priority list of every touched slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreOrderBody {
    pub expected_revision: String,
    pub rows: Vec<String>,
    pub slots: Vec<ExploreSlotOrder>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreSlotOrder {
    pub key: String,
    pub placement_ids: Vec<String>,
}

pub fn parse_app_category(name: &str) -> Option<AppCategory> {
    serde_json::from_value(Value::String(name.to_owned())).ok()
}

pub fn app_category_name(category: &AppCategory) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

macro_rules! package_category_pairs {
    ($($variant:ident),* $(,)?) => {
        #[cfg(test)]
        pub const ALL_PACKAGE_CATEGORIES: &[WasmPackageCategory] = &[$(WasmPackageCategory::$variant),*];

        pub fn package_category_to_db(category: WasmPackageCategory) -> DbWasmPackageCategory {
            match category {
                $(WasmPackageCategory::$variant => DbWasmPackageCategory::$variant,)*
            }
        }

        pub fn package_category_from_db(category: &DbWasmPackageCategory) -> WasmPackageCategory {
            match category {
                $(DbWasmPackageCategory::$variant => WasmPackageCategory::$variant,)*
            }
        }
    };
}

package_category_pairs!(
    DocumentProcessing,
    DataTransformation,
    WorkflowAutomation,
    Communication,
    AnalyticsReporting,
    FinanceBilling,
    ComplianceRegulatory,
    HrPeople,
    AiMl,
    IntegrationConnectors,
    SecurityIdentity,
    Devops,
    IotIndustrial,
    RoboticsPhysicalAi,
    GamingSimulation,
    Healthcare,
    Veterinary,
    Legal,
    Manufacturing,
    Agriculture,
    RealEstate,
    Logistics,
    Energy,
    ConstructionTrades,
    Education,
    GovernmentDefense,
    Ecommerce,
    Insurance,
    Telecom,
    ScientificEngineering,
    Geospatial,
    MediaContent,
    Other,
);

/// SCREAMING_SNAKE package category from either the manifest spelling or the entity's PascalCase one.
pub fn normalize_package_category(value: &str) -> Option<String> {
    WasmPackageCategory::from_str_opt(value)
        .or_else(|| {
            serde_json::from_value::<DbWasmPackageCategory>(Value::String(value.to_owned()))
                .ok()
                .map(|category| package_category_from_db(&category))
        })
        .map(|category| category.to_string())
}

/// Package categories an app category expands to when Browse mixes apps and packages.
pub fn package_categories_for(app: &AppCategory) -> &'static [WasmPackageCategory] {
    use WasmPackageCategory as P;
    match app {
        AppCategory::Business => &[P::AnalyticsReporting, P::IntegrationConnectors],
        AppCategory::Finance => &[P::FinanceBilling, P::Insurance],
        AppCategory::Productivity => &[P::WorkflowAutomation, P::DocumentProcessing],
        AppCategory::Education => &[P::Education],
        AppCategory::Communication => &[P::Communication],
        AppCategory::Utilities => &[P::DataTransformation, P::Devops],
        AppCategory::Health => &[P::Healthcare],
        AppCategory::Shopping => &[P::Ecommerce],
        AppCategory::Games => &[P::GamingSimulation],
        AppCategory::News
        | AppCategory::Photography
        | AppCategory::Music
        | AppCategory::Entertainment => &[P::MediaContent],
        AppCategory::Other
        | AppCategory::Social
        | AppCategory::Lifestyle
        | AppCategory::Travel
        | AppCategory::Sports
        | AppCategory::FoodAndDrink
        | AppCategory::Weather
        | AppCategory::Anime => &[],
    }
}

/// Inverse of [`package_categories_for`], as core `AppCategory` serde names.
pub fn app_categories_for(package: &WasmPackageCategory) -> &'static [&'static str] {
    use WasmPackageCategory as P;
    match package {
        P::AnalyticsReporting | P::IntegrationConnectors => &["Business"],
        P::FinanceBilling | P::Insurance => &["Finance"],
        P::WorkflowAutomation | P::DocumentProcessing => &["Productivity"],
        P::Education => &["Education"],
        P::Communication => &["Communication"],
        P::DataTransformation | P::Devops => &["Utilities"],
        P::Healthcare => &["Health"],
        P::Ecommerce => &["Shopping"],
        P::GamingSimulation => &["Games"],
        P::MediaContent => &["News", "Photography", "Music", "Entertainment"],
        P::ComplianceRegulatory
        | P::HrPeople
        | P::AiMl
        | P::SecurityIdentity
        | P::IotIndustrial
        | P::RoboticsPhysicalAi
        | P::Veterinary
        | P::Legal
        | P::Manufacturing
        | P::Agriculture
        | P::RealEstate
        | P::Logistics
        | P::Energy
        | P::ConstructionTrades
        | P::GovernmentDefense
        | P::Telecom
        | P::ScientificEngineering
        | P::Geospatial
        | P::Other => &[],
    }
}

/// One `categories` search value: `app:<AppCategory>` or `package:<PACKAGE_CATEGORY>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CategoryFilter {
    App(String),
    Package(WasmPackageCategory),
}

impl CategoryFilter {
    pub fn parse(value: &str) -> Option<Self> {
        if let Some(name) = value.strip_prefix("app:") {
            return parse_app_category(name).map(|_| Self::App(name.to_owned()));
        }
        value
            .strip_prefix("package:")
            .and_then(WasmPackageCategory::from_str_opt)
            .map(Self::Package)
    }

    /// The facet value this filter round-trips through.
    pub fn value(&self) -> String {
        match self {
            Self::App(name) => format!("app:{name}"),
            Self::Package(category) => format!("package:{category}"),
        }
    }

    pub fn app_category(&self) -> Option<AppCategory> {
        match self {
            Self::App(name) => parse_app_category(name),
            Self::Package(_) => None,
        }
    }
}

/// Permission facet group derived from a package's capability tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionGroup {
    #[serde(rename = "none")]
    NoPermissions,
    Network,
    Models,
    Storage,
}

impl PermissionGroup {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::NoPermissions),
            "network" => Some(Self::Network),
            "models" => Some(Self::Models),
            "storage" => Some(Self::Storage),
            _ => None,
        }
    }

    pub fn matches(self, capabilities: &[String]) -> bool {
        match self {
            Self::NoPermissions => capabilities.is_empty(),
            Self::Network => capabilities
                .iter()
                .any(|tag| tag.starts_with("net.") || tag == "widget.net"),
            Self::Models => capabilities.iter().any(|tag| tag == "models"),
            Self::Storage => capabilities.iter().any(|tag| tag.starts_with("storage.")),
        }
    }
}

fn parse_list<T: PartialEq>(
    raw: Option<&str>,
    param: &str,
    max: usize,
    expected: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Vec<T>, ApiError> {
    let values: Vec<&str> = raw
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();
    if values.len() > max {
        return Err(ApiError::bad_request(format!(
            "{param} accepts at most {max} values, got {}",
            values.len()
        )));
    }
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let item = parse(value).ok_or_else(|| {
            ApiError::bad_request(format!(
                "{param} value '{value}' is not supported; expected {expected}"
            ))
        })?;
        if !parsed.contains(&item) {
            parsed.push(item);
        }
    }
    Ok(parsed)
}

/// Comma-separated `categories` search parameter.
pub fn parse_categories(raw: Option<&str>) -> Result<Vec<CategoryFilter>, ApiError> {
    parse_list(
        raw,
        "categories",
        SEARCH_CATEGORIES_MAX,
        "app:<AppCategory> (e.g. app:Finance) or package:<PACKAGE_CATEGORY> (e.g. package:EDUCATION)",
        CategoryFilter::parse,
    )
}

/// Comma-separated `permissions` search parameter.
pub fn parse_permissions(raw: Option<&str>) -> Result<Vec<PermissionGroup>, ApiError> {
    parse_list(
        raw,
        "permissions",
        SEARCH_PERMISSIONS_MAX,
        "none, network, models or storage",
        PermissionGroup::parse,
    )
}

impl From<PlacementKind> for ExplorePlacementKind {
    fn from(value: PlacementKind) -> Self {
        match value {
            PlacementKind::Announcement => Self::Announcement,
            PlacementKind::Spotlight => Self::Spotlight,
            PlacementKind::Feature => Self::Feature,
            PlacementKind::Collection => Self::Collection,
            PlacementKind::Rail => Self::Rail,
            PlacementKind::Sponsored => Self::Sponsored,
        }
    }
}

impl From<ExplorePlacementKind> for PlacementKind {
    fn from(value: ExplorePlacementKind) -> Self {
        match value {
            ExplorePlacementKind::Announcement => Self::Announcement,
            ExplorePlacementKind::Spotlight => Self::Spotlight,
            ExplorePlacementKind::Feature => Self::Feature,
            ExplorePlacementKind::Collection => Self::Collection,
            ExplorePlacementKind::Rail => Self::Rail,
            ExplorePlacementKind::Sponsored => Self::Sponsored,
        }
    }
}

impl From<ItemKind> for ExploreItemKind {
    fn from(value: ItemKind) -> Self {
        match value {
            ItemKind::App => Self::App,
            ItemKind::Package => Self::Package,
            ItemKind::Collection => Self::Collection,
        }
    }
}

impl From<ExploreItemKind> for ItemKind {
    fn from(value: ExploreItemKind) -> Self {
        match value {
            ExploreItemKind::App => Self::App,
            ExploreItemKind::Package => Self::Package,
            ExploreItemKind::Collection => Self::Collection,
        }
    }
}

impl From<SlotArea> for ExploreSlotArea {
    fn from(value: SlotArea) -> Self {
        match value {
            SlotArea::Grid => Self::Grid,
            SlotArea::Row => Self::Row,
            SlotArea::Unplaced => Self::Unplaced,
        }
    }
}

impl From<ExploreSlotArea> for SlotArea {
    fn from(value: ExploreSlotArea) -> Self {
        match value {
            ExploreSlotArea::Grid => Self::Grid,
            ExploreSlotArea::Row => Self::Row,
            ExploreSlotArea::Unplaced => Self::Unplaced,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::Category as DbCategory;
    use axum::extract::Query;
    use sea_orm::{ActiveEnum, Iterable};
    use serde_json::json;

    fn message(error: ApiError) -> String {
        error.public_message().unwrap_or_default().to_owned()
    }

    fn rail(rail: RailKey) -> PlacementContent {
        PlacementContent::Rail { rail, title: None }
    }

    fn collection_content(source: CollectionSource) -> PlacementContent {
        PlacementContent::Collection {
            title: "Finance picks".into(),
            blurb: None,
            source,
            rule: (source == CollectionSource::Rule).then(|| CollectionRule {
                item_kind: ItemKind::Package,
                app_category: None,
                app_type: None,
                package_category: Some("FINANCE_BILLING".into()),
                verified_only: true,
                min_rating: Some(4.0),
                price: None,
                sort: RuleSort::Installs,
                limit: 6,
            }),
        }
    }

    fn announcement() -> PlacementContent {
        PlacementContent::Announcement {
            tone: Tone::Launch,
            title: "Packages are here".into(),
            body: "Extend flows with community nodes.".into(),
            cta_label: Some("Browse".into()),
            cta_href: Some("/store/explore?type=packages".into()),
            image_url: None,
            dismissible: true,
        }
    }

    fn item(kind: ItemKind, id: &str) -> PlacementItemDoc {
        PlacementItemDoc::from_row(kind, id.into(), ItemOverrides::default())
    }

    fn items(kind: ItemKind, count: usize) -> Vec<PlacementItemDoc> {
        (0..count).map(|i| item(kind, &format!("i{i}"))).collect()
    }

    fn input(content: PlacementContent, items: Vec<PlacementItemDoc>) -> PlacementInput {
        PlacementInput {
            name: "Placement".into(),
            enabled: true,
            starts_at: None,
            ends_at: None,
            audience: vec![],
            content,
            items,
        }
    }

    fn validate(input: PlacementInput) -> Result<PlacementInput, ApiError> {
        input.validated(&HashSet::from(["c1".to_owned()]))
    }

    fn all_contents() -> Vec<PlacementContent> {
        let mut contents = vec![
            announcement(),
            PlacementContent::Spotlight {
                rotation_seconds: 8,
                auto_fill: false,
            },
            PlacementContent::Feature { eyebrow: None },
            collection_content(CollectionSource::Hand),
            PlacementContent::Sponsored {
                advertiser: "Acme".into(),
            },
        ];
        contents.extend(
            [
                RailKey::Trending,
                RailKey::New,
                RailKey::NewCount,
                RailKey::TopPaid,
                RailKey::ForBuilders,
                RailKey::ByCategory,
                RailKey::Suites,
            ]
            .map(rail),
        );
        contents
    }

    #[test]
    fn placement_content_uses_kind_tag_and_camel_case_fields() {
        let value = serde_json::to_value(announcement()).unwrap();
        assert_eq!(value["kind"], "announcement");
        assert_eq!(value["ctaLabel"], "Browse");
        assert_eq!(value["ctaHref"], "/store/explore?type=packages");
        let spotlight: PlacementContent = serde_json::from_value(
            json!({"kind": "spotlight", "rotationSeconds": 12, "autoFill": true}),
        )
        .unwrap();
        assert_eq!(
            spotlight,
            PlacementContent::Spotlight {
                rotation_seconds: 12,
                auto_fill: true
            }
        );
        let defaulted: PlacementContent =
            serde_json::from_value(json!({"kind": "spotlight"})).unwrap();
        assert_eq!(
            defaulted,
            PlacementContent::Spotlight {
                rotation_seconds: ROTATION_DEFAULT,
                auto_fill: false
            }
        );
        assert!(
            serde_json::from_value::<PlacementContent>(
                json!({"kind": "spotlight", "rotation_seconds": 8})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<PlacementContent>(
                json!({"kind": "rail", "rail": "trending", "bogus": 1})
            )
            .is_err()
        );
        let rail: PlacementContent =
            serde_json::from_value(json!({"kind": "rail", "rail": "new_count"})).unwrap();
        assert_eq!(rail.kind(), PlacementKind::Rail);
        assert_eq!(rail.label(), "rail/new_count");
    }

    #[test]
    fn placement_content_schema_exposes_camel_case_fields() {
        let schema =
            serde_json::to_string(&<PlacementContent as utoipa::PartialSchema>::schema()).unwrap();
        for field in [
            "ctaLabel",
            "ctaHref",
            "imageUrl",
            "rotationSeconds",
            "autoFill",
        ] {
            assert!(
                schema.contains(field),
                "schema is missing {field}: {schema}"
            );
        }
        assert!(!schema.contains("cta_label"));
        let accent = serde_json::to_value(<Accent as utoipa::PartialSchema>::schema()).unwrap();
        assert_eq!(accent["type"], "string");
        let placement =
            serde_json::to_string(&<PlacementDoc as utoipa::PartialSchema>::schema()).unwrap();
        assert!(!placement.contains("createdAt"), "{placement}");
    }

    #[test]
    fn accent_round_trips_as_one_string() {
        for wire in [
            "\"auto\"",
            "\"category:Finance\"",
            "\"category:FoodAndDrink\"",
        ] {
            let accent: Accent = serde_json::from_str(wire).unwrap();
            assert_eq!(serde_json::to_string(&accent).unwrap(), wire);
        }
        assert_eq!(
            serde_json::from_str::<Accent>("\"category:Finance\"").unwrap(),
            Accent::Category("Finance".into())
        );
        for wire in [
            "\"category:Nope\"",
            "\"Finance\"",
            "\"category:\"",
            "{\"category\":\"Finance\"}",
        ] {
            assert!(serde_json::from_str::<Accent>(wire).is_err(), "{wire}");
        }
    }

    #[test]
    fn placement_items_are_strict_and_convert_to_overrides() {
        assert!(
            serde_json::from_value::<PlacementItemDoc>(
                json!({"kind": "app", "id": "a", "bogus": "h"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<PlacementItemDoc>(
                json!({"kind": "app", "id": "a", "artworkURL": "https://x.test/a.png"})
            )
            .is_err()
        );
        let doc: PlacementItemDoc = serde_json::from_value(json!({
            "kind": "package", "id": "p", "headline": "Invoices", "artworkUrl": "https://cdn.test/a.webp",
            "accent": "category:Finance"
        }))
        .unwrap();
        let overrides = doc.overrides();
        assert_eq!(overrides.accent, Some(Accent::Category("Finance".into())));
        let stored = serde_json::to_value(&overrides).unwrap();
        assert_eq!(
            stored,
            json!({"headline": "Invoices", "artworkUrl": "https://cdn.test/a.webp", "accent": "category:Finance"})
        );
        let restored: ItemOverrides = serde_json::from_value(stored).unwrap();
        assert_eq!(
            PlacementItemDoc::from_row(ItemKind::Package, "p".into(), restored),
            doc
        );
        assert_eq!(
            serde_json::from_value::<ItemOverrides>(json!({})).unwrap(),
            ItemOverrides::default()
        );
    }

    #[test]
    fn placement_input_has_no_kind_and_kind_cannot_change() {
        let body = json!({
            "name": "Hero", "enabled": true, "audience": [], "items": [],
            "content": {"kind": "spotlight", "rotationSeconds": 8, "autoFill": true}
        });
        let parsed: PlacementInput = serde_json::from_value(body.clone()).unwrap();
        assert!(
            !serde_json::to_value(&parsed)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("kind")
        );
        let mut with_kind = body;
        with_kind["kind"] = json!("spotlight");
        assert!(serde_json::from_value::<PlacementInput>(with_kind).is_err());
        assert!(
            ensure_kind_unchanged(
                PlacementKind::Collection,
                &collection_content(CollectionSource::Hand)
            )
            .is_ok()
        );
        let error =
            ensure_kind_unchanged(PlacementKind::Collection, &rail(RailKey::Trending)).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(
            message(error),
            "Placement kind cannot change (stored collection, got rail); duplicate it instead"
        );
        let doc = parsed.into_doc("p1".into());
        assert_eq!(doc.kind, PlacementKind::Spotlight);
    }

    #[test]
    fn slot_accepts_follows_the_compatibility_table() {
        let expectations: &[(&str, &[&str])] = &[
            (SLOT_HERO, &["spotlight"]),
            (SLOT_NOTICE, &["announcement"]),
            (SLOT_FEATURE, &["feature"]),
            (SLOT_COLLECTION, &["collection"]),
            (SLOT_STAT, &["rail/new_count"]),
            (SLOT_CATEGORIES, &["rail/by_category"]),
            (
                "row:x",
                &[
                    "collection",
                    "rail/trending",
                    "rail/new",
                    "rail/top_paid",
                    "rail/for_builders",
                    "rail/suites",
                ],
            ),
        ];
        for (slot, accepted) in expectations {
            for content in all_contents() {
                assert_eq!(
                    slot_accepts(slot, &content),
                    accepted.contains(&content.label().as_str()),
                    "{slot} vs {}",
                    content.label()
                );
            }
        }
        for content in all_contents() {
            assert!(slot_accepts(SLOT_UNPLACED, &content));
            assert!(!slot_accepts("row:", &content));
            assert!(!slot_accepts("row:Bad", &content));
            assert!(!slot_accepts("sidebar", &content));
        }
        assert!(!slot_accepts(SLOT_STAT, &rail(RailKey::Trending)));
        assert!(!slot_accepts("row:x", &rail(RailKey::ByCategory)));
        assert_eq!(
            message(check_slot(SLOT_STAT, &rail(RailKey::Trending)).unwrap_err()),
            "stat does not accept rail/trending"
        );
        assert!(is_row_key(&format!("row:{}", "a".repeat(ROW_SLUG_MAX))));
        assert!(!is_row_key(&format!(
            "row:{}",
            "a".repeat(ROW_SLUG_MAX + 1)
        )));
    }

    #[test]
    fn text_limits_hold_at_the_boundary_and_fail_one_past() {
        type Setter = fn(&mut PlacementInput, String);
        fn set_announcement(input: &mut PlacementInput, field: &str, value: String) {
            input.content = announcement();
            if let PlacementContent::Announcement {
                title,
                body,
                cta_label,
                ..
            } = &mut input.content
            {
                match field {
                    "title" => *title = value,
                    "body" => *body = value,
                    _ => *cta_label = Some(value),
                }
            }
        }
        let cases: Vec<(usize, Setter)> = vec![
            (NAME_MAX, |i, v| i.name = v),
            (ANNOUNCEMENT_TITLE_MAX, |i, v| {
                set_announcement(i, "title", v)
            }),
            (ANNOUNCEMENT_BODY_MAX, |i, v| set_announcement(i, "body", v)),
            (CTA_LABEL_MAX, |i, v| set_announcement(i, "cta", v)),
            (HEADLINE_MAX, |i, v| {
                i.content = PlacementContent::Feature { eyebrow: None };
                i.items = vec![PlacementItemDoc {
                    headline: Some(v),
                    ..item(ItemKind::App, "a")
                }];
            }),
            (SUBLINE_MAX, |i, v| {
                i.content = PlacementContent::Feature { eyebrow: None };
                i.items = vec![PlacementItemDoc {
                    subline: Some(v),
                    ..item(ItemKind::App, "a")
                }];
            }),
            (COLLECTION_TITLE_MAX, |i, v| {
                i.content = PlacementContent::Collection {
                    title: v,
                    blurb: None,
                    source: CollectionSource::Hand,
                    rule: None,
                };
                i.items = items(ItemKind::App, 2);
            }),
            (BLURB_MAX, |i, v| {
                i.content = PlacementContent::Collection {
                    title: "t".into(),
                    blurb: Some(v),
                    source: CollectionSource::Hand,
                    rule: None,
                };
                i.items = items(ItemKind::App, 2);
            }),
            (RAIL_TITLE_MAX, |i, v| {
                i.content = PlacementContent::Rail {
                    rail: RailKey::Trending,
                    title: Some(v),
                };
            }),
            (ADVERTISER_MAX, |i, v| {
                i.enabled = false;
                i.content = PlacementContent::Sponsored { advertiser: v };
                i.items = items(ItemKind::App, 1);
            }),
            (EYEBROW_MAX, |i, v| {
                i.content = PlacementContent::Feature { eyebrow: Some(v) };
                i.items = items(ItemKind::App, 1);
            }),
        ];
        for (max, set) in cases {
            let mut ok = input(rail(RailKey::Trending), vec![]);
            set(&mut ok, "é".repeat(max));
            validate(ok).unwrap_or_else(|e| panic!("limit {max}: {}", message(e)));
            let mut over = input(rail(RailKey::Trending), vec![]);
            set(&mut over, "é".repeat(max + 1));
            let error = message(validate(over).unwrap_err());
            assert!(
                error.contains(&format!("at most {max} characters")),
                "{error}"
            );
        }
    }

    #[test]
    fn item_counts_hold_at_the_boundary_and_fail_one_past() {
        let spotlight = |auto_fill| PlacementContent::Spotlight {
            rotation_seconds: 8,
            auto_fill,
        };
        let cases: Vec<(PlacementContent, ItemKind, usize, bool)> = vec![
            (spotlight(false), ItemKind::App, 1, true),
            (spotlight(false), ItemKind::App, 0, false),
            (spotlight(true), ItemKind::App, 0, true),
            (
                spotlight(false),
                ItemKind::Package,
                SPOTLIGHT_ITEMS_MAX,
                true,
            ),
            (
                spotlight(true),
                ItemKind::Package,
                SPOTLIGHT_ITEMS_MAX + 1,
                false,
            ),
            (
                PlacementContent::Feature { eyebrow: None },
                ItemKind::App,
                1,
                true,
            ),
            (
                PlacementContent::Feature { eyebrow: None },
                ItemKind::App,
                0,
                false,
            ),
            (
                PlacementContent::Feature { eyebrow: None },
                ItemKind::App,
                2,
                false,
            ),
            (
                PlacementContent::Feature { eyebrow: None },
                ItemKind::Collection,
                1,
                false,
            ),
            (
                collection_content(CollectionSource::Hand),
                ItemKind::App,
                COLLECTION_ITEMS_MIN,
                true,
            ),
            (
                collection_content(CollectionSource::Hand),
                ItemKind::App,
                COLLECTION_ITEMS_MIN - 1,
                false,
            ),
            (
                collection_content(CollectionSource::Hand),
                ItemKind::Package,
                COLLECTION_ITEMS_MAX,
                true,
            ),
            (
                collection_content(CollectionSource::Hand),
                ItemKind::Package,
                COLLECTION_ITEMS_MAX + 1,
                false,
            ),
            (
                collection_content(CollectionSource::Rule),
                ItemKind::App,
                0,
                true,
            ),
            (
                collection_content(CollectionSource::Rule),
                ItemKind::App,
                RULE_PINNED_MAX,
                true,
            ),
            (
                collection_content(CollectionSource::Rule),
                ItemKind::App,
                RULE_PINNED_MAX + 1,
                false,
            ),
            (rail(RailKey::Trending), ItemKind::App, 0, true),
            (rail(RailKey::Trending), ItemKind::App, 1, false),
            (announcement(), ItemKind::App, 1, false),
        ];
        for (content, kind, count, ok) in cases {
            let label = content.label();
            let result = validate(input(content, items(kind, count)));
            assert_eq!(result.is_ok(), ok, "{label} with {count} {kind:?}");
        }
        let sponsored = |enabled| PlacementInput {
            enabled,
            ..input(
                PlacementContent::Sponsored {
                    advertiser: "Acme".into(),
                },
                items(ItemKind::App, 1),
            )
        };
        assert!(validate(sponsored(false)).is_ok());
        assert_eq!(
            message(validate(sponsored(true)).unwrap_err()),
            "Sponsored placements cannot be published yet"
        );
    }

    #[test]
    fn item_ids_hold_at_the_boundary_and_fail_one_past() {
        let feature = |id: &str| {
            validate(input(
                PlacementContent::Feature { eyebrow: None },
                vec![item(ItemKind::App, id)],
            ))
        };
        assert!(feature(&"é".repeat(ITEM_ID_MAX)).is_ok());
        let over = "é".repeat(ITEM_ID_MAX + 1);
        assert_eq!(
            message(feature(&over).unwrap_err()),
            format!("Item ids must contain 1 to {ITEM_ID_MAX} characters (got '{over}')")
        );
        assert!(feature("  ").is_err());
    }

    fn capacity_layout(rows: usize, placements: usize) -> LayoutDoc {
        let rows = (0..rows).map(|i| SlotDoc {
            key: format!("row:r{i}"),
            area: SlotArea::Row,
            position: i as i32,
            placements: vec![],
        });
        let unplaced = SlotDoc {
            key: SLOT_UNPLACED.into(),
            area: SlotArea::Unplaced,
            position: 0,
            placements: (0..placements)
                .map(|i| input(rail(RailKey::New), vec![]).into_doc(format!("p{i}")))
                .collect(),
        };
        LayoutDoc {
            slots: rows.chain(std::iter::once(unplaced)).collect(),
        }
    }

    #[test]
    fn capacity_limits_hold_at_the_boundary_and_fail_one_past() {
        validate_layout(&capacity_layout(ROWS_MAX, PLACEMENTS_MAX)).unwrap();
        assert_eq!(
            message(validate_layout(&capacity_layout(ROWS_MAX + 1, 0)).unwrap_err()),
            format!(
                "An Explore layout holds at most {ROWS_MAX} rows (got {})",
                ROWS_MAX + 1
            )
        );
        assert_eq!(
            message(validate_layout(&capacity_layout(0, PLACEMENTS_MAX + 1)).unwrap_err()),
            format!(
                "An Explore layout holds at most {PLACEMENTS_MAX} placements (got {})",
                PLACEMENTS_MAX + 1
            )
        );
        assert!(
            capacity_layout(0, PLACEMENTS_MAX - 1)
                .ensure_room_for_placement()
                .is_ok()
        );
        assert_eq!(
            message(
                capacity_layout(0, PLACEMENTS_MAX)
                    .ensure_room_for_placement()
                    .unwrap_err()
            ),
            format!(
                "An Explore layout holds at most {PLACEMENTS_MAX} placements; delete one first"
            )
        );
    }

    #[test]
    fn numeric_limits_hold_at_the_boundary_and_fail_one_past() {
        for (seconds, ok) in [
            (ROTATION_MIN, true),
            (ROTATION_MAX, true),
            (ROTATION_MIN - 1, false),
            (ROTATION_MAX + 1, false),
        ] {
            let content = PlacementContent::Spotlight {
                rotation_seconds: seconds,
                auto_fill: true,
            };
            assert_eq!(validate(input(content, vec![])).is_ok(), ok, "{seconds}");
        }
        let with_rule = |edit: fn(&mut CollectionRule)| {
            let mut content = collection_content(CollectionSource::Rule);
            if let PlacementContent::Collection {
                rule: Some(rule), ..
            } = &mut content
            {
                edit(rule);
            }
            validate(input(content, vec![]))
        };
        assert!(with_rule(|r| r.min_rating = Some(0.0)).is_ok());
        assert!(with_rule(|r| r.min_rating = Some(5.0)).is_ok());
        assert!(with_rule(|r| r.min_rating = Some(-0.1)).is_err());
        assert!(with_rule(|r| r.min_rating = Some(5.1)).is_err());
        assert!(with_rule(|r| r.min_rating = Some(f32::NAN)).is_err());
        assert!(with_rule(|r| r.limit = RULE_LIMIT_MIN).is_ok());
        assert!(with_rule(|r| r.limit = RULE_LIMIT_MAX).is_ok());
        assert!(with_rule(|r| r.limit = RULE_LIMIT_MIN - 1).is_err());
        assert!(with_rule(|r| r.limit = RULE_LIMIT_MAX + 1).is_err());
        assert!(with_rule(|r| r.package_category = Some("Education".into())).is_err());
        assert!(with_rule(|r| r.item_kind = ItemKind::Collection).is_err());
        assert!(
            with_rule(|r| {
                r.item_kind = ItemKind::App;
                r.app_category = Some("Business".into());
                r.app_type = Some("Agent".into());
            })
            .is_ok()
        );
        assert!(
            with_rule(|r| {
                r.item_kind = ItemKind::App;
                r.app_category = Some("BUSINESS".into());
            })
            .is_err()
        );
        let missing_rule = PlacementContent::Collection {
            title: "t".into(),
            blurb: None,
            source: CollectionSource::Rule,
            rule: None,
        };
        assert!(validate(input(missing_rule, vec![])).is_err());
    }

    #[test]
    fn urls_and_hrefs_are_restricted() {
        let with_href = |href: &str| {
            let mut content = announcement();
            if let PlacementContent::Announcement { cta_href, .. } = &mut content {
                *cta_href = Some(href.into());
            }
            validate(input(content, vec![]))
        };
        assert!(with_href("/store/explore/search?q=invoice").is_ok());
        assert!(with_href("https://flow-like.com/blog").is_ok());
        for bad in [
            "//evil.test",
            "/\\evil.test",
            "javascript:alert(1)",
            "http://flow-like.com",
            "https://user:pw@flow-like.com",
            "/path with space",
            "store/explore",
            "https://flow-like.com/a\nb",
            "https://flow-like.com/\u{0}",
            "https://flow-like.com/a\tb",
            "/store\u{7f}",
        ] {
            assert!(with_href(bad).is_err(), "{bad:?}");
        }
        let url_of =
            |prefix: &str, length: usize| format!("{prefix}{}", "a".repeat(length - prefix.len()));
        const CDN: &str = "https://cdn.flow-like.com/explore/";
        for prefix in ["/", CDN] {
            assert!(with_href(&url_of(prefix, URL_MAX)).is_ok(), "{prefix}");
            assert!(with_href(&url_of(prefix, URL_MAX + 1)).is_err(), "{prefix}");
        }
        let cta = |label: Option<&str>, href: Option<&str>| {
            let mut content = announcement();
            if let PlacementContent::Announcement {
                cta_label,
                cta_href,
                ..
            } = &mut content
            {
                *cta_label = label.map(Into::into);
                *cta_href = href.map(Into::into);
            }
            validate(input(content, vec![]))
        };
        assert!(cta(None, None).is_ok());
        assert!(cta(Some(" "), Some("")).is_ok());
        for (label, href) in [
            (Some("Browse"), None),
            (None, Some("/store")),
            (Some("  "), Some("/store")),
        ] {
            assert_eq!(
                message(cta(label, href).unwrap_err()),
                "ctaLabel and ctaHref must be set together in Placement",
                "{label:?} {href:?}"
            );
        }
        let with_image = |url: &str| {
            let mut content = announcement();
            if let PlacementContent::Announcement { image_url, .. } = &mut content {
                *image_url = Some(url.into());
            }
            validate(input(content, vec![]))
        };
        assert!(with_image(&url_of(CDN, URL_MAX)).is_ok());
        assert!(
            message(with_image(&url_of(CDN, URL_MAX + 1)).unwrap_err()).starts_with("imageUrl")
        );
        assert!(with_image("http://cdn.flow-like.com/a.webp").is_err());
        let with_artwork = |url: &str| {
            validate(input(
                PlacementContent::Feature { eyebrow: None },
                vec![PlacementItemDoc {
                    artwork_url: Some(url.into()),
                    ..item(ItemKind::App, "a")
                }],
            ))
        };
        assert!(with_artwork("https://cdn.flow-like.com/explore/a.webp").is_ok());
        assert!(with_artwork(&url_of(CDN, URL_MAX)).is_ok());
        assert!(
            message(with_artwork(&url_of(CDN, URL_MAX + 1)).unwrap_err()).starts_with("artworkUrl")
        );
        assert!(with_artwork("http://cdn.flow-like.com/a.webp").is_err());
        assert!(with_artwork("/relative.webp").is_err());
        assert!(with_artwork("https://u:p@cdn.test/a.webp").is_err());
    }

    #[test]
    fn duplicate_items_collection_refs_and_windows_are_rejected() {
        let duplicate = input(
            collection_content(CollectionSource::Hand),
            vec![item(ItemKind::App, "a"), item(ItemKind::App, "a")],
        );
        assert_eq!(
            message(validate(duplicate).unwrap_err()),
            "Item app:a appears twice in Placement"
        );
        let same_id_other_kind = input(
            collection_content(CollectionSource::Hand),
            vec![item(ItemKind::App, "a"), item(ItemKind::Package, "a")],
        );
        assert!(validate(same_id_other_kind).is_ok());
        let spotlight = |id: &str| {
            input(
                PlacementContent::Spotlight {
                    rotation_seconds: 8,
                    auto_fill: false,
                },
                vec![item(ItemKind::Collection, id)],
            )
        };
        assert!(validate(spotlight("c1")).is_ok());
        assert!(validate(spotlight("missing")).is_err());
        let now = Utc::now();
        let window = |starts: i64, ends: i64| PlacementInput {
            starts_at: Some(now + chrono::Duration::hours(starts)),
            ends_at: Some(now + chrono::Duration::hours(ends)),
            ..input(rail(RailKey::New), vec![])
        };
        assert!(validate(window(0, 1)).is_ok());
        assert!(validate(window(1, 1)).is_err());
        assert!(validate(window(2, 1)).is_err());
    }

    #[test]
    fn validation_trims_copy_and_drops_inert_rule_fields() {
        let mut content = collection_content(CollectionSource::Rule);
        if let PlacementContent::Collection {
            title,
            blurb,
            rule: Some(rule),
            ..
        } = &mut content
        {
            *title = "  Finance  ".into();
            *blurb = Some("   ".into());
            rule.app_category = Some(" Finance ".into());
            rule.app_type = Some("NOT_A_TYPE".into());
            rule.package_category = Some(" FINANCE_BILLING ".into());
        }
        let validated = validate(PlacementInput {
            name: " Hero ".into(),
            audience: vec![" dev ".into(), "dev".into()],
            ..input(content, vec![])
        })
        .unwrap();
        assert_eq!(validated.name, "Hero");
        assert_eq!(validated.audience, ["dev"]);
        let rule_of = |content: PlacementContent| match content {
            PlacementContent::Collection {
                title, blurb, rule, ..
            } => (title, blurb, rule),
            _ => panic!("collection expected"),
        };
        let (title, blurb, rule) = rule_of(validated.content);
        assert_eq!(title, "Finance");
        assert_eq!(blurb, None);
        let rule = rule.unwrap();
        assert_eq!(rule.app_category, None);
        assert_eq!(rule.app_type, None);
        assert_eq!(rule.package_category.as_deref(), Some("FINANCE_BILLING"));
        assert!(rule.verified_only);

        let app_rule = PlacementContent::Collection {
            title: "Apps".into(),
            blurb: None,
            source: CollectionSource::Rule,
            rule: Some(CollectionRule {
                item_kind: ItemKind::App,
                app_category: Some("Finance".into()),
                package_category: Some("NOT_A_CATEGORY".into()),
                ..rule.clone()
            }),
        };
        let (_, _, app_rule) = rule_of(validate(input(app_rule, vec![])).unwrap().content);
        let app_rule = app_rule.unwrap();
        assert_eq!(app_rule.app_category.as_deref(), Some("Finance"));
        assert_eq!(app_rule.package_category, None);
        assert!(!app_rule.verified_only);

        let hand = PlacementContent::Collection {
            title: "Picks".into(),
            blurb: None,
            source: CollectionSource::Hand,
            rule: Some(CollectionRule {
                limit: 0,
                package_category: Some("x\0".repeat(500)),
                ..rule
            }),
        };
        let (_, _, hand_rule) = rule_of(
            validate(input(hand, items(ItemKind::App, 2)))
                .unwrap()
                .content,
        );
        assert_eq!(hand_rule, None);
        assert!(
            validate(PlacementInput {
                name: "   ".into(),
                ..input(rail(RailKey::New), vec![])
            })
            .is_err()
        );
    }

    #[test]
    fn audience_is_validated_with_the_placement() {
        let with = |tags: &[&str]| {
            validate(PlacementInput {
                audience: tags.iter().map(|tag| tag.to_string()).collect(),
                ..input(rail(RailKey::New), vec![])
            })
        };
        assert!(with(&["everyone"]).is_ok());
        assert!(with(&["dev", "desktop", "locale:de"]).is_ok());
        assert!(with(&["everyone", "dev"]).is_err());
        assert!(with(&["signed_in", "signed_out"]).is_err());
        assert!(with(&["vip"]).is_err());
    }

    #[test]
    fn placement_status_boundaries() {
        let now = Utc::now();
        let hour = chrono::Duration::hours(1);
        assert_eq!(
            placement_status(false, None, None, now),
            PlacementStatus::Draft
        );
        assert_eq!(
            placement_status(true, None, None, now),
            PlacementStatus::Live
        );
        assert_eq!(
            placement_status(true, Some(now), None, now),
            PlacementStatus::Live
        );
        assert_eq!(
            placement_status(true, Some(now + hour), None, now),
            PlacementStatus::Scheduled
        );
        assert_eq!(
            placement_status(true, None, Some(now), now),
            PlacementStatus::Ended
        );
        assert_eq!(
            placement_status(true, Some(now - hour), Some(now + hour), now),
            PlacementStatus::Live
        );
        assert_eq!(
            placement_status(false, Some(now + hour), None, now),
            PlacementStatus::Draft
        );
    }

    #[test]
    fn app_to_package_category_mapping_parses_and_inverts() {
        let business = parse_app_category("Business").unwrap();
        assert_eq!(
            package_categories_for(&business)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["ANALYTICS_REPORTING", "INTEGRATION_CONNECTORS"]
        );
        let app_categories: Vec<AppCategory> = DbCategory::iter().map(AppCategory::from).collect();
        assert_eq!(app_categories.len(), 21);
        for app in &app_categories {
            let name = app_category_name(app);
            assert!(parse_app_category(&name).is_some(), "{name}");
            for package in ALL_PACKAGE_CATEGORIES {
                assert_eq!(
                    package_categories_for(app).contains(package),
                    app_categories_for(package).contains(&name.as_str()),
                    "{name} vs {package}"
                );
            }
        }
        for package in ALL_PACKAGE_CATEGORIES {
            for name in app_categories_for(package) {
                assert!(parse_app_category(name).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn package_categories_normalize_and_map_to_the_entity() {
        assert_eq!(
            normalize_package_category("AnalyticsReporting").as_deref(),
            Some("ANALYTICS_REPORTING")
        );
        assert_eq!(
            normalize_package_category("ANALYTICS_REPORTING").as_deref(),
            Some("ANALYTICS_REPORTING")
        );
        assert_eq!(normalize_package_category("Analytics"), None);
        assert_eq!(
            ALL_PACKAGE_CATEGORIES.len(),
            DbWasmPackageCategory::iter().count()
        );
        for category in ALL_PACKAGE_CATEGORIES {
            let db = package_category_to_db(*category);
            assert_eq!(db.to_value(), category.to_string());
            assert_eq!(package_category_from_db(&db), *category);
        }
    }

    #[derive(Deserialize)]
    struct SearchParams {
        categories: Option<String>,
        permissions: Option<String>,
    }

    fn search(
        uri: &str,
    ) -> (
        Result<Vec<CategoryFilter>, ApiError>,
        Result<Vec<PermissionGroup>, ApiError>,
    ) {
        let Query(params) = Query::<SearchParams>::try_from_uri(&uri.parse().unwrap()).unwrap();
        (
            parse_categories(params.categories.as_deref()),
            parse_permissions(params.permissions.as_deref()),
        )
    }

    #[test]
    fn search_lists_parse_from_comma_separated_params() {
        let (categories, permissions) = search("/?categories=app:Finance");
        assert_eq!(categories.unwrap(), [CategoryFilter::App("Finance".into())]);
        assert!(permissions.unwrap().is_empty());
        let (categories, permissions) =
            search("/?categories=app:Finance,package:EDUCATION&permissions=network,models");
        assert_eq!(
            categories.unwrap(),
            [
                CategoryFilter::App("Finance".into()),
                CategoryFilter::Package(WasmPackageCategory::Education)
            ]
        );
        assert_eq!(
            permissions.unwrap(),
            [PermissionGroup::Network, PermissionGroup::Models]
        );
        let (categories, _) = search("/?categories=app%3AFinance%2Cpackage%3AEDUCATION");
        assert_eq!(categories.unwrap().len(), 2);

        let seventeen = vec!["app:Finance"; SEARCH_CATEGORIES_MAX + 1].join(",");
        let (categories, _) = search(&format!("/?categories={seventeen}"));
        assert_eq!(
            message(categories.unwrap_err()),
            "categories accepts at most 16 values, got 17"
        );
        let (_, permissions) = search("/?permissions=none,network,models,storage,none");
        assert!(permissions.is_err());
        let (categories, _) = search("/?categories=package:Education");
        assert!(message(categories.unwrap_err()).contains("'package:Education'"));
        let (_, permissions) = search("/?permissions=foo");
        let error = permissions.unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(message(error).contains("'foo'"));
    }

    #[test]
    fn facet_values_round_trip_through_the_category_parser() {
        for value in [
            "app:Finance",
            "app:FoodAndDrink",
            "package:DOCUMENT_PROCESSING",
        ] {
            let parsed = CategoryFilter::parse(value).unwrap();
            assert_eq!(parsed.value(), value);
        }
        for package in ALL_PACKAGE_CATEGORIES {
            let value = CategoryFilter::Package(*package).value();
            assert_eq!(parse_categories(Some(&value)).unwrap()[0].value(), value);
        }
    }

    #[test]
    fn permission_groups_match_capability_tags() {
        let tags = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert!(PermissionGroup::Network.matches(&tags(&["net.http"])));
        assert!(PermissionGroup::Network.matches(&tags(&["widget.net"])));
        assert!(!PermissionGroup::Network.matches(&tags(&["models"])));
        assert!(PermissionGroup::Models.matches(&tags(&["models"])));
        assert!(PermissionGroup::Storage.matches(&tags(&["storage.node"])));
        assert!(PermissionGroup::NoPermissions.matches(&[]));
        assert!(!PermissionGroup::NoPermissions.matches(&tags(&["models"])));
        assert_eq!(
            serde_json::to_value(PermissionGroup::NoPermissions).unwrap(),
            "none"
        );
    }

    #[test]
    fn layout_helpers_find_references_and_capacity() {
        let spotlight = PlacementInput {
            items: vec![item(ItemKind::Collection, "c1")],
            ..input(
                PlacementContent::Spotlight {
                    rotation_seconds: 8,
                    auto_fill: false,
                },
                vec![],
            )
        }
        .into_doc("s1".into());
        let collection = input(
            collection_content(CollectionSource::Hand),
            items(ItemKind::App, 2),
        )
        .into_doc("c1".into());
        let layout = LayoutDoc {
            slots: vec![
                SlotDoc {
                    key: SLOT_HERO.into(),
                    area: SlotArea::Grid,
                    position: 0,
                    placements: vec![spotlight],
                },
                SlotDoc {
                    key: SLOT_UNPLACED.into(),
                    area: SlotArea::Unplaced,
                    position: 0,
                    placements: vec![collection],
                },
            ],
        };
        assert_eq!(layout.collection_ids(), HashSet::from(["c1".to_owned()]));
        assert_eq!(layout.referencing("c1")[0].id, "s1");
        assert_eq!(layout.find("c1").unwrap().0.key, SLOT_UNPLACED);
        assert!(layout.ensure_room_for_placement().is_ok());
        validate_layout(&layout).unwrap();
        let spotlight = PlacementContent::Spotlight {
            rotation_seconds: 8,
            auto_fill: true,
        };
        assert!(layout.check_target(SLOT_HERO, &spotlight).is_ok());
        assert!(layout.check_target(SLOT_UNPLACED, &spotlight).is_ok());
        assert_eq!(
            message(
                layout
                    .check_target("row:x", &collection_content(CollectionSource::Hand))
                    .unwrap_err()
            ),
            "Slot row:x is not part of the layout; create the row first"
        );
        assert_eq!(
            message(
                layout
                    .check_target(SLOT_HERO, &rail(RailKey::Trending))
                    .unwrap_err()
            ),
            "hero does not accept rail/trending"
        );
        assert!(
            layout
                .check_target("sidebar", &rail(RailKey::Trending))
                .is_err()
        );
    }
}
