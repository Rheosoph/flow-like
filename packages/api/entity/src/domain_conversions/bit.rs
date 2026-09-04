use std::collections::HashMap;
use std::time::SystemTime;

use crate::{bit, meta, sea_orm_active_enums::BitType};
use flow_like::bit::{Bit, BitTypes, Metadata};

impl From<BitType> for BitTypes {
    fn from(value: BitType) -> Self {
        match value {
            BitType::Llm => Self::Llm,
            BitType::Vlm => Self::Vlm,
            BitType::Tts => Self::Tts,
            BitType::Stt => Self::Stt,
            BitType::Embedding => Self::Embedding,
            BitType::ImageEmbedding => Self::ImageEmbedding,
            BitType::File => Self::File,
            BitType::Media => Self::Media,
            BitType::ImageGeneration => Self::ImageGeneration,
            BitType::VideoGeneration => Self::VideoGeneration,
            BitType::Template => Self::Template,
            BitType::Tokenizer => Self::Tokenizer,
            BitType::TokenizerConfig => Self::TokenizerConfig,
            BitType::SpecialTokensMap => Self::SpecialTokensMap,
            BitType::Config => Self::Config,
            BitType::Course => Self::Course,
            BitType::PreprocessorConfig => Self::PreprocessorConfig,
            BitType::Projection => Self::Projection,
            BitType::Project => Self::Project,
            BitType::Board => Self::Board,
            BitType::Other => Self::Other,
            BitType::ObjectDetection => Self::ObjectDetection,
        }
    }
}

impl From<BitTypes> for BitType {
    fn from(value: BitTypes) -> Self {
        match value {
            BitTypes::Llm => Self::Llm,
            BitTypes::Vlm => Self::Vlm,
            BitTypes::Tts => Self::Tts,
            BitTypes::Stt => Self::Stt,
            BitTypes::Embedding => Self::Embedding,
            BitTypes::ImageEmbedding => Self::ImageEmbedding,
            BitTypes::File => Self::File,
            BitTypes::Media => Self::Media,
            BitTypes::ImageGeneration => Self::ImageGeneration,
            BitTypes::VideoGeneration => Self::VideoGeneration,
            BitTypes::Template => Self::Template,
            BitTypes::Tokenizer => Self::Tokenizer,
            BitTypes::TokenizerConfig => Self::TokenizerConfig,
            BitTypes::SpecialTokensMap => Self::SpecialTokensMap,
            BitTypes::Config => Self::Config,
            BitTypes::Course => Self::Course,
            BitTypes::PreprocessorConfig => Self::PreprocessorConfig,
            BitTypes::Projection => Self::Projection,
            BitTypes::Project => Self::Project,
            BitTypes::Board => Self::Board,
            BitTypes::Other => Self::Other,
            BitTypes::ObjectDetection => Self::ObjectDetection,
        }
    }
}

impl From<bit::Model> for Bit {
    fn from(value: bit::Model) -> Self {
        let created = value.created_at.to_rfc3339();
        let updated = value.updated_at.to_rfc3339();
        let id = value.id.clone();

        Self {
            id: value.id,
            authors: value.authors.unwrap_or_default().into_inner(),
            bit_type: value.r#type.into(),
            updated,
            created,
            dependencies: value.dependencies.unwrap_or_default().into_inner(),
            dependency_tree_hash: value.dependency_tree_hash.unwrap_or_else(|| id.clone()),
            download_link: value.download_link,
            license: value.license,
            file_name: value.file_name,
            hash: value.hash.unwrap_or_else(|| id.clone()),
            hub: value.hub,
            meta: HashMap::new(),
            parameters: value.parameters.unwrap_or_default(),
            repository: value.repository,
            size: value.size.map(|size| size as u64),
            version: value.version,
            model_slug: value.model_slug,
            model_evaluation: None,
        }
    }
}

impl From<Bit> for bit::Model {
    fn from(value: Bit) -> Self {
        Self {
            id: value.id,
            authors: Some(value.authors.into()),
            r#type: value.bit_type.into(),
            updated_at: chrono::DateTime::parse_from_rfc3339(&value.updated).unwrap_or_default(),
            created_at: chrono::DateTime::parse_from_rfc3339(&value.created).unwrap_or_default(),
            dependencies: Some(value.dependencies.into()),
            dependency_tree_hash: Some(value.dependency_tree_hash),
            download_link: value.download_link,
            license: value.license,
            file_name: value.file_name,
            hash: Some(value.hash),
            hub: value.hub,
            parameters: Some(value.parameters),
            repository: value.repository,
            size: value.size.map(|size| size as i64),
            version: value.version,
            model_slug: value.model_slug,
        }
    }
}

impl From<meta::Model> for Metadata {
    fn from(model: meta::Model) -> Self {
        Self {
            name: model.name,
            description: model.description.unwrap_or_default(),
            long_description: model.long_description,
            release_notes: model.release_notes,
            tags: model.tags.unwrap_or_default().into_inner(),
            use_case: model.use_case,
            icon: model.icon,
            thumbnail: model.thumbnail,
            preview_media: model.preview_media.unwrap_or_default().into_inner(),
            age_rating: model.age_rating.map(|rating| rating as i32),
            website: model.website,
            support_url: model.support_url,
            docs_url: model.docs_url,
            organization_specific_values: model
                .organization_specific_values
                .map(|json| json.to_string().into_bytes()),
            created_at: SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(model.created_at.timestamp() as u64),
            updated_at: SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(model.updated_at.timestamp() as u64),
        }
    }
}

impl From<Metadata> for meta::Model {
    fn from(metadata: Metadata) -> Self {
        Self {
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
                Some(metadata.tags.into())
            },
            use_case: metadata.use_case,
            icon: metadata.icon,
            thumbnail: metadata.thumbnail,
            preview_media: if metadata.preview_media.is_empty() {
                None
            } else {
                Some(metadata.preview_media.into())
            },
            age_rating: metadata.age_rating.map(|rating| rating as i64),
            website: metadata.website,
            support_url: metadata.support_url,
            docs_url: metadata.docs_url,
            app_id: None,
            template_id: None,
            bit_id: None,
            course_id: None,
            widget_id: None,
            wasm_package_id: None,
            group_id: None,
            id: String::new(),
            lang: String::new(),
            organization_specific_values: metadata
                .organization_specific_values
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|json| serde_json::from_str(&json).ok()),
            created_at: chrono::DateTime::from_timestamp(
                metadata
                    .created_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                0,
            )
            .unwrap_or_default()
            .fixed_offset(),
            updated_at: chrono::DateTime::from_timestamp(
                metadata
                    .updated_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                0,
            )
            .unwrap_or_default()
            .fixed_offset(),
        }
    }
}
