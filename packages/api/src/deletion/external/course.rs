use flow_like_storage::object_store::path::Path;

use super::delete_prefix;
use crate::deletion::drain::{Flow, Pass};
use crate::error::ApiError;
use crate::state::AppState;

/// `media/courses/{id}` on the content store: banner and icon media plus the
/// `assets/` objects referenced by `CourseAsset.storageKey`.
pub async fn delete_media(
    state: &AppState,
    course_id: &str,
    pass: &mut Pass<'_>,
) -> Result<Flow, ApiError> {
    let credentials = state.master_credentials().await?;
    let content = credentials.to_store(false).await?.as_generic();
    let prefix = Path::from("media").child("courses").child(course_id);
    delete_prefix(&content, &prefix, "course media", pass).await
}
