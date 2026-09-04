use flow_like_storage::object_store::path::Path;
use sea_orm::EntityTrait;

use crate::entity::bit;
use crate::error::ApiError;
use crate::state::AppState;

/// The CDN object named by `Bit.hash`; needs the row, so it runs first.
pub async fn delete_cdn_artifact(state: &AppState, bit_id: &str) -> Result<(), ApiError> {
    let Some(bit) = bit::Entity::find_by_id(bit_id).one(&state.db).await? else {
        return Ok(());
    };
    let Some(hash) = bit.hash.filter(|hash| !hash.is_empty()) else {
        return Ok(());
    };
    let path = Path::from("bits").child(hash.as_str());
    match state.cdn_bucket.as_generic().delete(&path).await {
        Ok(()) => Ok(()),
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
