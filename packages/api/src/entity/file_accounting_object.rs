//! `SeaORM` entity for durable AWS object accounting.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(schema_name = "public", table_name = "FileAccountingObject")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub id: String,
    pub bucket: String,
    #[sea_orm(column_name = "objectKey")]
    pub object_key: String,
    #[sea_orm(column_name = "appId")]
    pub app_id: String,
    #[sea_orm(column_name = "userId")]
    pub user_id: Option<String>,
    pub size: i64,
    pub sequencer: String,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: DateTimeWithTimeZone,
}

impl ActiveModelBehavior for ActiveModel {}
