//! The entity table: one row per person, place, organization, project or thing the facts are
//! about, with its alias keys. Small enough to read whole; no vectors.

use flow_like_storage::arrow_schema::Schema;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::Result;
use serde::{Deserialize, Serialize};

use super::lance::{Table, from_millis, in_list, int_field, millis, quote, text_field};
use crate::entities::{Entity, EntityKind};

const COLUMNS: [&str; 8] = [
    "id",
    "name",
    "kind",
    "aliases",
    "named",
    "merged_into",
    "created_at",
    "updated_at",
];

fn schema() -> Schema {
    Schema::new(vec![
        text_field("id", false),
        text_field("name", false),
        text_field("kind", false),
        text_field("aliases", false),
        int_field("named", false),
        text_field("merged_into", true),
        int_field("created_at", false),
        int_field("updated_at", false),
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EntityRow {
    id: String,
    name: String,
    kind: String,
    aliases: String,
    named: i64,
    merged_into: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl EntityRow {
    fn new(entity: &Entity) -> Result<Self> {
        Ok(Self {
            id: entity.id.clone(),
            name: entity.name.clone(),
            kind: entity.kind.as_str().to_string(),
            aliases: serde_json::to_string(&entity.aliases)?,
            named: i64::from(entity.named),
            merged_into: entity.merged_into.clone(),
            created_at: millis(entity.created_at),
            updated_at: millis(entity.updated_at),
        })
    }

    fn into_entity(self) -> Result<Entity> {
        Ok(Entity {
            id: self.id,
            name: self.name,
            kind: EntityKind::parse(&self.kind).unwrap_or_default(),
            aliases: serde_json::from_str(&self.aliases)?,
            named: self.named != 0,
            merged_into: self.merged_into,
            created_at: from_millis(self.created_at),
            updated_at: from_millis(self.updated_at),
        })
    }
}

pub struct EntityStore {
    table: Table,
}

impl EntityStore {
    pub const TABLE: &'static str = "memory_entities";

    pub(crate) async fn open(connection: Connection, write_options: Option<WriteOptions>) -> Self {
        Self::open_named(connection, write_options, Self::TABLE.to_string()).await
    }

    pub(crate) async fn open_named(
        connection: Connection,
        write_options: Option<WriteOptions>,
        name: String,
    ) -> Self {
        Self {
            table: Table::open(connection, name, None, write_options).await,
        }
    }

    /// Every entity, merged ones included (their ids still resolve), oldest first.
    pub async fn all(&self) -> Result<Vec<Entity>> {
        let rows: Vec<EntityRow> = self.table.all_rows("1 = 1", &COLUMNS).await?;
        let mut entities = rows
            .into_iter()
            .map(EntityRow::into_entity)
            .collect::<Result<Vec<_>>>()?;
        entities.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(entities)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Entity>> {
        let rows: Vec<EntityRow> = self
            .table
            .rows(&format!("id = {}", quote(id)), &COLUMNS, 1, 0)
            .await?;
        rows.into_iter()
            .next()
            .map(EntityRow::into_entity)
            .transpose()
    }

    pub async fn upsert(&self, entities: &[Entity]) -> Result<()> {
        if entities.is_empty() {
            return Ok(());
        }
        let rows = entities
            .iter()
            .map(EntityRow::new)
            .collect::<Result<Vec<_>>>()?;
        let _writes = self.table.write_lock().await;
        self.table.create(schema()).await?;
        for rows in rows.chunks(256) {
            self.table.upsert(rows).await?;
        }
        Ok(())
    }

    pub async fn delete(&self, ids: &[String]) -> Result<()> {
        let _writes = self.table.write_lock().await;
        for chunk in ids.chunks(100) {
            self.table
                .delete(&in_list("id", chunk.iter().map(String::as_str)))
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn optimize(&self) -> Result<()> {
        self.table.optimize().await
    }

    pub(crate) async fn prune(&self) -> Result<()> {
        self.table.prune().await
    }

    pub(crate) async fn refresh(&self) -> Result<()> {
        self.table.refresh().await
    }

    pub(crate) async fn drop_all(&self) -> Result<()> {
        self.table.drop_table().await
    }

    pub(crate) async fn clear(&self) -> Result<()> {
        self.table.clear().await
    }
}
