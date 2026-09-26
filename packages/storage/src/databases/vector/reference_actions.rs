use flow_like_storage_contracts::database::{
    DatabaseAction, DatabaseActionResult, DatabaseHistory, DatabaseSelector,
};
use flow_like_types::Result;

use super::lancedb::LanceDBVectorStore;

impl LanceDBVectorStore {
    pub async fn history(&self) -> Result<DatabaseHistory> {
        Ok(DatabaseHistory {
            reference: self.reference().await?,
            versions: self.list_versions().await?,
            branches: self.list_branches().await?,
            tags: self.list_tags().await?,
        })
    }

    pub async fn reference_action(&self, action: DatabaseAction) -> Result<DatabaseActionResult> {
        let mut cleanup = None;
        let reference = match action {
            DatabaseAction::CreateBranch { name } => {
                self.create_branch(&name).await?.reference().await?
            }
            DatabaseAction::DeleteBranch { name } => {
                self.delete_branch(&name).await?;
                self.reference().await?
            }
            DatabaseAction::CreateTag { name } => {
                self.create_tag(&name).await?;
                self.reference().await?
            }
            DatabaseAction::UpdateTag { name } => {
                self.update_tag(&name).await?;
                self.reference().await?
            }
            DatabaseAction::DeleteTag { name } => {
                self.delete_tag(&name).await?;
                self.reference().await?
            }
            DatabaseAction::Restore => self.restore().await?,
            DatabaseAction::Snapshot { name } => {
                let reference = self.reference().await?;
                let snapshot = self
                    .checkout(DatabaseSelector {
                        branch: reference.branch,
                        version: Some(reference.version),
                        tag: None,
                        read_only: self.selector().read_only,
                    })
                    .await?;
                if let Some(name) = name {
                    snapshot.create_tag(&name).await?;
                }
                snapshot.reference().await?
            }
            DatabaseAction::Cleanup { older_than_days } => {
                cleanup = Some(self.cleanup_versions(older_than_days).await?);
                self.reference().await?
            }
            DatabaseAction::Clone { name } => self.clone_table(&name).await?.reference().await?,
        };
        Ok(DatabaseActionResult { reference, cleanup })
    }
}
