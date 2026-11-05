use super::*;
use crate::native_auth::SessionBinding;
use serde::Serialize;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Summary {
    pub scopes: usize,
    pub pending_changes: u64,
    pub conflicts: Vec<String>,
}

fn pending(root: &Path, descriptor: &ScopeDescriptor) -> Result<u64> {
    Ok(
        Outbox::open_read_only(&engine_dir(root, &descriptor.app_id, &descriptor.scope))?
            .map(|reader| reader.status().map(|status| status.pending_count))
            .transpose()?
            .unwrap_or(0),
    )
}

fn owned(descriptor: &ScopeDescriptor, binding: &SessionBinding) -> ScopeKey {
    ScopeKey {
        profile_id: Some(binding.profile_id.clone()),
        issuer: Some(binding.issuer.clone()),
        hub: descriptor.hub.clone(),
        subject: descriptor.subject.clone(),
        app_id: descriptor.app_id.clone(),
    }
}

impl OfflineWrites {
    fn recovery_target_exists(
        &self,
        target: &ScopeKey,
        descriptors: &[ScopeDescriptor],
    ) -> Result<bool> {
        let id = owned_scope_id(target, &self.installation_id()?);
        Ok(descriptors
            .iter()
            .any(|candidate| candidate.key() == *target)
            || descriptor_path(&self.root, &id).exists()
            || engine_dir(&self.root, &target.app_id, &id).exists()
            || scope_dir(&self.root, &id).exists())
    }

    pub(crate) fn legacy_summary(&self, binding: &SessionBinding) -> Result<Summary> {
        let descriptors = self.descriptors();
        let mut summary = Summary::default();
        for descriptor in descriptors.iter().filter(|descriptor| {
            descriptor.profile_id.is_none()
                && descriptor.hub == binding.hub
                && descriptor.subject == binding.subject
        }) {
            let target = owned(descriptor, binding);
            if self.recovery_target_exists(&target, &descriptors)? {
                summary.conflicts.push(descriptor.app_id.clone());
                continue;
            }
            summary.scopes += 1;
            summary.pending_changes = summary
                .pending_changes
                .saturating_add(pending(&self.root, descriptor)?);
        }
        Ok(summary)
    }

    pub(crate) async fn recover_legacy(
        &self,
        app: &AppHandle,
        binding: &SessionBinding,
    ) -> Result<Summary> {
        let (summary, reopen) = self.recover_legacy_in_place(binding).await?;
        let dirs = scope::cache_dirs(app).await?;
        for key in reopen {
            let scope = self.obtain(Some(app.clone()), dirs.clone(), key)?;
            scope.manager().await?;
            scope.wake();
        }
        Ok(summary)
    }

    pub(crate) async fn recover_legacy_in_place(
        &self,
        binding: &SessionBinding,
    ) -> Result<(Summary, Vec<ScopeKey>)> {
        let guard = self.begin_mutation()?;
        let mut summary = Summary::default();
        let mut reopen = Vec::new();
        let descriptors = self.descriptors();
        for mut descriptor in descriptors
            .iter()
            .filter(|descriptor| {
                descriptor.profile_id.is_none()
                    && descriptor.hub == binding.hub
                    && descriptor.subject == binding.subject
            })
            .cloned()
        {
            ensure!(
                binding.is_current(),
                "The profile authentication changed during recovery"
            );
            let previous = descriptor.key();
            let target = owned(&descriptor, binding);
            let conflict = self.recovery_target_exists(&target, &descriptors)?;
            let active = self.scopes().iter().any(|(key, scope)| {
                (key == &previous || key == &target) && scope.has_hosted_runs()
            });
            if conflict || active {
                summary.conflicts.push(descriptor.app_id.clone());
                continue;
            }
            let closing: Vec<_> = {
                let mut scopes = self.scopes();
                [previous, target.clone()]
                    .iter()
                    .filter_map(|key| scopes.remove(key))
                    .collect()
            };
            for scope in closing {
                scope.close().await;
            }
            // A manager acquired before recovery may have opened its files while closing.
            // Preserve both stores if any target data appeared. The older engine never moves.
            if self.recovery_target_exists(&target, &self.descriptors())? {
                summary.conflicts.push(descriptor.app_id.clone());
                continue;
            }
            ensure!(
                binding.is_current(),
                "The profile authentication changed during recovery"
            );
            summary.pending_changes = summary
                .pending_changes
                .saturating_add(pending(&self.root, &descriptor)?);
            descriptor.profile_id = Some(binding.profile_id.clone());
            descriptor.issuer = Some(binding.issuer.clone());
            scope::write_descriptor(&self.root, &descriptor)?;
            summary.scopes += 1;
            reopen.push(target);
        }
        drop(guard);
        Ok((summary, reopen))
    }
}
