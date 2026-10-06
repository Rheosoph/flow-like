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
        ensure!(total_bytes <= 64 * 1024 * 1024, "Selected Bit metadata exceeds 64 MiB");
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
    if !provider.get("provider_name").and_then(Value::as_str)
        .is_some_and(|name| name.trim().eq_ignore_ascii_case("device"))
    {
        return Ok(false);
    }
    let target = provider.pointer("/params/device_id").and_then(Value::as_str)
        .context("A device Bit must name its device")?.trim();
    ensure!(!target.is_empty(), "A device Bit must name its device");
    Ok(target == device_id)
}
