use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DeleteMany, EntityTrait, QueryFilter, QueryOrder,
    sea_query::{Expr, OnConflict},
};

use super::defaults::default_layout;
use super::model::{
    ChangeKind, ExploreChange, ExploreOrderBody, GRID_SLOTS, ItemOverrides, LayoutDoc,
    PlacementContent, PlacementDoc, PlacementItemDoc, ROW_SLUG_MAX, ROWS_MAX, SLOT_UNPLACED,
    SlotArea, SlotDoc, check_slot, is_row_key, validate_layout,
};
use crate::entity::sea_orm_active_enums::ExploreSlotArea;
use crate::entity::{explore_layout, explore_placement, explore_placement_item, explore_slot};
use crate::error::ApiError;

/// Revision of an edition that has no header row yet (the built-in default layout).
pub const DEFAULT_REVISION: &str = "default";
pub const DRAFT_CHANGED: &str = "The Explore draft changed. Reload it before saving.";
const WRITE_CHUNK: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edition {
    Draft,
    Live,
}

impl Edition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Live => "LIVE",
        }
    }
}

/// An edition's header row (absent until its first write) and the layout under it.
pub struct Loaded {
    pub header: Option<explore_layout::Model>,
    /// Every placement whose stored content parses; reads show this and skip the rest.
    pub layout: LayoutDoc,
    /// Ids of placements whose stored content this server cannot parse, e.g. one saved by a newer version
    /// during a rolling deploy.
    pub unreadable: Vec<String>,
}

impl Loaded {
    pub fn revision(&self) -> &str {
        self.header
            .as_ref()
            .map_or(DEFAULT_REVISION, |header| header.revision.as_str())
    }

    /// The layout a mutation builds on. Writing from a layout that skipped unreadable placements would drop
    /// them from a copy or leave them behind in a reordered slot, so any unreadable placement fails the
    /// mutation before anything is written.
    pub fn writable(self) -> Result<LayoutDoc, ApiError> {
        if self.unreadable.is_empty() {
            return Ok(self.layout);
        }
        Err(ApiError::conflict(format!(
            "Explore placements {} cannot be read by this server version, so nothing was changed. Retry once every server runs the same version.",
            self.unreadable.join(", ")
        )))
    }
}

pub async fn header<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
) -> Result<Option<explore_layout::Model>, ApiError> {
    Ok(explore_layout::Entity::find_by_id(edition.as_str())
        .one(db)
        .await?)
}

/// Header and layout of an edition, read back to back. They agree only inside one snapshot: mutations call
/// this in their coordinated transaction, and readers (the public page, whose cache key is
/// [`Loaded::revision`], and the editor) in a REPEATABLE READ transaction via `state.transaction_with`.
pub async fn load<C: ConnectionTrait>(db: &C, edition: Edition) -> Result<Loaded, ApiError> {
    let header = header(db, edition).await?;
    let (layout, unreadable) = layout_for(db, edition, header.as_ref()).await?;
    Ok(Loaded {
        header,
        layout,
        unreadable,
    })
}

/// The layout under a `header` read earlier in the same transaction, plus the ids of the placements it had to
/// skip: the built-in default when there is no header, otherwise the slot, placement and item queries.
pub async fn layout_for<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
    header: Option<&explore_layout::Model>,
) -> Result<(LayoutDoc, Vec<String>), ApiError> {
    if header.is_none() {
        return Ok((default_layout(), Vec::new()));
    }
    let slots = explore_slot::Entity::find()
        .filter(explore_slot::Column::Edition.eq(edition.as_str()))
        .order_by_asc(explore_slot::Column::Area)
        .order_by_asc(explore_slot::Column::Position)
        .order_by_asc(explore_slot::Column::Key)
        .all(db)
        .await?;
    let placements = explore_placement::Entity::find()
        .filter(explore_placement::Column::Edition.eq(edition.as_str()))
        .order_by_asc(explore_placement::Column::SlotKey)
        .order_by_asc(explore_placement::Column::Position)
        .order_by_asc(explore_placement::Column::Id)
        .all(db)
        .await?;
    let items = explore_placement_item::Entity::find()
        .filter(explore_placement_item::Column::Edition.eq(edition.as_str()))
        .order_by_asc(explore_placement_item::Column::PlacementId)
        .order_by_asc(explore_placement_item::Column::Position)
        .all(db)
        .await?;
    Ok(assemble(slots, placements, items))
}

/// Builds the layout from rows already in load order: fixed grid slots, rows by position, then `unplaced`,
/// which also collects placements whose slot row is missing. Placements whose content does not parse are left
/// out and returned by id.
fn assemble(
    slots: Vec<explore_slot::Model>,
    placements: Vec<explore_placement::Model>,
    items: Vec<explore_placement_item::Model>,
) -> (LayoutDoc, Vec<String>) {
    let mut items_by_placement: HashMap<String, Vec<PlacementItemDoc>> = HashMap::new();
    for item in items {
        let overrides = serde_json::from_value::<ItemOverrides>(item.overrides).unwrap_or_else(|error| {
            tracing::warn!(placement = %item.placement_id, %error, "ignoring unreadable Explore item overrides");
            ItemOverrides::default()
        });
        items_by_placement
            .entry(item.placement_id)
            .or_default()
            .push(PlacementItemDoc::from_row(
                item.item_kind.into(),
                item.item_id,
                overrides,
            ));
    }

    let mut by_slot: HashMap<String, Vec<PlacementDoc>> = HashMap::new();
    let mut unreadable = Vec::new();
    for row in placements {
        let content = match serde_json::from_value::<PlacementContent>(row.content) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(placement = %row.id, %error, "skipping unreadable Explore placement");
                unreadable.push(row.id);
                continue;
            }
        };
        let items = items_by_placement.remove(&row.id).unwrap_or_default();
        by_slot.entry(row.slot_key).or_default().push(PlacementDoc {
            id: row.id,
            kind: content.kind(),
            name: row.name,
            enabled: row.enabled,
            starts_at: row.starts_at.map(|at| at.with_timezone(&Utc)),
            ends_at: row.ends_at.map(|at| at.with_timezone(&Utc)),
            audience: row.audience.into_inner(),
            content,
            items,
            status: None,
            updated_at: Some(row.updated_at.with_timezone(&Utc)),
            created_at: Some(row.created_at.with_timezone(&Utc)),
        });
    }

    let mut layout = LayoutDoc::default();
    for (position, key) in GRID_SLOTS.iter().enumerate() {
        layout.slots.push(SlotDoc {
            key: (*key).to_owned(),
            area: SlotArea::Grid,
            position: position as i32,
            placements: by_slot.remove(*key).unwrap_or_default(),
        });
    }
    let rows = slots
        .into_iter()
        .filter(|slot| slot.area == ExploreSlotArea::Row && is_row_key(&slot.key));
    for (position, slot) in rows.enumerate() {
        layout.slots.push(SlotDoc {
            placements: by_slot.remove(&slot.key).unwrap_or_default(),
            key: slot.key,
            area: SlotArea::Row,
            position: position as i32,
        });
    }
    let mut unplaced = by_slot.remove(SLOT_UNPLACED).unwrap_or_default();
    let mut orphans: Vec<(String, Vec<PlacementDoc>)> = by_slot.into_iter().collect();
    orphans.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, placements) in orphans {
        tracing::warn!(slot = %key, "Explore placements reference a missing slot; listing them as unplaced");
        unplaced.extend(placements);
    }
    layout.slots.push(SlotDoc {
        key: SLOT_UNPLACED.to_owned(),
        area: SlotArea::Unplaced,
        position: 0,
        placements: unplaced,
    });
    (layout, unreadable)
}

async fn insert_all<C, A>(db: &C, rows: Vec<A>) -> Result<(), ApiError>
where
    C: ConnectionTrait,
    A: ActiveModelTrait + Clone + Send,
{
    for chunk in rows.chunks(WRITE_CHUNK) {
        <A::Entity as EntityTrait>::insert_many(chunk.to_vec())
            .exec_without_returning(db)
            .await?;
    }
    Ok(())
}

fn slot_row(
    edition: Edition,
    key: &str,
    area: SlotArea,
    position: i32,
) -> explore_slot::ActiveModel {
    explore_slot::ActiveModel {
        edition: Set(edition.as_str().to_owned()),
        key: Set(key.to_owned()),
        area: Set(area.into()),
        position: Set(position),
    }
}

fn placement_row(
    edition: Edition,
    slot_key: &str,
    position: i32,
    placement: &PlacementDoc,
    now: DateTime<Utc>,
) -> Result<explore_placement::ActiveModel, ApiError> {
    Ok(explore_placement::ActiveModel {
        edition: Set(edition.as_str().to_owned()),
        id: Set(placement.id.clone()),
        slot_key: Set(slot_key.to_owned()),
        position: Set(position),
        kind: Set(placement.content.kind().into()),
        name: Set(placement.name.clone()),
        enabled: Set(placement.enabled),
        starts_at: Set(placement.starts_at.map(Into::into)),
        ends_at: Set(placement.ends_at.map(Into::into)),
        audience: Set(placement.audience.clone().into()),
        content: Set(serde_json::to_value(&placement.content)?),
        created_at: Set(placement.created_at.unwrap_or(now).into()),
        updated_at: Set(placement.updated_at.unwrap_or(now).into()),
    })
}

fn item_rows(
    edition: Edition,
    placement_id: &str,
    items: &[PlacementItemDoc],
) -> Result<Vec<explore_placement_item::ActiveModel>, ApiError> {
    items
        .iter()
        .enumerate()
        .map(|(position, item)| {
            Ok(explore_placement_item::ActiveModel {
                edition: Set(edition.as_str().to_owned()),
                placement_id: Set(placement_id.to_owned()),
                position: Set(position as i32),
                item_kind: Set(item.kind.into()),
                item_id: Set(item.id.clone()),
                overrides: Set(serde_json::to_value(item.overrides())?),
            })
        })
        .collect()
}

/// Writes every row of `layout` into `edition` with positions `0..n-1`; the edition must hold no rows.
async fn write_layout<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
    layout: &LayoutDoc,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    let mut slots = Vec::new();
    let mut placements = Vec::new();
    let mut items = Vec::new();
    let mut rows = 0;
    for slot in &layout.slots {
        let position = match slot.area {
            SlotArea::Grid => GRID_SLOTS
                .iter()
                .position(|key| *key == slot.key)
                .unwrap_or_default() as i32,
            SlotArea::Row => {
                rows += 1;
                rows - 1
            }
            SlotArea::Unplaced => 0,
        };
        slots.push(slot_row(edition, &slot.key, slot.area, position));
        for (index, placement) in slot.placements.iter().enumerate() {
            placements.push(placement_row(
                edition,
                &slot.key,
                index as i32,
                placement,
                now,
            )?);
            items.extend(item_rows(edition, &placement.id, &placement.items)?);
        }
    }
    insert_all(db, slots).await?;
    insert_all(db, placements).await?;
    insert_all(db, items).await
}

async fn clear<C: ConnectionTrait>(db: &C, edition: Edition) -> Result<(), ApiError> {
    explore_placement_item::Entity::delete_many()
        .filter(explore_placement_item::Column::Edition.eq(edition.as_str()))
        .exec(db)
        .await?;
    explore_placement::Entity::delete_many()
        .filter(explore_placement::Column::Edition.eq(edition.as_str()))
        .exec(db)
        .await?;
    explore_slot::Entity::delete_many()
        .filter(explore_slot::Column::Edition.eq(edition.as_str()))
        .exec(db)
        .await?;
    Ok(())
}

/// Opens every draft mutation, before anything is loaded or validated, so a stale editor always gets the 409.
/// An existing DRAFT header must carry `expected_revision`. Without one, `"default"` materializes the default
/// layout under a header whose revision is "default" (the closing [`cas`] replaces it); any other revision, or
/// a concurrent materialization, is a 409.
pub async fn ensure_draft<C: ConnectionTrait>(
    db: &C,
    expected_revision: &str,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    if let Some(header) = header(db, Edition::Draft).await? {
        return if header.revision == expected_revision {
            Ok(())
        } else {
            Err(ApiError::conflict(DRAFT_CHANGED))
        };
    }
    if expected_revision != DEFAULT_REVISION {
        return Err(ApiError::conflict(DRAFT_CHANGED));
    }
    let inserted = explore_layout::Entity::insert(explore_layout::ActiveModel {
        edition: Set(Edition::Draft.as_str().to_owned()),
        revision: Set(DEFAULT_REVISION.to_owned()),
        published_at: Set(None),
        updated_at: Set(now.into()),
    })
    .on_conflict(
        OnConflict::column(explore_layout::Column::Edition)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    if inserted == 0 {
        return Err(ApiError::conflict(DRAFT_CHANGED));
    }
    clear(db, Edition::Draft).await?;
    write_layout(db, Edition::Draft, &default_layout(), now).await
}

/// Closes every draft mutation: compare-and-swap on the DRAFT revision. Returns the new revision.
pub async fn cas<C: ConnectionTrait>(
    db: &C,
    expected_revision: &str,
    now: DateTime<Utc>,
) -> Result<String, ApiError> {
    let revision = create_id();
    let result = explore_layout::Entity::update_many()
        .col_expr(
            explore_layout::Column::Revision,
            Expr::value(revision.clone()),
        )
        .col_expr(
            explore_layout::Column::UpdatedAt,
            Expr::value(DateTime::<chrono::FixedOffset>::from(now)),
        )
        .filter(explore_layout::Column::Edition.eq(Edition::Draft.as_str()))
        .filter(explore_layout::Column::Revision.eq(expected_revision))
        .exec(db)
        .await?;
    if result.rows_affected != 1 {
        return Err(ApiError::conflict(DRAFT_CHANGED));
    }
    Ok(revision)
}

async fn write_header<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
    revision: &str,
    published_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    explore_layout::Entity::insert(explore_layout::ActiveModel {
        edition: Set(edition.as_str().to_owned()),
        revision: Set(revision.to_owned()),
        published_at: Set(published_at.map(Into::into)),
        updated_at: Set(now.into()),
    })
    .on_conflict(
        OnConflict::column(explore_layout::Column::Edition)
            .update_columns([
                explore_layout::Column::Revision,
                explore_layout::Column::PublishedAt,
                explore_layout::Column::UpdatedAt,
            ])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

async fn copy<C: ConnectionTrait>(
    db: &C,
    layout: &LayoutDoc,
    to: Edition,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    clear(db, to).await?;
    write_layout(db, to, layout, now).await
}

/// `POST /admin/explore/publish`, inside the caller's coordinated transaction: checks the revision, validates
/// the whole draft, bumps the draft revision and replaces LIVE with the draft under a new LIVE revision and
/// `publishedAt = now`. Returns the new draft revision.
pub async fn publish<C: ConnectionTrait>(
    db: &C,
    expected_revision: &str,
    now: DateTime<Utc>,
) -> Result<String, ApiError> {
    ensure_draft(db, expected_revision, now).await?;
    let draft = load(db, Edition::Draft).await?.writable()?;
    validate_layout(&draft)?;
    let revision = cas(db, expected_revision, now).await?;
    copy(db, &draft, Edition::Live, now).await?;
    write_header(db, Edition::Live, &create_id(), Some(now), now).await?;
    Ok(revision)
}

/// `POST /admin/explore/discard`, inside the caller's coordinated transaction: DRAFT becomes a copy of LIVE,
/// or of the default layout while nothing is live. Returns the new draft revision.
pub async fn discard<C: ConnectionTrait>(
    db: &C,
    expected_revision: &str,
    now: DateTime<Utc>,
) -> Result<String, ApiError> {
    ensure_draft(db, expected_revision, now).await?;
    let live = load(db, Edition::Live).await?.writable()?;
    copy(db, &live, Edition::Draft, now).await?;
    cas(db, expected_revision, now).await
}

pub async fn insert_placement<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
    slot_key: &str,
    position: i32,
    placement: &PlacementDoc,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    insert_all(
        db,
        vec![placement_row(edition, slot_key, position, placement, now)?],
    )
    .await?;
    insert_all(db, item_rows(edition, &placement.id, &placement.items)?).await
}

fn delete_items(
    edition: Edition,
    placement_id: &str,
) -> DeleteMany<explore_placement_item::Entity> {
    explore_placement_item::Entity::delete_many()
        .filter(explore_placement_item::Column::Edition.eq(edition.as_str()))
        .filter(explore_placement_item::Column::PlacementId.eq(placement_id))
}

/// What [`delete_placement`] removes, items first.
fn delete_statements(
    edition: Edition,
    id: &str,
) -> (
    DeleteMany<explore_placement_item::Entity>,
    DeleteMany<explore_placement::Entity>,
) {
    (
        delete_items(edition, id),
        explore_placement::Entity::delete_many()
            .filter(explore_placement::Column::Edition.eq(edition.as_str()))
            .filter(explore_placement::Column::Id.eq(id)),
    )
}

/// Replaces a placement's copy, window, audience, content and items. The kind and slot stay.
pub async fn update_placement<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
    placement: &PlacementDoc,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    let result = explore_placement::Entity::update_many()
        .set(explore_placement::ActiveModel {
            name: Set(placement.name.clone()),
            enabled: Set(placement.enabled),
            starts_at: Set(placement.starts_at.map(Into::into)),
            ends_at: Set(placement.ends_at.map(Into::into)),
            audience: Set(placement.audience.clone().into()),
            content: Set(serde_json::to_value(&placement.content)?),
            updated_at: Set(now.into()),
            ..Default::default()
        })
        .filter(explore_placement::Column::Edition.eq(edition.as_str()))
        .filter(explore_placement::Column::Id.eq(&placement.id))
        .exec(db)
        .await?;
    if result.rows_affected != 1 {
        return Err(ApiError::not_found(format!(
            "Explore placement {} does not exist",
            placement.id
        )));
    }
    delete_items(edition, &placement.id).exec(db).await?;
    insert_all(db, item_rows(edition, &placement.id, &placement.items)?).await
}

/// Puts `ids` into `slot_key` in this priority order, positions `0..n-1`.
pub async fn write_slot_order<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
    slot_key: &str,
    ids: &[String],
) -> Result<(), ApiError> {
    for (position, id) in ids.iter().enumerate() {
        explore_placement::Entity::update_many()
            .col_expr(explore_placement::Column::SlotKey, Expr::value(slot_key))
            .col_expr(
                explore_placement::Column::Position,
                Expr::value(position as i32),
            )
            .filter(explore_placement::Column::Edition.eq(edition.as_str()))
            .filter(explore_placement::Column::Id.eq(id))
            .exec(db)
            .await?;
    }
    Ok(())
}

/// Deletes a placement and its item rows, then closes the gap in its slot. Returns the slot it was in, or
/// `None` when it did not exist.
pub async fn delete_placement<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
    id: &str,
) -> Result<Option<String>, ApiError> {
    let Some(row) =
        explore_placement::Entity::find_by_id((edition.as_str().to_owned(), id.to_owned()))
            .one(db)
            .await?
    else {
        return Ok(None);
    };
    let (items, placement) = delete_statements(edition, id);
    items.exec(db).await?;
    placement.exec(db).await?;
    let remaining: Vec<String> = explore_placement::Entity::find()
        .filter(explore_placement::Column::Edition.eq(edition.as_str()))
        .filter(explore_placement::Column::SlotKey.eq(&row.slot_key))
        .order_by_asc(explore_placement::Column::Position)
        .order_by_asc(explore_placement::Column::Id)
        .all(db)
        .await?
        .into_iter()
        .map(|placement| placement.id)
        .collect();
    write_slot_order(db, edition, &row.slot_key, &remaining).await?;
    Ok(Some(row.slot_key))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotOrderPlan {
    pub key: String,
    pub placement_ids: Vec<String>,
}

/// A validated `PUT /admin/explore/order`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderPlan {
    /// Complete row order; positions become `0..n-1`.
    pub rows: Vec<String>,
    pub created_rows: Vec<String>,
    /// Rows that are empty after the moves and are deleted.
    pub removed_rows: Vec<String>,
    /// Touched slots (removed rows excluded) with their complete new priority order. When rows are created,
    /// `unplaced` is always included, so a placement listed there because its slot row was missing is moved
    /// into `unplaced` for real instead of reappearing in a new row of the same key.
    pub slots: Vec<SlotOrderPlan>,
}

/// §3.5 order payload rules: the body names the complete row list and, for every touched slot, its full new
/// priority list; together those lists are a permutation of the placements currently in the touched slots.
pub fn validate_order(current: &LayoutDoc, body: &ExploreOrderBody) -> Result<OrderPlan, ApiError> {
    if body.rows.len() > ROWS_MAX {
        return Err(ApiError::bad_request(format!(
            "An Explore layout holds at most {ROWS_MAX} rows (got {})",
            body.rows.len()
        )));
    }
    let mut rows = HashSet::new();
    for key in &body.rows {
        if !is_row_key(key) {
            return Err(ApiError::bad_request(format!(
                "Row key '{key}' must be row:<slug> with 1 to {ROW_SLUG_MAX} lowercase letters, digits or hyphens"
            )));
        }
        if !rows.insert(key.as_str()) {
            return Err(ApiError::bad_request(format!("Row {key} is listed twice")));
        }
    }
    let existing_rows: Vec<&str> = current.rows().map(|slot| slot.key.as_str()).collect();
    let created_rows: Vec<String> = body
        .rows
        .iter()
        .filter(|key| !existing_rows.contains(&key.as_str()))
        .cloned()
        .collect();
    let removed_rows: Vec<String> = existing_rows
        .iter()
        .filter(|key| !rows.contains(*key))
        .map(|key| (*key).to_owned())
        .collect();

    let mut touched = HashSet::new();
    for slot in &body.slots {
        let key = slot.key.as_str();
        let known = GRID_SLOTS.contains(&key)
            || key == SLOT_UNPLACED
            || rows.contains(key)
            || existing_rows.contains(&key);
        if !known {
            return Err(ApiError::bad_request(format!(
                "Slot {key} is not part of the layout"
            )));
        }
        if !touched.insert(key) {
            return Err(ApiError::bad_request(format!("Slot {key} is listed twice")));
        }
    }

    let located: HashMap<&str, (&str, &PlacementDoc)> = current
        .placements()
        .map(|(slot, placement)| (placement.id.as_str(), (slot.key.as_str(), placement)))
        .collect();
    let mut listed = HashSet::new();
    for slot in &body.slots {
        for id in &slot.placement_ids {
            let Some((source, placement)) = located.get(id.as_str()) else {
                return Err(ApiError::bad_request(format!(
                    "Placement {id} does not exist"
                )));
            };
            if !touched.contains(source) {
                return Err(ApiError::bad_request(format!(
                    "Placement {id} is in {source}, which is not in this request"
                )));
            }
            if !listed.insert(id.as_str()) {
                return Err(ApiError::bad_request(format!(
                    "Placement {id} is listed twice"
                )));
            }
            check_slot(&slot.key, &placement.content)?;
        }
    }
    for (slot, placement) in current.placements() {
        if touched.contains(slot.key.as_str()) && !listed.contains(placement.id.as_str()) {
            return Err(ApiError::bad_request(format!(
                "Placement {} from {} is missing",
                placement.id, slot.key
            )));
        }
    }
    for key in &removed_rows {
        let still_filled = match body.slots.iter().find(|slot| slot.key == *key) {
            Some(slot) => !slot.placement_ids.is_empty(),
            None => current
                .slot(key)
                .is_some_and(|slot| !slot.placements.is_empty()),
        };
        if still_filled {
            return Err(ApiError::bad_request(format!(
                "Row {key} still has placements; move them first"
            )));
        }
    }

    let mut slots: Vec<SlotOrderPlan> = body
        .slots
        .iter()
        .filter(|slot| !removed_rows.contains(&slot.key))
        .map(|slot| SlotOrderPlan {
            key: slot.key.clone(),
            placement_ids: slot.placement_ids.clone(),
        })
        .collect();
    if !created_rows.is_empty()
        && !touched.contains(SLOT_UNPLACED)
        && let Some(unplaced) = current.slot(SLOT_UNPLACED)
        && !unplaced.placements.is_empty()
    {
        slots.push(SlotOrderPlan {
            key: SLOT_UNPLACED.to_owned(),
            placement_ids: unplaced
                .placements
                .iter()
                .map(|placement| placement.id.clone())
                .collect(),
        });
    }

    Ok(OrderPlan {
        rows: body.rows.clone(),
        created_rows,
        removed_rows,
        slots,
    })
}

/// The layout `plan` produces, with contiguous positions; the oracle for what [`apply_order`] writes.
#[cfg(test)]
fn apply_plan(current: &LayoutDoc, plan: &OrderPlan) -> LayoutDoc {
    let by_id: HashMap<&str, &PlacementDoc> = current
        .placements()
        .map(|(_, placement)| (placement.id.as_str(), placement))
        .collect();
    let placements_of = |slot: &SlotDoc| -> Vec<PlacementDoc> {
        match plan.slots.iter().find(|order| order.key == slot.key) {
            Some(order) => order
                .placement_ids
                .iter()
                .filter_map(|id| by_id.get(id.as_str()).map(|placement| (*placement).clone()))
                .collect(),
            None => slot.placements.clone(),
        }
    };
    let grid = current
        .slots
        .iter()
        .filter(|slot| slot.area == SlotArea::Grid)
        .map(|slot| SlotDoc {
            placements: placements_of(slot),
            ..slot.clone()
        });
    let rows = plan.rows.iter().enumerate().map(|(position, key)| {
        let slot = current.slot(key).cloned().unwrap_or(SlotDoc {
            key: key.clone(),
            area: SlotArea::Row,
            position: 0,
            placements: Vec::new(),
        });
        SlotDoc {
            placements: placements_of(&slot),
            position: position as i32,
            ..slot
        }
    });
    let unplaced = current
        .slots
        .iter()
        .filter(|slot| slot.area == SlotArea::Unplaced)
        .map(|slot| SlotDoc {
            placements: placements_of(slot),
            ..slot.clone()
        });
    LayoutDoc {
        slots: grid.chain(rows).chain(unplaced).collect(),
    }
}

pub async fn apply_order<C: ConnectionTrait>(
    db: &C,
    edition: Edition,
    plan: &OrderPlan,
) -> Result<(), ApiError> {
    if !plan.removed_rows.is_empty() {
        explore_slot::Entity::delete_many()
            .filter(explore_slot::Column::Edition.eq(edition.as_str()))
            .filter(explore_slot::Column::Key.is_in(plan.removed_rows.clone()))
            .exec(db)
            .await?;
    }
    insert_all(
        db,
        plan.created_rows
            .iter()
            .map(|key| slot_row(edition, key, SlotArea::Row, 0))
            .collect(),
    )
    .await?;
    for (position, key) in plan.rows.iter().enumerate() {
        explore_slot::Entity::update_many()
            .col_expr(explore_slot::Column::Position, Expr::value(position as i32))
            .filter(explore_slot::Column::Edition.eq(edition.as_str()))
            .filter(explore_slot::Column::Key.eq(key))
            .exec(db)
            .await?;
    }
    for slot in &plan.slots {
        write_slot_order(db, edition, &slot.key, &slot.placement_ids).await?;
    }
    Ok(())
}

struct Located<'a> {
    placement: &'a PlacementDoc,
    slot: &'a str,
    index: usize,
    row: Option<usize>,
}

fn locate(layout: &LayoutDoc) -> HashMap<&str, Located<'_>> {
    let mut located = HashMap::new();
    let mut rows = 0;
    for slot in &layout.slots {
        let row = (slot.area == SlotArea::Row).then(|| {
            rows += 1;
            rows - 1
        });
        for (index, placement) in slot.placements.iter().enumerate() {
            located.insert(
                placement.id.as_str(),
                Located {
                    placement,
                    slot: slot.key.as_str(),
                    index,
                    row,
                },
            );
        }
    }
    located
}

/// Placement-level changes of the draft against the live edition: added, removed, modified (copy, content,
/// items, window, audience or enabled) or moved (slot, priority or row order).
pub fn diff(draft: &LayoutDoc, live: &LayoutDoc) -> Vec<ExploreChange> {
    let before = locate(live);
    let after = locate(draft);
    let change = |placement: &PlacementDoc, change: ChangeKind| ExploreChange {
        placement_id: placement.id.clone(),
        name: placement.name.clone(),
        change,
    };
    let mut changes = Vec::new();
    for (_, placement) in draft.placements() {
        let now = &after[placement.id.as_str()];
        let kind = match before.get(placement.id.as_str()) {
            None => Some(ChangeKind::Added),
            Some(old) if !old.placement.same_content(placement) => Some(ChangeKind::Modified),
            Some(old) if (old.slot, old.index, old.row) != (now.slot, now.index, now.row) => {
                Some(ChangeKind::Moved)
            }
            Some(_) => None,
        };
        if let Some(kind) = kind {
            changes.push(change(placement, kind));
        }
    }
    for (_, placement) in live.placements() {
        if !after.contains_key(placement.id.as_str()) {
            changes.push(change(placement, ChangeKind::Removed));
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::{ExploreItemKind, ExplorePlacementKind};
    use crate::routes::explore::model::{
        CollectionSource, ExploreSlotOrder, ItemKind, RailKey, SLOT_COLLECTION, SLOT_FEATURE,
        SLOT_HERO, SLOT_STAT, slot_area,
    };
    use sea_orm::{DbBackend, QueryTrait};
    use serde_json::json;

    fn message(error: ApiError) -> String {
        error.public_message().unwrap_or_default().to_owned()
    }

    fn placement(id: &str, content: PlacementContent) -> PlacementDoc {
        PlacementDoc {
            id: id.into(),
            kind: content.kind(),
            name: id.into(),
            enabled: true,
            starts_at: None,
            ends_at: None,
            audience: vec![],
            content,
            items: vec![],
            status: None,
            updated_at: None,
            created_at: None,
        }
    }

    fn rail(id: &str, key: RailKey) -> PlacementDoc {
        placement(
            id,
            PlacementContent::Rail {
                rail: key,
                title: None,
            },
        )
    }

    fn collection(id: &str) -> PlacementDoc {
        placement(
            id,
            PlacementContent::Collection {
                title: id.into(),
                blurb: None,
                source: CollectionSource::Hand,
                rule: None,
            },
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

    fn sample() -> LayoutDoc {
        layout(vec![
            (SLOT_HERO, vec![]),
            (SLOT_COLLECTION, vec![collection("c1")]),
            (SLOT_STAT, vec![rail("stat", RailKey::NewCount)]),
            (
                "row:a",
                vec![rail("trending", RailKey::Trending), collection("c2")],
            ),
            ("row:b", vec![rail("new", RailKey::New)]),
            ("row:empty", vec![]),
            (SLOT_UNPLACED, vec![rail("cats", RailKey::ByCategory)]),
        ])
    }

    fn body(rows: &[&str], slots: &[(&str, &[&str])]) -> ExploreOrderBody {
        ExploreOrderBody {
            expected_revision: "rev".into(),
            rows: rows.iter().map(|row| row.to_string()).collect(),
            slots: slots
                .iter()
                .map(|(key, ids)| ExploreSlotOrder {
                    key: key.to_string(),
                    placement_ids: ids.iter().map(|id| id.to_string()).collect(),
                })
                .collect(),
        }
    }

    const ROWS: &[&str] = &["row:a", "row:b", "row:empty"];

    fn ids(layout: &LayoutDoc, key: &str) -> Vec<String> {
        layout
            .slot(key)
            .map(|slot| slot.placements.iter().map(|p| p.id.clone()).collect())
            .unwrap_or_default()
    }

    fn slot_ids(layout: &LayoutDoc) -> Vec<(String, Vec<String>)> {
        layout
            .slots
            .iter()
            .map(|slot| (slot.key.clone(), ids(layout, &slot.key)))
            .collect()
    }

    #[test]
    fn a_move_must_name_its_source_slot() {
        let error =
            validate_order(&sample(), &body(ROWS, &[("row:b", &["new", "trending"])])).unwrap_err();
        assert_eq!(
            message(error),
            "Placement trending is in row:a, which is not in this request"
        );
    }

    #[test]
    fn duplicated_and_missing_ids_are_rejected() {
        let duplicated = body(
            ROWS,
            &[
                ("row:a", &["trending", "c2"]),
                ("row:b", &["new", "trending"]),
            ],
        );
        assert_eq!(
            message(validate_order(&sample(), &duplicated).unwrap_err()),
            "Placement trending is listed twice"
        );
        let missing = body(ROWS, &[("row:a", &["c2"]), ("row:b", &["new"])]);
        assert_eq!(
            message(validate_order(&sample(), &missing).unwrap_err()),
            "Placement trending from row:a is missing"
        );
        let unknown = body(ROWS, &[("row:b", &["new", "ghost"])]);
        assert!(validate_order(&sample(), &unknown).is_err());
        let twice = body(ROWS, &[("row:b", &["new"]), ("row:b", &["new"])]);
        assert_eq!(
            message(validate_order(&sample(), &twice).unwrap_err()),
            "Slot row:b is listed twice"
        );
    }

    #[test]
    fn moves_respect_slot_compatibility() {
        let trending_to_stat = body(
            ROWS,
            &[("row:a", &["c2"]), (SLOT_STAT, &["stat", "trending"])],
        );
        assert_eq!(
            message(validate_order(&sample(), &trending_to_stat).unwrap_err()),
            "stat does not accept rail/trending"
        );
        let categories_to_row = body(ROWS, &[(SLOT_UNPLACED, &[]), ("row:b", &["new", "cats"])]);
        assert_eq!(
            message(validate_order(&sample(), &categories_to_row).unwrap_err()),
            "row:b does not accept rail/by_category"
        );
        let new_count_to_unplaced = body(
            ROWS,
            &[(SLOT_STAT, &[]), (SLOT_UNPLACED, &["cats", "stat"])],
        );
        assert!(validate_order(&sample(), &new_count_to_unplaced).is_ok());
        let feature_slot = body(ROWS, &[(SLOT_COLLECTION, &[]), (SLOT_FEATURE, &["c1"])]);
        assert!(validate_order(&sample(), &feature_slot).is_err());
    }

    #[test]
    fn a_valid_move_leaves_both_slots_contiguous() {
        let current = sample();
        let move_body = body(ROWS, &[("row:a", &["c2"]), ("row:b", &["trending", "new"])]);
        let plan = validate_order(&current, &move_body).unwrap();
        assert!(plan.created_rows.is_empty() && plan.removed_rows.is_empty());
        let next = apply_plan(&current, &plan);
        assert_eq!(ids(&next, "row:a"), ["c2"]);
        assert_eq!(ids(&next, "row:b"), ["trending", "new"]);
        let rows: Vec<(String, i32)> = next
            .rows()
            .map(|slot| (slot.key.clone(), slot.position))
            .collect();
        assert_eq!(
            rows,
            [
                ("row:a".into(), 0),
                ("row:b".into(), 1),
                ("row:empty".into(), 2)
            ]
        );
        assert_eq!(next.placement_count(), current.placement_count());
        let changes = diff(&next, &current);
        assert_eq!(
            changes
                .iter()
                .map(|change| (change.placement_id.as_str(), change.change))
                .collect::<Vec<_>>(),
            [
                ("c2", ChangeKind::Moved),
                ("trending", ChangeKind::Moved),
                ("new", ChangeKind::Moved)
            ]
        );
    }

    #[test]
    fn rows_are_created_reordered_and_removed_only_when_empty() {
        let current = sample();
        let plan = validate_order(&current, &body(&["row:b", "row:new", "row:a"], &[])).unwrap();
        assert_eq!(plan.created_rows, ["row:new"]);
        assert_eq!(plan.removed_rows, ["row:empty"]);
        let next = apply_plan(&current, &plan);
        assert_eq!(
            next.rows()
                .map(|slot| slot.key.as_str())
                .collect::<Vec<_>>(),
            ["row:b", "row:new", "row:a"]
        );
        assert!(
            next.rows()
                .enumerate()
                .all(|(i, slot)| slot.position == i as i32)
        );
        let filled = validate_order(&current, &body(&["row:a", "row:empty"], &[]));
        assert_eq!(
            message(filled.unwrap_err()),
            "Row row:b still has placements; move them first"
        );
        let emptied = body(
            &["row:a", "row:empty"],
            &[("row:b", &[]), ("row:a", &["trending", "c2", "new"])],
        );
        let plan = validate_order(&current, &emptied).unwrap();
        assert_eq!(plan.removed_rows, ["row:b"]);
        assert_eq!(plan.slots.len(), 1);
        assert_eq!(
            ids(&apply_plan(&current, &plan), "row:a"),
            ["trending", "c2", "new"]
        );
        assert!(validate_order(&current, &body(&["row:a", "row:a"], &[])).is_err());
        assert!(validate_order(&current, &body(&["row:Bad"], &[])).is_err());
        assert!(validate_order(&current, &body(ROWS, &[("row:ghost", &[])])).is_err());
        let into_new = body(
            &["row:a", "row:b", "row:empty", "row:fresh"],
            &[("row:b", &[]), ("row:fresh", &["new"])],
        );
        let plan = validate_order(&current, &into_new).unwrap();
        assert_eq!(ids(&apply_plan(&current, &plan), "row:fresh"), ["new"]);
    }

    #[test]
    fn the_row_limit_holds_at_the_boundary_and_fails_one_past() {
        let keys: Vec<String> = (0..=ROWS_MAX).map(|i| format!("row:r{i}")).collect();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let empty = layout(vec![]);
        let plan = validate_order(&empty, &body(&keys[..ROWS_MAX], &[])).unwrap();
        assert_eq!(plan.created_rows.len(), ROWS_MAX);
        assert_eq!(
            message(validate_order(&empty, &body(&keys, &[])).unwrap_err()),
            format!(
                "An Explore layout holds at most {ROWS_MAX} rows (got {})",
                ROWS_MAX + 1
            )
        );
    }

    #[test]
    fn creating_a_row_rewrites_the_unplaced_list() {
        let current = sample();
        let plan = validate_order(&current, &body(&["row:a", "row:b", "row:x"], &[])).unwrap();
        assert_eq!(plan.created_rows, ["row:x"]);
        assert_eq!(
            plan.slots,
            [SlotOrderPlan {
                key: SLOT_UNPLACED.into(),
                placement_ids: vec!["cats".into()],
            }]
        );
        assert_eq!(ids(&apply_plan(&current, &plan), SLOT_UNPLACED), ["cats"]);
        let touched = body(&["row:a", "row:b", "row:x"], &[(SLOT_UNPLACED, &["cats"])]);
        assert_eq!(validate_order(&current, &touched).unwrap().slots.len(), 1);
        let unchanged = validate_order(&current, &body(ROWS, &[])).unwrap();
        assert!(unchanged.slots.is_empty());
    }

    #[test]
    fn deleting_a_placement_also_deletes_its_item_rows() {
        let (items, placement) = delete_statements(Edition::Draft, "p1");
        let items = items.build(DbBackend::Postgres).to_string();
        assert!(
            items.starts_with(r#"DELETE FROM "public"."ExplorePlacementItem""#),
            "{items}"
        );
        assert!(items.contains(r#""edition" = 'DRAFT'"#), "{items}");
        assert!(items.contains(r#""placementId" = 'p1'"#), "{items}");
        let placement = placement.build(DbBackend::Postgres).to_string();
        assert!(
            placement.starts_with(r#"DELETE FROM "public"."ExplorePlacement""#),
            "{placement}"
        );
        assert!(placement.contains(r#""id" = 'p1'"#), "{placement}");
    }

    #[test]
    fn diff_reports_added_removed_modified_and_moved() {
        let live = sample();
        let mut draft = sample();
        draft.slots[1].placements[0].name = "Renamed".into();
        draft.slots[3].placements.swap(0, 1);
        draft.slots[4].placements.clear();
        draft.slots[5].placements.push(rail("fresh", RailKey::New));
        let changes: Vec<(String, ChangeKind)> = diff(&draft, &live)
            .into_iter()
            .map(|change| (change.placement_id, change.change))
            .collect();
        assert_eq!(
            changes,
            [
                ("c1".into(), ChangeKind::Modified),
                ("c2".into(), ChangeKind::Moved),
                ("trending".into(), ChangeKind::Moved),
                ("fresh".into(), ChangeKind::Added),
                ("new".into(), ChangeKind::Removed),
            ]
        );
        let mut reordered_rows = sample();
        reordered_rows.slots.swap(3, 4);
        let moved: Vec<String> = diff(&reordered_rows, &live)
            .into_iter()
            .map(|change| change.placement_id)
            .collect();
        assert_eq!(moved, ["new", "trending", "c2"]);
        let mut status_only = sample();
        status_only.fill_statuses(Utc::now());
        assert!(diff(&status_only, &live).is_empty());
        assert!(diff(&default_layout(), &default_layout()).is_empty());
    }

    fn slot_model(key: &str, area: ExploreSlotArea, position: i32) -> explore_slot::Model {
        explore_slot::Model {
            edition: "DRAFT".into(),
            key: key.into(),
            area,
            position,
        }
    }

    fn placement_model(
        id: &str,
        slot_key: &str,
        position: i32,
        content: serde_json::Value,
    ) -> explore_placement::Model {
        let now = Utc::now().fixed_offset();
        explore_placement::Model {
            edition: "DRAFT".into(),
            id: id.into(),
            slot_key: slot_key.into(),
            position,
            kind: ExplorePlacementKind::Rail,
            name: id.into(),
            enabled: true,
            starts_at: None,
            ends_at: None,
            audience: vec!["dev".to_owned()].into(),
            content,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn assemble_orders_slots_and_parks_orphans_as_unplaced() {
        let rail_json = json!({"kind": "rail", "rail": "trending"});
        let (layout, unreadable) = assemble(
            vec![
                slot_model(SLOT_HERO, ExploreSlotArea::Grid, 0),
                slot_model("row:first", ExploreSlotArea::Row, 3),
                slot_model("row:second", ExploreSlotArea::Row, 7),
                slot_model(SLOT_UNPLACED, ExploreSlotArea::Unplaced, 0),
            ],
            vec![
                placement_model("orphan", "row:gone", 0, rail_json.clone()),
                placement_model("r1", "row:first", 0, rail_json.clone()),
                placement_model("broken", "row:first", 1, json!({"kind": "nope"})),
                placement_model("r2", "row:second", 0, rail_json.clone()),
                placement_model("r3", "row:second", 1, rail_json),
            ],
            vec![explore_placement_item::Model {
                edition: "DRAFT".into(),
                placement_id: "r1".into(),
                position: 0,
                item_kind: ExploreItemKind::App,
                item_id: "a1".into(),
                overrides: json!({"headline": "Hi", "accent": "category:Finance"}),
            }],
        );
        let keys: Vec<(&str, i32)> = layout
            .slots
            .iter()
            .map(|slot| (slot.key.as_str(), slot.position))
            .collect();
        assert_eq!(
            keys,
            [
                ("hero", 0),
                ("notice", 1),
                ("feature", 2),
                ("collection", 3),
                ("stat", 4),
                ("categories", 5),
                ("row:first", 0),
                ("row:second", 1),
                ("unplaced", 0),
            ]
        );
        assert_eq!(ids(&layout, "row:first"), ["r1"]);
        assert_eq!(ids(&layout, "row:second"), ["r2", "r3"]);
        assert_eq!(ids(&layout, SLOT_UNPLACED), ["orphan"]);
        assert_eq!(unreadable, ["broken"]);
        let r1 = layout.find("r1").unwrap().1;
        assert_eq!(r1.items[0].kind, ItemKind::App);
        assert_eq!(r1.items[0].headline.as_deref(), Some("Hi"));
        assert_eq!(r1.audience, ["dev"]);
        assert!(r1.updated_at.is_some() && r1.created_at.is_some());
        assert!(
            !serde_json::to_value(r1)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("createdAt")
        );
    }

    #[test]
    fn mutations_refuse_a_layout_with_unreadable_placements() {
        let loaded = |unreadable: &[&str]| Loaded {
            header: None,
            layout: sample(),
            unreadable: unreadable.iter().map(|id| id.to_string()).collect(),
        };
        assert_eq!(
            slot_ids(&loaded(&[]).writable().unwrap()),
            slot_ids(&sample())
        );
        let error = loaded(&["broken", "newer"]).writable().unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
        assert!(
            message(error).starts_with("Explore placements broken, newer cannot be read"),
            "names every unreadable placement"
        );
    }

    /// Run with FLOW_LIKE_EXPLORE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server. The entities
    /// pin the `public` schema, so each run creates its own database and drops it at the end.
    mod database {
        use super::*;
        use chrono::Duration;
        use flow_like_types::tokio;
        use sea_orm::{Database, DatabaseBackend, DatabaseConnection, Statement};

        async fn execute(db: &DatabaseConnection, sql: &str) {
            db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
                .await
                .unwrap();
        }

        struct Fixture {
            db: DatabaseConnection,
            server: String,
            name: String,
        }

        impl Fixture {
            async fn new() -> Self {
                let server = std::env::var("FLOW_LIKE_EXPLORE_TEST_DATABASE_URL")
                    .expect("Use a disposable Explore test database");
                let setup = Database::connect(&server).await.unwrap();
                let name = format!("explore_{}", create_id().to_lowercase());
                execute(&setup, &format!("CREATE DATABASE \"{name}\"")).await;
                setup.close().await.unwrap();
                let mut target = reqwest::Url::parse(&server).unwrap();
                target.set_path(&name);
                let db = Database::connect(target.as_str()).await.unwrap();
                for statement in include_str!(
                    "../../../prisma/migrations/20260924200000_explore_layout/migration.sql"
                )
                .split(';')
                .filter(|statement| !statement.trim().is_empty())
                {
                    execute(&db, statement).await;
                }
                Self { db, server, name }
            }

            async fn drop_database(self) {
                self.db.close().await.unwrap();
                let setup = Database::connect(&self.server).await.unwrap();
                execute(
                    &setup,
                    &format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name),
                )
                .await;
                setup.close().await.unwrap();
            }
        }

        fn assert_conflict<T>(result: Result<T, ApiError>) {
            match result {
                Ok(_) => panic!("expected a 409"),
                Err(error) => {
                    assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
                    assert_eq!(message(error), DRAFT_CHANGED);
                }
            }
        }

        async fn layout_of(db: &DatabaseConnection, edition: Edition) -> LayoutDoc {
            load(db, edition).await.unwrap().layout
        }

        async fn item_rows_of(db: &DatabaseConnection, edition: Edition, id: &str) -> usize {
            explore_placement_item::Entity::find()
                .filter(explore_placement_item::Column::Edition.eq(edition.as_str()))
                .filter(explore_placement_item::Column::PlacementId.eq(id))
                .all(db)
                .await
                .unwrap()
                .len()
        }

        async fn stored(
            db: &DatabaseConnection,
            edition: Edition,
            id: &str,
        ) -> explore_placement::Model {
            explore_placement::Entity::find_by_id((edition.as_str().to_owned(), id.to_owned()))
                .one(db)
                .await
                .unwrap()
                .unwrap()
        }

        async fn positions(
            db: &DatabaseConnection,
            edition: Edition,
            slot_key: &str,
        ) -> Vec<(String, i32)> {
            explore_placement::Entity::find()
                .filter(explore_placement::Column::Edition.eq(edition.as_str()))
                .filter(explore_placement::Column::SlotKey.eq(slot_key))
                .order_by_asc(explore_placement::Column::Position)
                .all(db)
                .await
                .unwrap()
                .into_iter()
                .map(|row| (row.id, row.position))
                .collect()
        }

        async fn row_slots(db: &DatabaseConnection, edition: Edition) -> Vec<(String, i32)> {
            explore_slot::Entity::find()
                .filter(explore_slot::Column::Edition.eq(edition.as_str()))
                .filter(explore_slot::Column::Area.eq(ExploreSlotArea::Row))
                .order_by_asc(explore_slot::Column::Position)
                .all(db)
                .await
                .unwrap()
                .into_iter()
                .map(|slot| (slot.key, slot.position))
                .collect()
        }

        fn pairs(values: &[(&str, i32)]) -> Vec<(String, i32)> {
            values
                .iter()
                .map(|(key, position)| (key.to_string(), *position))
                .collect()
        }

        async fn apply(db: &DatabaseConnection, body: ExploreOrderBody) -> (LayoutDoc, OrderPlan) {
            let draft = layout_of(db, Edition::Draft).await;
            let plan = validate_order(&draft, &body).unwrap();
            apply_order(db, Edition::Draft, &plan).await.unwrap();
            assert_eq!(
                slot_ids(&layout_of(db, Edition::Draft).await),
                slot_ids(&apply_plan(&draft, &plan))
            );
            (draft, plan)
        }

        #[tokio::test]
        #[ignore = "requires a disposable PostgreSQL database"]
        async fn draft_writes_round_trip_through_postgres() {
            let fixture = Fixture::new().await;
            let db = &fixture.db;
            let now = Utc::now();

            let draft = load(db, Edition::Draft).await.unwrap();
            assert!(draft.header.is_none());
            assert_eq!(draft.revision(), DEFAULT_REVISION);
            assert_eq!(slot_ids(&draft.layout), slot_ids(&default_layout()));
            assert_conflict(ensure_draft(db, "stale", now).await);
            ensure_draft(db, DEFAULT_REVISION, now).await.unwrap();
            let revision = cas(db, DEFAULT_REVISION, now).await.unwrap();
            assert_conflict(cas(db, DEFAULT_REVISION, now).await);
            assert_conflict(ensure_draft(db, DEFAULT_REVISION, now).await);
            assert_conflict(ensure_draft(db, "stale", now).await);
            ensure_draft(db, &revision, now).await.unwrap();
            let draft = load(db, Edition::Draft).await.unwrap();
            assert_eq!(draft.revision(), revision);
            assert_eq!(slot_ids(&draft.layout), slot_ids(&default_layout()));

            let mut picks = collection("picks");
            picks.items = ["a1", "a2", "a3"]
                .map(|id| {
                    PlacementItemDoc::from_row(ItemKind::App, id.into(), ItemOverrides::default())
                })
                .to_vec();
            draft
                .layout
                .check_target("row:trending", &picks.content)
                .unwrap();
            assert!(
                draft
                    .layout
                    .check_target("row:ghost", &picks.content)
                    .is_err()
            );
            insert_placement(db, Edition::Draft, "row:trending", 0, &picks, now)
                .await
                .unwrap();
            write_slot_order(
                db,
                Edition::Draft,
                "row:trending",
                &["picks".into(), "default-trending".into()],
            )
            .await
            .unwrap();
            let draft = layout_of(db, Edition::Draft).await;
            assert_eq!(ids(&draft, "row:trending"), ["picks", "default-trending"]);
            assert_eq!(draft.find("picks").unwrap().1.items.len(), 3);

            picks.items.truncate(2);
            picks.name = "Picks".into();
            update_placement(db, Edition::Draft, &picks, now)
                .await
                .unwrap();
            assert_eq!(item_rows_of(db, Edition::Draft, "picks").await, 2);

            let later = now + Duration::seconds(30);
            assert_conflict(publish(db, "stale", later).await);
            let revision = publish(db, &revision, later).await.unwrap();
            let live = load(db, Edition::Live).await.unwrap();
            let live_header = live.header.clone().unwrap();
            assert_ne!(live.revision(), DEFAULT_REVISION);
            assert_ne!(live.revision(), revision);
            assert!(live_header.published_at.is_some());
            let draft = load(db, Edition::Draft).await.unwrap();
            assert_eq!(draft.revision(), revision);
            assert_eq!(slot_ids(&live.layout), slot_ids(&draft.layout));
            assert!(diff(&draft.layout, &live.layout).is_empty());
            let created = stored(db, Edition::Draft, "picks").await.created_at;
            assert_eq!(stored(db, Edition::Live, "picks").await.created_at, created);
            assert!(created < later.fixed_offset());

            assert_eq!(
                delete_placement(db, Edition::Draft, "picks")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("row:trending")
            );
            assert_eq!(item_rows_of(db, Edition::Draft, "picks").await, 0);
            assert_eq!(item_rows_of(db, Edition::Live, "picks").await, 2);
            assert_eq!(
                delete_placement(db, Edition::Draft, "picks").await.unwrap(),
                None
            );
            assert_eq!(
                positions(db, Edition::Draft, "row:trending").await,
                pairs(&[("default-trending", 0)])
            );

            let (_, plan) = apply(
                db,
                ExploreOrderBody {
                    expected_revision: revision.clone(),
                    rows: vec![
                        "row:suites".into(),
                        "row:trending".into(),
                        "row:builders".into(),
                        "row:fresh".into(),
                    ],
                    slots: vec![
                        ExploreSlotOrder {
                            key: "row:top-paid".into(),
                            placement_ids: vec![],
                        },
                        ExploreSlotOrder {
                            key: "row:builders".into(),
                            placement_ids: vec![
                                "default-top-paid".into(),
                                "default-builders".into(),
                            ],
                        },
                        ExploreSlotOrder {
                            key: SLOT_UNPLACED.into(),
                            placement_ids: vec!["default-builders-new".into()],
                        },
                    ],
                },
            )
            .await;
            assert_eq!(plan.removed_rows, ["row:top-paid"]);
            assert_eq!(plan.created_rows, ["row:fresh"]);
            assert_eq!(
                row_slots(db, Edition::Draft).await,
                pairs(&[
                    ("row:suites", 0),
                    ("row:trending", 1),
                    ("row:builders", 2),
                    ("row:fresh", 3)
                ])
            );
            assert_eq!(
                positions(db, Edition::Draft, "row:builders").await,
                pairs(&[("default-top-paid", 0), ("default-builders", 1)])
            );
            assert_eq!(
                positions(db, Edition::Draft, SLOT_UNPLACED).await,
                pairs(&[("default-builders-new", 0)])
            );
            assert!(
                positions(db, Edition::Draft, "row:top-paid")
                    .await
                    .is_empty()
            );

            insert_placement(
                db,
                Edition::Draft,
                "row:ghost",
                0,
                &rail("ghost", RailKey::New),
                now,
            )
            .await
            .unwrap();
            assert_eq!(
                ids(&layout_of(db, Edition::Draft).await, SLOT_UNPLACED),
                ["default-builders-new", "ghost"]
            );
            apply(
                db,
                ExploreOrderBody {
                    expected_revision: revision.clone(),
                    rows: vec![
                        "row:suites".into(),
                        "row:trending".into(),
                        "row:builders".into(),
                        "row:fresh".into(),
                        "row:ghost".into(),
                    ],
                    slots: vec![],
                },
            )
            .await;
            assert!(positions(db, Edition::Draft, "row:ghost").await.is_empty());
            assert_eq!(
                positions(db, Edition::Draft, SLOT_UNPLACED).await,
                pairs(&[("default-builders-new", 0), ("ghost", 1)])
            );

            assert_conflict(discard(db, "stale", now).await);
            let revision = discard(db, &revision, now).await.unwrap();
            let draft = load(db, Edition::Draft).await.unwrap();
            assert_eq!(draft.revision(), revision);
            assert_eq!(slot_ids(&draft.layout), slot_ids(&live.layout));
            assert_eq!(
                load(db, Edition::Live).await.unwrap().header.unwrap(),
                live_header
            );

            for edition in [Edition::Draft, Edition::Live] {
                explore_placement::Entity::insert(
                    explore_placement::ActiveModel::from(explore_placement::Model {
                        edition: edition.as_str().into(),
                        ..placement_model("newer", "row:trending", 9, json!({"kind": "nope"}))
                    })
                    .reset_all(),
                )
                .exec_without_returning(db)
                .await
                .unwrap();
                let loaded = load(db, edition).await.unwrap();
                assert_eq!(loaded.unreadable, ["newer"]);
                assert!(loaded.layout.find("newer").is_none());
            }
            let refused = |result: Result<String, ApiError>| {
                let error = result.unwrap_err();
                assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
                assert!(message(error).starts_with("Explore placements newer cannot be read"));
            };
            refused(publish(db, &revision, now).await);
            refused(discard(db, &revision, now).await);
            assert_eq!(load(db, Edition::Draft).await.unwrap().revision(), revision);
            assert_eq!(
                load(db, Edition::Live).await.unwrap().header.unwrap(),
                live_header
            );
            assert_eq!(stored(db, Edition::Live, "newer").await.position, 9);

            fixture.drop_database().await;
        }
    }
}
