use flow_like::flow::board::Board;
use flow_like_storage::object_store::{Error as ObjectStoreError, path::Path};
use flow_like_types::anyhow;
use sea_orm::EntityTrait;

use super::delete_prefix;
use crate::deletion::drain::{Flow, Pass};
use crate::entity::template;
use crate::error::ApiError;
use crate::state::AppState;

/// The template's board file, its version archive and its page payloads, all
/// under the owning app's meta prefix.
///
/// Runs as an `after_drain` step so the objects outlive the row: a template
/// whose board vanished while the row was still listed reads as live but
/// cannot be opened.
pub async fn delete_storage(
    state: &AppState,
    template_id: &str,
    pass: &mut Pass<'_>,
) -> Result<Flow, ApiError> {
    // The row is deleted by `Step::DeleteRoot`, one phase later, so it is
    // still readable here; a resumed job that already passed this phase has
    // nothing left to sweep.
    let Some(row) = template::Entity::find_by_id(template_id)
        .one(&state.db)
        .await?
    else {
        return Ok(Flow::Continue);
    };

    let credentials = state.master_credentials().await?;
    let meta = credentials.to_store(true).await?.as_generic();
    let app_dir = Path::from("apps").child(row.app_id);

    let board = app_dir.child(format!("{template_id}.template"));
    match meta.delete(&board).await {
        Ok(()) | Err(ObjectStoreError::NotFound { .. }) => {}
        Err(error) => {
            return Err(ApiError::internal_error(anyhow!(
                "delete template board {board}: {error}"
            )));
        }
    }

    let versions = Board::versioned_template_dir(&app_dir, template_id);
    if delete_prefix(&meta, &versions, "template versions", pass).await? == Flow::Suspend {
        return Ok(Flow::Suspend);
    }

    // A template that never had pages has no directory at all, which a
    // filesystem store reports as an error rather than an empty listing.
    // Leaving payloads behind is a leak; parking the job at `FAILED` over
    // their absence would strand the whole template.
    let pages = Board::template_pages_dir(&app_dir, template_id);
    match delete_prefix(&meta, &pages, "template pages", pass).await {
        Ok(flow) => Ok(flow),
        Err(error) => {
            tracing::warn!(
                template_id,
                error = %error,
                "Sweeping the page payloads of a deleted template failed"
            );
            Ok(Flow::Continue)
        }
    }
}
