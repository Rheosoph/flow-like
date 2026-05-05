use std::{collections::HashMap, time::SystemTime};

use axum::{
    Router,
    routing::{get, post},
};
use flow_like::bit::{Bit, BitTypes, LlmModelEvaluation, Metadata};

use crate::{
    entity::{bit, llm_model, meta::Model, sea_orm_active_enums::BitType},
    state::AppState,
};

pub mod get_bit;
pub mod get_with_dependencies;
pub mod search_bits;

impl From<BitType> for BitTypes {
    fn from(value: BitType) -> Self {
        match value {
            BitType::Llm => BitTypes::Llm,
            BitType::Vlm => BitTypes::Vlm,
            BitType::Embedding => BitTypes::Embedding,
            BitType::ImageEmbedding => BitTypes::ImageEmbedding,
            BitType::File => BitTypes::File,
            BitType::Media => BitTypes::Media,
            BitType::ImageGeneration => BitTypes::ImageGeneration,
            BitType::VideoGeneration => BitTypes::VideoGeneration,
            BitType::Template => BitTypes::Template,
            BitType::Tokenizer => BitTypes::Tokenizer,
            BitType::TokenizerConfig => BitTypes::TokenizerConfig,
            BitType::SpecialTokensMap => BitTypes::SpecialTokensMap,
            BitType::Config => BitTypes::Config,
            BitType::Course => BitTypes::Course,
            BitType::PreprocessorConfig => BitTypes::PreprocessorConfig,
            BitType::Projection => BitTypes::Projection,
            BitType::Project => BitTypes::Project,
            BitType::Board => BitTypes::Board,
            BitType::Other => BitTypes::Other,
            BitType::ObjectDetection => BitTypes::ObjectDetection,
        }
    }
}

impl From<BitTypes> for BitType {
    fn from(value: BitTypes) -> Self {
        match value {
            BitTypes::Llm => BitType::Llm,
            BitTypes::Vlm => BitType::Vlm,
            BitTypes::Embedding => BitType::Embedding,
            BitTypes::ImageEmbedding => BitType::ImageEmbedding,
            BitTypes::File => BitType::File,
            BitTypes::Media => BitType::Media,
            BitTypes::ImageGeneration => BitType::ImageGeneration,
            BitTypes::VideoGeneration => BitType::VideoGeneration,
            BitTypes::Template => BitType::Template,
            BitTypes::Tokenizer => BitType::Tokenizer,
            BitTypes::TokenizerConfig => BitType::TokenizerConfig,
            BitTypes::SpecialTokensMap => BitType::SpecialTokensMap,
            BitTypes::Config => BitType::Config,
            BitTypes::Course => BitType::Course,
            BitTypes::PreprocessorConfig => BitType::PreprocessorConfig,
            BitTypes::Projection => BitType::Projection,
            BitTypes::Project => BitType::Project,
            BitTypes::Board => BitType::Board,
            BitTypes::Other => BitType::Other,
            BitTypes::ObjectDetection => BitType::ObjectDetection,
        }
    }
}

pub(crate) fn llm_model_to_evaluation(model: llm_model::Model) -> LlmModelEvaluation {
    LlmModelEvaluation {
        slug: model.slug,
        name: model.name,
        release_date: model.release_date.map(|d| d.to_string()),
        creator_name: model.creator_name,
        creator_slug: model.creator_slug,
        evaluations: model.evaluations,
        pricing: model.pricing,
        median_output_tokens_per_second: model.median_output_tokens_per_second,
        median_time_to_first_token_seconds: model.median_time_to_first_token_seconds,
        median_time_to_first_answer_token: model.median_time_to_first_answer_token,
    }
}

impl From<bit::Model> for Bit {
    fn from(value: bit::Model) -> Self {
        let created_string = value.created_at.and_utc().to_rfc3339();
        let updated_string = value.updated_at.and_utc().to_rfc3339();
        let id = value.id.clone();
        Bit {
            id: value.id,
            authors: value.authors.unwrap_or_default(),
            bit_type: value.r#type.into(),
            updated: updated_string,
            created: created_string,
            dependencies: value.dependencies.unwrap_or_default(),
            dependency_tree_hash: value.dependency_tree_hash.unwrap_or_else(|| id.clone()),
            download_link: value.download_link,
            license: value.license,
            file_name: value.file_name,
            hash: value.hash.unwrap_or_else(|| id.clone()),
            hub: value.hub,
            meta: HashMap::new(),
            parameters: value.parameters.unwrap_or_default(),
            repository: value.repository,
            size: value.size.map(|s| s as u64),
            version: value.version,
            model_slug: value.model_slug,
            model_evaluation: None,
        }
    }
}

impl From<Bit> for bit::Model {
    fn from(value: Bit) -> Self {
        bit::Model {
            id: value.id,
            authors: Some(value.authors),
            r#type: value.bit_type.into(),
            updated_at: chrono::DateTime::parse_from_rfc3339(&value.updated)
                .unwrap_or_default()
                .naive_utc(),
            created_at: chrono::DateTime::parse_from_rfc3339(&value.created)
                .unwrap_or_default()
                .naive_utc(),
            dependencies: Some(value.dependencies),
            dependency_tree_hash: Some(value.dependency_tree_hash),
            download_link: value.download_link,
            license: value.license,
            file_name: value.file_name,
            hash: Some(value.hash),
            hub: value.hub,
            parameters: Some(value.parameters),
            repository: value.repository,
            size: value.size.map(|s| s as i64),
            version: value.version,
            model_slug: value.model_slug,
        }
    }
}

impl From<Model> for Metadata {
    fn from(model: Model) -> Self {
        Self {
            name: model.name,
            description: model.description.unwrap_or_default(),
            long_description: model.long_description,
            release_notes: model.release_notes,
            tags: model.tags.unwrap_or_default(),
            use_case: model.use_case,
            icon: model.icon,
            thumbnail: model.thumbnail,
            preview_media: model.preview_media.unwrap_or_default(),
            age_rating: model.age_rating.map(|r| r as i32),
            website: model.website,
            support_url: model.support_url,
            docs_url: model.docs_url,
            organization_specific_values: model
                .organization_specific_values
                .map(|json| json.to_string().into_bytes()),
            created_at: SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(model.created_at.and_utc().timestamp() as u64),
            updated_at: SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(model.updated_at.and_utc().timestamp() as u64),
        }
    }
}

impl From<Metadata> for Model {
    fn from(metadata: Metadata) -> Self {
        Model {
            name: metadata.name,
            description: if metadata.description.is_empty() {
                None
            } else {
                Some(metadata.description)
            },
            long_description: metadata.long_description,
            release_notes: metadata.release_notes,
            tags: if metadata.tags.is_empty() {
                None
            } else {
                Some(metadata.tags)
            },
            use_case: metadata.use_case,
            icon: metadata.icon,
            thumbnail: metadata.thumbnail,
            preview_media: if metadata.preview_media.is_empty() {
                None
            } else {
                Some(metadata.preview_media)
            },
            age_rating: metadata.age_rating.map(|r| r as i64),
            website: metadata.website,
            support_url: metadata.support_url,
            docs_url: metadata.docs_url,
            app_id: None,
            template_id: None,
            bit_id: None,
            course_id: None,
            widget_id: None,
            wasm_package_id: None,
            id: "".to_string(),
            lang: "".to_string(),
            organization_specific_values: metadata
                .organization_specific_values
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|s| serde_json::from_str(&s).ok()),
            created_at: chrono::DateTime::from_timestamp(
                metadata
                    .created_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                0,
            )
            .unwrap_or_default()
            .naive_utc(),
            updated_at: chrono::DateTime::from_timestamp(
                metadata
                    .updated_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                0,
            )
            .unwrap_or_default()
            .naive_utc(),
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", post(search_bits::search_bits))
        .route("/{bit_id}", get(get_bit::get_bit))
        .route(
            "/{bit_id}/dependencies",
            get(get_with_dependencies::get_with_dependencies),
        )
}
