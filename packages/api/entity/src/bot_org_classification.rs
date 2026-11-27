//! A person's classification in an org, keyed by the org's top-level team
//! (`PersonClassification` in the contracts crate). The level is stored lowercase.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "public", table_name = "BotOrgClassification")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false, column_name = "orgId")]
    pub org_id: String,
    #[sea_orm(primary_key, auto_increment = false, column_name = "userId")]
    pub user_id: String,
    pub classification: String,
    #[sea_orm(column_name = "setBy")]
    pub set_by: String,
    #[sea_orm(column_name = "updatedAt")]
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
