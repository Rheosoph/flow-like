use flow_like::{models::llm::ExecutionSettings, profile::Profile as FlowLikeProfile};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct UserProfile {
    #[serde(default)]
    pub hub_profile: FlowLikeProfile,

    #[serde(default)]
    pub execution_settings: ExecutionSettings,

    pub updated: String,
    pub created: String,
}

impl UserProfile {
    pub fn new(profile: FlowLikeProfile) -> Self {
        UserProfile {
            hub_profile: profile,
            execution_settings: ExecutionSettings::new(),
            updated: String::new(),
            created: String::new(),
        }
    }

    pub fn merge_synced(
        &mut self,
        mut incoming: Self,
        expected_updated: &str,
        remote_updated: &str,
    ) -> bool {
        if self.hub_profile.updated != expected_updated {
            // A local edit arrived after the upload snapshot. Preserve it and
            // advance past the server acknowledgement so the next sync sends it.
            self.advance_revision(Some(remote_updated));
            return false;
        }
        incoming.hub_profile.custom_bits = self.hub_profile.custom_bits.clone();
        *self = incoming;
        true
    }

    pub fn advance_revision(&mut self, remote_updated: Option<&str>) {
        let newest = std::iter::once(self.hub_profile.updated.as_str())
            .chain(remote_updated)
            .filter_map(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .fold(chrono::Utc::now().fixed_offset(), std::cmp::max);
        let updated = (newest + chrono::Duration::milliseconds(1)).to_rfc3339();
        self.hub_profile.updated = updated.clone();
        self.updated = updated;
    }

    pub fn prune_missing_bits(
        &mut self,
        snapshot: &FlowLikeProfile,
        missing: &[String],
        custom_bit_ids: &HashSet<String>,
    ) -> Vec<String> {
        let profile = &mut self.hub_profile;
        if profile.updated != snapshot.updated
            || profile.id != snapshot.id
            || profile.hub != snapshot.hub
            || profile.hubs != snapshot.hubs
            || profile.secure != snapshot.secure
            || profile.bits != snapshot.bits
        {
            return Vec::new();
        }

        let missing: HashSet<&str> = missing.iter().map(String::as_str).collect();
        let mut removed = Vec::new();
        profile.bits.retain(|reference| {
            let bit_id = reference
                .rsplit_once(':')
                .map_or(reference.as_str(), |(_, id)| id);
            let is_custom = custom_bit_ids.contains(bit_id)
                || profile
                    .custom_bits
                    .iter()
                    .any(|custom| custom.0.id == bit_id);
            if missing.contains(reference.as_str()) && !is_custom {
                removed.push(reference.clone());
                false
            } else {
                true
            }
        });
        if !removed.is_empty() {
            removed.sort();
            removed.dedup();
            self.advance_revision(None);
        }
        removed
    }
}

#[cfg(test)]
mod profile_sync_tests {
    use super::*;

    fn profile_with_bits() -> UserProfile {
        UserProfile::new(FlowLikeProfile {
            hub: "https://hub.test".into(),
            bits: vec!["https://hub.test:missing".into(), "missing".into()],
            updated: "2099-09-05T10:00:00Z".into(),
            ..Default::default()
        })
    }

    #[test]
    fn pruning_removes_only_confirmed_references_and_advances_the_revision() {
        let mut profile = profile_with_bits();
        let snapshot = profile.hub_profile.clone();
        let missing = vec!["https://hub.test:missing".into()];

        assert_eq!(
            profile.prune_missing_bits(&snapshot, &missing, &HashSet::new()),
            missing
        );
        assert_eq!(profile.hub_profile.bits, vec!["missing"]);
        assert_eq!(profile.updated, profile.hub_profile.updated);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&profile.updated).unwrap()
                > chrono::DateTime::parse_from_rfc3339(&snapshot.updated).unwrap()
        );

        let after = profile.clone();
        assert!(
            profile
                .prune_missing_bits(&after.hub_profile, &missing, &HashSet::new())
                .is_empty()
        );
        assert_eq!(profile, after);
    }

    #[test]
    fn pruning_preserves_edits_made_while_validation_is_in_flight() {
        for change in 0..4 {
            let mut profile = profile_with_bits();
            let snapshot = profile.hub_profile.clone();
            match change {
                0 => profile.advance_revision(None),
                1 => profile.hub_profile.hub = "https://new-hub.test".into(),
                2 => profile
                    .hub_profile
                    .hubs
                    .push("https://fallback.test".into()),
                _ => profile.hub_profile.bits.push("new-bit".into()),
            }
            let after_edit = profile.clone();

            assert!(
                profile
                    .prune_missing_bits(&snapshot, &snapshot.bits, &HashSet::new())
                    .is_empty()
            );
            assert_eq!(profile, after_edit);
        }
    }

    #[test]
    fn pruning_preserves_custom_bits_added_while_validation_is_in_flight() {
        for inline in [false, true] {
            let mut profile = profile_with_bits();
            let snapshot = profile.hub_profile.clone();
            let mut custom_bit_ids = HashSet::new();
            if inline {
                let mut custom = flow_like::profile::ProfileCustomBit::default();
                custom.0.id = "missing".into();
                profile.hub_profile.custom_bits.push(custom);
            } else {
                custom_bit_ids.insert("missing".into());
            }
            let after_edit = profile.clone();

            assert!(
                profile
                    .prune_missing_bits(&snapshot, &snapshot.bits, &custom_bit_ids)
                    .is_empty()
            );
            assert_eq!(profile, after_edit);
        }
    }

    #[test]
    fn local_changes_advance_past_an_acknowledgement_ahead_of_the_device_clock() {
        let mut profile = UserProfile::new(FlowLikeProfile::default());
        profile.hub_profile.updated = "2099-09-05T10:00:00Z".into();
        profile.advance_revision(None);
        let first = chrono::DateTime::parse_from_rfc3339(&profile.updated).unwrap();
        profile.advance_revision(None);
        let second = chrono::DateTime::parse_from_rfc3339(&profile.updated).unwrap();
        assert!(first > chrono::DateTime::parse_from_rfc3339("2099-09-05T10:00:00Z").unwrap());
        assert!(second > first);
        assert_eq!(profile.updated, profile.hub_profile.updated);
    }

    #[test]
    fn in_flight_sync_preserves_a_new_home_and_retries_past_server_time() {
        let mut local = UserProfile::new(FlowLikeProfile::default());
        local.hub_profile.updated = "2026-09-05T10:00:00Z".into();
        let uploaded_revision = local.hub_profile.updated.clone();
        let mut incoming = local.clone();
        incoming.hub_profile.updated = "2099-09-05T10:00:02Z".into();
        incoming.updated = incoming.hub_profile.updated.clone();
        local.hub_profile.updated = "2026-09-05T10:00:01Z".into();
        local.hub_profile.home_layout = Some(serde_json::json!({
            "version": 1,
            "widgets": [{ "id": "new-widget" }]
        }));
        let home = local.hub_profile.home_layout.clone();
        let remote_revision = incoming.hub_profile.updated.clone();

        assert!(!local.merge_synced(incoming, &uploaded_revision, &remote_revision));
        assert_eq!(local.hub_profile.home_layout, home);
        assert_eq!(local.updated, local.hub_profile.updated);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&local.updated).unwrap()
                > chrono::DateTime::parse_from_rfc3339(&remote_revision).unwrap()
        );

        let retry_revision = local.hub_profile.updated.clone();
        let retry = local.clone();
        assert!(local.merge_synced(retry, &retry_revision, &remote_revision));
        assert_eq!(local.hub_profile.home_layout, home);
    }

    #[test]
    fn unchanged_snapshot_accepts_a_remote_home_reset() {
        let mut local = UserProfile::new(FlowLikeProfile::default());
        local.hub_profile.updated = "2026-09-05T10:00:00Z".into();
        local.hub_profile.home_layout = Some(serde_json::json!({ "version": 1, "widgets": [] }));
        local.hub_profile.home_default_id = Some("team-default".into());
        let mut incoming = local.clone();
        incoming.hub_profile.home_layout = None;
        incoming.hub_profile.updated = "2026-09-05T10:01:00Z".into();
        incoming.updated = incoming.hub_profile.updated.clone();
        let expected = local.hub_profile.updated.clone();
        let remote_revision = incoming.hub_profile.updated.clone();

        assert!(local.merge_synced(incoming, &expected, &remote_revision));
        assert!(local.hub_profile.home_layout.is_none());
        assert_eq!(
            local.hub_profile.home_default_id.as_deref(),
            Some("team-default")
        );
        assert_eq!(local.updated, remote_revision);
    }
}
