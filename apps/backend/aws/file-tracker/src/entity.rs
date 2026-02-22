use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "Category")]
pub enum Category {
    #[sea_orm(string_value = "OTHER")]
    Other,
    #[sea_orm(string_value = "PRODUCTIVITY")]
    Productivity,
    #[sea_orm(string_value = "SOCIAL")]
    Social,
    #[sea_orm(string_value = "ENTERTAINMENT")]
    Entertainment,
    #[sea_orm(string_value = "EDUCATION")]
    Education,
    #[sea_orm(string_value = "HEALTH")]
    Health,
    #[sea_orm(string_value = "FINANCE")]
    Finance,
    #[sea_orm(string_value = "LIFESTYLE")]
    Lifestyle,
    #[sea_orm(string_value = "TRAVEL")]
    Travel,
    #[sea_orm(string_value = "NEWS")]
    News,
    #[sea_orm(string_value = "SPORTS")]
    Sports,
    #[sea_orm(string_value = "SHOPPING")]
    Shopping,
    #[sea_orm(string_value = "FOOD_AND_DRINK")]
    FoodAndDrink,
    #[sea_orm(string_value = "MUSIC")]
    Music,
    #[sea_orm(string_value = "PHOTOGRAPHY")]
    Photography,
    #[sea_orm(string_value = "UTILITIES")]
    Utilities,
    #[sea_orm(string_value = "WEATHER")]
    Weather,
    #[sea_orm(string_value = "GAMES")]
    Games,
    #[sea_orm(string_value = "BUSINESS")]
    Business,
    #[sea_orm(string_value = "COMMUNICATION")]
    Communication,
    #[sea_orm(string_value = "ANIME")]
    Anime,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "ExecutionMode")]
pub enum ExecutionMode {
    #[sea_orm(string_value = "ANY")]
    Any,
    #[sea_orm(string_value = "LOCAL")]
    Local,
    #[sea_orm(string_value = "REMOTE")]
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "Status")]
pub enum Status {
    #[sea_orm(string_value = "ACTIVE")]
    Active,
    #[sea_orm(string_value = "INACTIVE")]
    Inactive,
    #[sea_orm(string_value = "ARCHIVED")]
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "Visibility")]
pub enum Visibility {
    #[sea_orm(string_value = "PUBLIC")]
    Public,
    #[sea_orm(string_value = "PUBLIC_REQUEST_ACCESS")]
    PublicRequestAccess,
    #[sea_orm(string_value = "PRIVATE")]
    Private,
    #[sea_orm(string_value = "PROTOTYPE")]
    Prototype,
    #[sea_orm(string_value = "OFFLINE")]
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "UserStatus")]
pub enum UserStatus {
    #[sea_orm(string_value = "ACTIVE")]
    Active,
    #[sea_orm(string_value = "INACTIVE")]
    Inactive,
    #[sea_orm(string_value = "BANNED")]
    Banned,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "UserTier")]
pub enum UserTier {
    #[sea_orm(string_value = "FREE")]
    Free,
    #[sea_orm(string_value = "PREMIUM")]
    Premium,
    #[sea_orm(string_value = "PRO")]
    Pro,
    #[sea_orm(string_value = "ENTERPRISE")]
    Enterprise,
}

pub mod app {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(schema_name = "public", table_name = "App")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
        pub id: String,
        pub status: Status,
        pub visibility: Visibility,
        #[sea_orm(column_type = "Text", nullable)]
        pub changelog: Option<String>,
        #[sea_orm(column_name = "defaultRoleId", column_type = "Text", nullable, unique)]
        pub default_role_id: Option<String>,
        #[sea_orm(column_name = "ownerRoleId", column_type = "Text", nullable, unique)]
        pub owner_role_id: Option<String>,
        #[sea_orm(column_name = "primaryCategory")]
        pub primary_category: Option<Category>,
        #[sea_orm(column_name = "secondaryCategory")]
        pub secondary_category: Option<Category>,
        #[sea_orm(column_name = "ratingSum")]
        pub rating_sum: i64,
        #[sea_orm(column_name = "ratingCount")]
        pub rating_count: i64,
        #[sea_orm(column_name = "downloadCount")]
        pub download_count: i64,
        #[sea_orm(column_name = "interactionsCount")]
        pub interactions_count: i64,
        #[sea_orm(column_name = "avgRating", column_type = "Double", nullable)]
        pub avg_rating: Option<f64>,
        #[sea_orm(column_name = "relevanceScore", column_type = "Double", nullable)]
        pub relevance_score: Option<f64>,
        #[sea_orm(column_name = "totalSize")]
        pub total_size: i64,
        pub price: i64,
        #[sea_orm(column_type = "Text", nullable)]
        pub version: Option<String>,
        #[sea_orm(column_name = "executionMode")]
        pub execution_mode: ExecutionMode,
        pub bits: Option<Vec<String>>,
        #[sea_orm(column_name = "createdAt")]
        pub created_at: DateTime,
        #[sea_orm(column_name = "updatedAt")]
        pub updated_at: DateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod user {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(schema_name = "public", table_name = "User")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
        pub id: String,
        #[sea_orm(column_name = "stripeId", column_type = "Text", nullable, unique)]
        pub stripe_id: Option<String>,
        #[sea_orm(column_name = "trackingId", column_type = "Text", nullable, unique)]
        pub tracking_id: Option<String>,
        #[sea_orm(column_type = "Text", nullable, unique)]
        pub email: Option<String>,
        #[sea_orm(column_type = "Text", nullable, unique)]
        pub username: Option<String>,
        #[sea_orm(column_name = "preferredUsername", column_type = "Text", nullable, unique)]
        pub preferred_username: Option<String>,
        #[sea_orm(column_type = "Text", nullable)]
        pub name: Option<String>,
        #[sea_orm(column_type = "Text", nullable)]
        pub description: Option<String>,
        #[sea_orm(column_type = "Text", nullable)]
        pub avatar: Option<String>,
        #[sea_orm(column_name = "additionalInformation", column_type = "JsonBinary", nullable)]
        pub additional_information: Option<Json>,
        pub permission: i64,
        #[sea_orm(column_name = "acceptedTermsVersion", column_type = "Text", nullable)]
        pub accepted_terms_version: Option<String>,
        #[sea_orm(column_name = "tutorialCompleted")]
        pub tutorial_completed: bool,
        pub status: UserStatus,
        pub tier: UserTier,
        #[sea_orm(column_name = "totalSize")]
        pub total_size: i64,
        #[sea_orm(column_name = "totalLLMPrice")]
        pub total_llm_price: i64,
        #[sea_orm(column_name = "totalEmbeddingPrice")]
        pub total_embedding_price: i64,
        #[sea_orm(column_name = "llmPriceTrackingMonth", column_type = "Text", nullable)]
        pub llm_price_tracking_month: Option<String>,
        #[sea_orm(column_name = "createdAt")]
        pub created_at: DateTime,
        #[sea_orm(column_name = "updatedAt")]
        pub updated_at: DateTime,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
