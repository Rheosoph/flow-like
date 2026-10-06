use super::{Authority, RejectAs, RejectionCode};
use crate::{config::PlacementConfig, project_artifacts};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::ManagementCapability;
use serde_json::Value;

/// Deploying a reference to an existing model delegates access to that model to the placement.
pub(super) fn require(
    authority: &Authority,
    config: &PlacementConfig,
    device_id: &str,
) -> Result<()> {
    if authority.permits(ManagementCapability::ModelUse, None, None) {
        return Ok(());
    }
    if uses_own_model(config, device_id).reject_as(RejectionCode::Invalid)? {
        authority.require(ManagementCapability::ModelUse, None, None)?;
    }
    Ok(())
}

fn uses_own_model(config: &PlacementConfig, device_id: &str) -> Result<bool> {
    let mut total_bytes = 0usize;
    for pin in &config.bit_pins {
        pin.validate()?;
        let bytes = project_artifacts::read_selected_metadata(&config.project_path, pin)?;
        total_bytes += bytes.len();
        ensure!(
            total_bytes <= 64 * 1024 * 1024,
            "Selected Bit metadata exceeds 64 MiB"
        );
        let metadata = project_artifacts::packaged_metadata(&bytes, pin)?;
        for bit in std::iter::once(metadata.bit()).chain(metadata.dependencies()) {
            if own_device_bit(bit, device_id)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn own_device_bit(bit: &Value, device_id: &str) -> Result<bool> {
    let Some(provider) = bit.pointer("/parameters/provider") else {
        return Ok(false);
    };
    if !provider
        .get("provider_name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.trim().eq_ignore_ascii_case("device"))
    {
        return Ok(false);
    }
    let target = provider
        .pointer("/params/device_id")
        .and_then(Value::as_str)
        .context("A device Bit must name its device")?
        .trim();
    ensure!(!target.is_empty(), "A device Bit must name its device");
    Ok(target == device_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        management,
        state::{DesiredState, StateStore},
    };
    use anyhow::Result;
    use flow_like_device_protocol::*;
    use serde_json::json;
    use std::path::PathBuf;

    struct Device {
        _temp: tempfile::TempDir,
        root: PathBuf,
        store: StateStore,
        manifest: OnboardingManifest,
        owner: Authority,
        deployer: Authority,
        model_user: Authority,
    }

    fn grantee(id: &str, model_use: bool) -> Authority {
        let key = SigningKey::generate().public_key();
        let mut capabilities = vec![ManagementCapability::Deploy, ManagementCapability::Start];
        if model_use {
            capabilities.push(ManagementCapability::ModelUse);
        }
        Authority {
            principal: format!("{id}:{id}"),
            key: key.clone(),
            grant: Some(ManagementGrant {
                grant_id: id.into(),
                user_id: id.into(),
                controller_key: key,
                scope: ManagementScope::Device,
                capabilities,
                expires_at: 1000,
                group_id: None,
                group_version: None,
            }),
        }
    }

    impl Device {
        fn new() -> Result<Self> {
            let temp = tempfile::tempdir()?;
            let root = crate::supervisor::prepare_state_dir(temp.path())?;
            let store = StateStore::open(&root.join("management.sqlite"))?;
            let signing = SigningKey::generate();
            let manifest = OnboardingManifest {
                version: 1,
                enrollment_id: "enrollment".into(),
                device_id: "device".into(),
                owner_id: "owner-user".into(),
                name: "Test device".into(),
                api_base_url: "https://example.test/api/v1".into(),
                bootstrap_key: SigningKey::generate().public_key(),
                controller_key: SigningKey::generate().public_key(),
                owner_invitation_key: signing.public_key(),
                issued_at: 100,
                expires_at: 1000,
            };
            let owner = Authority {
                principal: "owner-user:owner".into(),
                key: manifest.controller_key.clone(),
                grant: None,
            };
            let deployer = grantee("deployer", false);
            let model_user = grantee("model-user", true);
            let policy = ManagementPolicy {
                version: 1,
                device_id: "device".into(),
                policy_version: 1,
                previous_policy_digest: None,
                grants: vec![
                    deployer.grant.clone().unwrap(),
                    model_user.grant.clone().unwrap(),
                ],
                issued_at: 100,
                expires_at: 1000,
            };
            store.accept_management_policy(
                &sign_management_policy(&policy, &signing)?,
                &signing.public_key(),
                "device",
                100,
            )?;
            Ok(Self {
                _temp: temp,
                root,
                store,
                manifest,
                owner,
                deployer,
                model_user,
            })
        }

        fn config(&self, version: u8, target: &str, dependency: bool) -> Result<PlacementConfig> {
            let project = project_artifacts::managed_project_root(&self.root, "project")?;
            let device_bit = json!({"id": if dependency { "device-model" } else { "model" },
                "parameters": {"provider": {"provider_name": " DeViCe ",
                "params": {"device_id": target, "model": "private-model"}}}});
            let mut metadata = if dependency {
                json!({"bit": {"id": "model"}, "dependencies": [device_bit], "artifacts": []})
            } else {
                json!({"bit": device_bit, "dependencies": [], "artifacts": []})
            };
            if version == 2 {
                metadata["version"] = json!(2);
                metadata["assets"] = json!([]);
            }
            let bytes = serde_json::to_vec(&metadata)?;
            std::fs::create_dir_all(project.join("bits/metadata"))?;
            std::fs::write(project.join("bits/metadata/model.json"), &bytes)?;
            Ok(serde_json::from_value(json!({
                "id": "api", "project_id": "project", "deployment_id": "deployment",
                "revision": "release-1", "source": "offline", "project_path": project,
                "events": [{"event_id": "event", "event_version": [1,0,0], "board_version": [1,0,0]}],
                "bit_pins": [{"bit_id": "model", "metadata_sha256": artifact_sha256(&bytes)}]
            }))?)
        }

        fn run(
            &mut self,
            authority: &Authority,
            id: &str,
            command: ManagementCommand,
        ) -> Result<ManagementResponse> {
            management::execute(
                &mut self.store,
                authority,
                &ManagementRequest {
                    operation_id: id.into(),
                    device_id: "device".into(),
                    issued_at: 100,
                    expires_at: 300,
                    command,
                },
                &self.manifest,
                "boot",
                &self.root,
                101,
            )
        }
    }

    fn apply(config: &PlacementConfig) -> ManagementCommand {
        ManagementCommand::Apply {
            config: serde_json::to_value(config).unwrap(),
            expected_revision: 0,
            start: false,
        }
    }

    fn stage(config: &PlacementConfig) -> ManagementCommand {
        ManagementCommand::StageRollout {
            config: serde_json::to_value(config).unwrap(),
            expected_revision: 1,
            stabilization_seconds: 2,
            deadline_seconds: 30,
        }
    }

    #[test]
    fn own_device_models_require_use_permission_for_apply_stage_and_activate() -> Result<()> {
        for version in [1, 2] {
            for dependency in [false, true] {
                let mut device = Device::new()?;
                let config = device.config(version, " device ", dependency)?;
                let (deployer, model_user, owner) = (
                    device.deployer.clone(),
                    device.model_user.clone(),
                    device.owner.clone(),
                );
                let refused = device
                    .run(&deployer, "apply-denied", apply(&config))
                    .unwrap_err();
                assert_eq!(
                    management::rejection_code(&refused),
                    RejectionCode::Unauthorized
                );
                assert!(device.store.get_placement("api")?.is_none());
                device.run(&model_user, "apply-allowed", apply(&config))?;
                device
                    .store
                    .set_desired_state("api", DesiredState::Running)?;

                let refused = device
                    .run(&deployer, "stage-denied", stage(&config))
                    .unwrap_err();
                assert_eq!(
                    management::rejection_code(&refused),
                    RejectionCode::Unauthorized
                );
                assert!(!device.store.has_active_rollouts()?);
                device.run(&owner, "stage-allowed", stage(&config))?;
                let activate = || ManagementCommand::ActivateRollout {
                    rollout_id: "stage-allowed".into(),
                };
                let refused = device
                    .run(&deployer, "activate-denied", activate())
                    .unwrap_err();
                assert_eq!(
                    management::rejection_code(&refused),
                    RejectionCode::Unauthorized
                );
                assert_eq!(
                    device.store.rollout("stage-allowed")?.unwrap().state,
                    "staged"
                );
                device.run(&model_user, "activate-allowed", activate())?;
            }
        }
        Ok(())
    }

    #[test]
    fn remote_device_references_keep_deployment_authorization() -> Result<()> {
        let mut device = Device::new()?;
        let config = device.config(2, "another-device", false)?;
        let deployer = device.deployer.clone();
        device.run(&deployer, "apply-remote", apply(&config))?;
        device
            .store
            .set_desired_state("api", DesiredState::Running)?;
        device.run(&deployer, "stage-remote", stage(&config))?;
        device.run(
            &deployer,
            "activate-remote",
            ManagementCommand::ActivateRollout {
                rollout_id: "stage-remote".into(),
            },
        )?;
        Ok(())
    }

    #[test]
    fn metadata_is_verified_before_a_deployer_can_avoid_the_model_check() -> Result<()> {
        let device = Device::new()?;
        let mut config = device.config(2, "another-device", false)?;
        let path = config.project_path.join("bits/metadata/model.json");
        let tampered =
            br#"{"version":2,"bit":{"id":"model"},"dependencies":[],"assets":[],"artifacts":[]}"#;
        std::fs::write(&path, tampered)?;
        let error = require(&device.deployer, &config, "device").unwrap_err();
        assert_eq!(management::rejection_code(&error), RejectionCode::Invalid);
        std::fs::write(&path, b"invalid metadata")?;
        config.bit_pins[0].metadata_sha256 = artifact_sha256(b"invalid metadata");
        let error = require(&device.deployer, &config, "device").unwrap_err();
        assert_eq!(management::rejection_code(&error), RejectionCode::Invalid);
        Ok(())
    }
}
