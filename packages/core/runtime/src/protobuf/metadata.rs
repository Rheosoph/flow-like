use std::time::SystemTime;

use crate::{app::AppCategory, bit::Metadata};
use flow_like_types::{FromProto, Timestamp, ToProto};

impl ToProto<flow_like_types::proto::Metadata> for Metadata {
    fn to_proto(&self) -> flow_like_types::proto::Metadata {
        flow_like_types::proto::Metadata {
            name: self.name.clone(),
            description: self.description.clone(),
            long_description: self.long_description.clone(),
            release_notes: self.release_notes.clone(),
            tags: self.tags.clone(),
            use_case: self.use_case.clone(),
            icon: self.icon.clone(),
            thumbnail: self.thumbnail.clone(),
            preview_media: self.preview_media.clone(),
            age_rating: self.age_rating,
            website: self.website.clone(),
            support_url: self.support_url.clone(),
            docs_url: self.docs_url.clone(),
            organization_specific_values: self.organization_specific_values.clone(),
            created_at: Some(Timestamp::from(self.created_at)),
            updated_at: Some(Timestamp::from(self.updated_at)),
        }
    }
}

impl FromProto<flow_like_types::proto::Metadata> for Metadata {
    fn from_proto(proto: flow_like_types::proto::Metadata) -> Self {
        Metadata {
            name: proto.name,
            description: proto.description,
            long_description: proto.long_description,
            release_notes: proto.release_notes,
            tags: proto.tags,
            use_case: proto.use_case,
            icon: proto.icon,
            thumbnail: proto.thumbnail,
            preview_media: proto.preview_media,
            age_rating: proto.age_rating,
            website: proto.website,
            support_url: proto.support_url,
            docs_url: proto.docs_url,
            organization_specific_values: proto.organization_specific_values,
            created_at: proto
                .created_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            updated_at: proto
                .updated_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
        }
    }
}

impl From<AppCategory> for flow_like_types::proto::AppCategory {
    fn from(category: AppCategory) -> Self {
        flow_like_types::proto::AppCategory::try_from(category as i32)
            .unwrap_or(flow_like_types::proto::AppCategory::Other)
    }
}

impl TryFrom<i32> for AppCategory {
    type Error = ();

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AppCategory::Other),
            1 => Ok(AppCategory::Productivity),
            2 => Ok(AppCategory::Social),
            3 => Ok(AppCategory::Entertainment),
            4 => Ok(AppCategory::Education),
            5 => Ok(AppCategory::Health),
            6 => Ok(AppCategory::Finance),
            7 => Ok(AppCategory::Lifestyle),
            8 => Ok(AppCategory::Travel),
            9 => Ok(AppCategory::News),
            10 => Ok(AppCategory::Sports),
            11 => Ok(AppCategory::Shopping),
            12 => Ok(AppCategory::FoodAndDrink),
            13 => Ok(AppCategory::Music),
            14 => Ok(AppCategory::Photography),
            15 => Ok(AppCategory::Utilities),
            16 => Ok(AppCategory::Weather),
            17 => Ok(AppCategory::Games),
            18 => Ok(AppCategory::Business),
            19 => Ok(AppCategory::Communication),
            20 => Ok(AppCategory::Anime),
            _ => Err(()),
        }
    }
}

impl From<flow_like_types::proto::AppCategory> for AppCategory {
    fn from(proto: flow_like_types::proto::AppCategory) -> Self {
        AppCategory::try_from(proto as i32).unwrap_or(AppCategory::Other)
    }
}
