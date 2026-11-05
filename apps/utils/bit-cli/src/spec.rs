use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, anyhow, bail};
use flow_like::{
    bit::{Bit, BitTypes, ForecastingAssetRef, ForecastingModelParameters, Metadata},
    flow_like_model_provider::provider::is_hosted_provider_name,
    flow_like_types::create_id,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One file describes a bit and everything it depends on. Entries carry the
/// fields of a `Bit` plus two authoring conveniences: `ref`, a local alias
/// other entries point at as `@alias`, and `meta`, the per-language metadata
/// the hub stores through its own endpoint.
#[derive(Deserialize, Serialize)]
pub struct Spec {
    pub bits: Vec<Value>,
}

pub struct Planned {
    pub id: String,
    pub alias: Option<String>,
    pub bit_type: String,
    pub payload: Value,
    pub dependencies: Vec<Dependency>,
    forecasting_assets: Vec<Dependency>,
    pub meta: BTreeMap<String, Metadata>,
}

/// A dependency is stored as `hub:id`, and the hub half is only known once the
/// dependency itself has been written, so the reference is resolved at push
/// time rather than while planning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dependency {
    Local(String),
    Remote(String),
}

impl Planned {
    /// Use the identities returned by the Hub, which may differ from the IDs
    /// submitted in the spec when a file already exists or an entry is new.
    pub fn push_payload(&self, stored: &BTreeMap<String, Bit>, hub: &str) -> Result<Value> {
        let mut payload = self.payload.clone();
        let dependencies = self
            .dependencies
            .iter()
            .map(|dependency| reference(dependency, stored, hub).map(Value::String))
            .collect::<Result<Vec<_>>>()?;
        payload["dependencies"] = Value::Array(dependencies);
        if !self.forecasting_assets.is_empty() {
            let assets = payload
                .pointer_mut("/parameters/assets")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| anyhow!("entry {} has no forecasting assets", self.id))?;
            if assets.len() != self.forecasting_assets.len() {
                bail!("entry {} has inconsistent forecasting assets", self.id);
            }
            for (asset, dependency) in assets.iter_mut().zip(&self.forecasting_assets) {
                asset["bit"] = Value::String(reference(dependency, stored, hub)?);
            }
        }
        Ok(payload)
    }
}

/// The hub may reuse a hosted Bit by its authoring hash without applying the new
/// parameters. Address that stored ID directly while keeping its catalog identity.
pub fn hosted_update_payload(payload: &Value, stored: &Bit) -> Option<Value> {
    let authoring_id = payload.get("id")?.as_str()?;
    if authoring_id == stored.id
        || stored.hash != authoring_id
        || !payload
            .get("hash")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || !payload
            .pointer("/parameters/provider/provider_name")
            .and_then(Value::as_str)
            .is_some_and(is_hosted_provider_name)
        || payload
            .get("download_link")
            .is_some_and(|link| !link.is_null())
        || payload.get("file_name").is_some_and(|name| !name.is_null())
        || !payload
            .get("dependencies")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        || stored.download_link.is_some()
        || stored.file_name.is_some()
        || !stored.dependencies.is_empty()
    {
        return None;
    }

    let mut update = payload.clone();
    update["id"] = Value::String(stored.id.clone());
    update["hub"] = Value::String(stored.hub.clone());
    update["hash"] = Value::String(stored.hash.clone());
    update["dependency_tree_hash"] = Value::String(stored.dependency_tree_hash.clone());
    Some(update)
}

fn reference(dependency: &Dependency, stored: &BTreeMap<String, Bit>, hub: &str) -> Result<String> {
    match dependency {
        Dependency::Local(id) => {
            let bit = stored
                .get(id)
                .ok_or_else(|| anyhow!("dependency {id} was not written before its dependent"))?;
            let bit_hub = if bit.hub.is_empty() { hub } else { &bit.hub };
            Ok(format!("{bit_hub}:{}", bit.id))
        }
        Dependency::Remote(reference) if reference.contains(':') => Ok(reference.clone()),
        Dependency::Remote(id) => Ok(format!("{hub}:{id}")),
    }
}

impl Dependency {
    pub fn describe(&self) -> String {
        match self {
            Dependency::Local(id) => id.clone(),
            Dependency::Remote(reference) => reference.clone(),
        }
    }
}

pub fn load(path: &std::path::Path) -> Result<Spec> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| anyhow!("could not read the spec at {}: {err}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|err| anyhow!("could not parse the spec at {}: {err}", path.display()))
}

/// Resolve aliases, fill in ids and order the entries so a dependency is
/// always stored before the bit that names it.
pub fn plan(spec: &Spec, hub: &str) -> Result<Vec<Planned>> {
    let entries = spec
        .bits
        .iter()
        .map(|entry| {
            entry
                .as_object()
                .cloned()
                .ok_or_else(|| anyhow!("every entry under \"bits\" must be an object"))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut ids = Vec::new();
    for entry in &entries {
        let id = match entry.get("id").and_then(Value::as_str) {
            Some(id) if !id.trim().is_empty() => id.trim().to_string(),
            _ => create_id(),
        };
        if let Some(alias) = entry.get("ref").and_then(Value::as_str)
            && aliases.insert(alias.to_string(), id.clone()).is_some()
        {
            bail!("the alias @{alias} is used by more than one entry");
        }
        ids.push(id);
    }

    let allowed = bit_fields();
    let mut planned = Vec::new();
    for (entry, id) in entries.iter().zip(&ids) {
        let alias = entry.get("ref").and_then(Value::as_str).map(str::to_string);

        let mut payload = entry.clone();
        let meta = take_meta(&mut payload)?;
        payload.remove("ref");
        payload.insert("id".to_string(), Value::String(id.clone()));

        for key in payload.keys() {
            if !allowed.contains(key.as_str()) {
                bail!("entry {id} carries the unknown field \"{key}\"");
            }
        }

        let mut dependencies = resolve_dependencies(&payload, &aliases, id)?;
        payload.remove("dependencies");

        if payload
            .get("hub")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            payload.insert("hub".to_string(), Value::String(hub.to_string()));
        }

        let mut payload = merge(serde_json::to_value(Bit::default())?, &payload);
        let bit: Bit = serde_json::from_value(payload.clone())
            .map_err(|err| anyhow!("entry {id} is not a valid bit: {err}"))?;
        let forecasting_assets =
            resolve_forecasting_assets(&mut payload, &bit, &aliases, &mut dependencies)?;

        planned.push(Planned {
            id: id.clone(),
            alias,
            bit_type: serde_json::to_value(&bit.bit_type)?
                .as_str()
                .unwrap_or("Other")
                .to_string(),
            payload,
            dependencies,
            forecasting_assets,
            meta,
        });
    }

    order(planned)
}

/// Build a spec out of bits the hub already stores, so an existing entry is
/// the starting point for the next one.
pub fn from_bits(bits: &[Bit]) -> Spec {
    let known: HashSet<&str> = bits.iter().map(|bit| bit.id.as_str()).collect();
    let aliases: HashMap<&str, String> = bits
        .iter()
        .enumerate()
        .map(|(index, bit)| (bit.id.as_str(), alias_for(bit, index)))
        .collect();

    let entries = bits
        .iter()
        .map(|bit| {
            let mut entry = Map::new();
            entry.insert(
                "ref".to_string(),
                Value::String(aliases[bit.id.as_str()].clone()),
            );

            let mut body = serde_json::to_value(bit)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            body.remove("meta");
            for computed in [
                "hash",
                "dependency_tree_hash",
                "created",
                "updated",
                "model_evaluation",
            ] {
                body.remove(computed);
            }
            if let Some(Value::Array(dependencies)) = body.get_mut("dependencies") {
                for dependency in dependencies.iter_mut() {
                    let Some(reference) = dependency.as_str() else {
                        continue;
                    };
                    let id = reference.split_once(':').map_or(reference, |(_, id)| id);
                    if known.contains(id) {
                        *dependency = Value::String(format!("@{}", aliases[id]));
                    }
                }
            }
            if bit.bit_type == BitTypes::Forecasting
                && let Some(Value::Array(assets)) = body
                    .get_mut("parameters")
                    .and_then(|parameters| parameters.get_mut("assets"))
            {
                for asset in assets {
                    let Some(reference) = asset.get("bit").and_then(Value::as_str) else {
                        continue;
                    };
                    let reference = ForecastingAssetRef {
                        bit: reference.to_string(),
                        relative_path: String::new(),
                    };
                    if let Some((index, dependency)) =
                        bits.iter().enumerate().find(|(_, candidate)| {
                            reference
                                .matches_reference(candidate, &bit.hub)
                                .unwrap_or(false)
                        })
                    {
                        asset["bit"] = Value::String(format!("@{}", alias_for(dependency, index)));
                    }
                }
            }
            entry.extend(body);

            let meta: BTreeMap<_, _> = bit.meta.iter().collect();
            if !meta.is_empty()
                && let Ok(meta) = serde_json::to_value(meta)
            {
                entry.insert("meta".to_string(), meta);
            }

            Value::Object(entry)
        })
        .collect();

    Spec { bits: entries }
}

fn alias_for(bit: &Bit, index: usize) -> String {
    let base = serde_json::to_value(&bit.bit_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_lowercase))
        .unwrap_or_else(|| "bit".to_string());
    format!("{base}-{index}")
}

fn take_meta(entry: &mut Map<String, Value>) -> Result<BTreeMap<String, Metadata>> {
    let Some(meta) = entry.remove("meta") else {
        return Ok(BTreeMap::new());
    };
    let Value::Object(languages) = meta else {
        bail!("\"meta\" must map a language code to its metadata");
    };

    let base = serde_json::to_value(Metadata::default())?;
    languages
        .into_iter()
        .map(|(language, value)| {
            let Value::Object(fields) = value else {
                bail!("the metadata for {language} must be an object");
            };
            let merged = merge(base.clone(), &fields);
            let metadata: Metadata = serde_json::from_value(merged)
                .map_err(|err| anyhow!("the metadata for {language} is invalid: {err}"))?;
            Ok((language, metadata))
        })
        .collect()
}

fn resolve_dependencies(
    entry: &Map<String, Value>,
    aliases: &HashMap<String, String>,
    id: &str,
) -> Result<Vec<Dependency>> {
    let Some(dependencies) = entry.get("dependencies") else {
        return Ok(Vec::new());
    };
    let Value::Array(dependencies) = dependencies else {
        bail!("the dependencies of {id} must be a list");
    };

    dependencies
        .iter()
        .map(|dependency| {
            let Some(dependency) = dependency.as_str() else {
                bail!("every dependency of {id} must be a bit id or an @alias");
            };
            resolve_reference(dependency, aliases, id)
        })
        .collect()
}

fn resolve_reference(
    reference: &str,
    aliases: &HashMap<String, String>,
    id: &str,
) -> Result<Dependency> {
    match reference.strip_prefix('@') {
        Some(alias) => aliases
            .get(alias)
            .map(|id| Dependency::Local(id.clone()))
            .ok_or_else(|| anyhow!("{id} depends on @{alias}, which the spec never defines")),
        None => Ok(Dependency::Remote(reference.to_string())),
    }
}

fn resolve_forecasting_assets(
    payload: &mut Value,
    bit: &Bit,
    aliases: &HashMap<String, String>,
    dependencies: &mut Vec<Dependency>,
) -> Result<Vec<Dependency>> {
    if bit.bit_type != BitTypes::Forecasting {
        return Ok(Vec::new());
    }
    let mut parameters: ForecastingModelParameters = serde_json::from_value(bit.parameters.clone())
        .map_err(|error| {
            anyhow!(
                "entry {} has invalid forecasting parameters: {error}",
                bit.id
            )
        })?;
    let assets = payload
        .pointer_mut("/parameters/assets")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("entry {} must list forecasting assets", bit.id))?;
    let mut resolved = Vec::new();
    let mut kept = Vec::new();
    let mut validated_assets = Vec::new();
    for (mut asset, authored) in parameters.assets.into_iter().zip(std::mem::take(assets)) {
        let dependency = resolve_reference(&asset.bit, aliases, &bit.id)?;
        let is_root = match &dependency {
            Dependency::Local(id) => id == &bit.id,
            Dependency::Remote(_) => asset.matches_reference(bit, &bit.hub)?,
        };
        if is_root {
            if bit.file_name.as_deref() != Some(asset.relative_path.as_str()) {
                bail!(
                    "entry {} maps itself to {}; a self asset must use the root file_name",
                    bit.id,
                    asset.relative_path
                );
            }
            // The runtime already stages the root file. Omitting this redundant
            // mapping also avoids predicting the ID assigned during publication.
            continue;
        }
        asset.bit = match &dependency {
            Dependency::Local(id) => format!("{}:{id}", bit.hub),
            Dependency::Remote(reference) => reference.clone(),
        };
        validated_assets.push(asset);
        kept.push(authored);
        if !dependencies.contains(&dependency) {
            dependencies.push(dependency.clone());
        }
        resolved.push(dependency);
    }
    parameters.assets = validated_assets;
    parameters.validate().map_err(|error| {
        anyhow!(
            "entry {} has invalid forecasting parameters: {error}",
            bit.id
        )
    })?;
    *assets = kept;
    Ok(resolved)
}

fn order(planned: Vec<Planned>) -> Result<Vec<Planned>> {
    let mut pending = planned;
    let mut stored: HashSet<String> = HashSet::new();
    let mut ordered = Vec::new();

    while !pending.is_empty() {
        let (ready, waiting): (Vec<_>, Vec<_>) = pending.into_iter().partition(|entry| {
            entry
                .dependencies
                .iter()
                .all(|dependency| match dependency {
                    Dependency::Local(id) => stored.contains(id),
                    Dependency::Remote(_) => true,
                })
        });

        if ready.is_empty() {
            bail!(
                "the spec has a dependency cycle between {}",
                waiting
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        stored.extend(ready.iter().map(|entry| entry.id.clone()));
        ordered.extend(ready);
        pending = waiting;
    }

    Ok(ordered)
}

fn merge(base: Value, overlay: &Map<String, Value>) -> Value {
    let mut merged = base.as_object().cloned().unwrap_or_default();
    for (key, value) in overlay {
        merged.insert(key.clone(), value.clone());
    }
    Value::Object(merged)
}

fn bit_fields() -> HashSet<&'static str> {
    static FIELDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FIELDS
        .get_or_init(|| {
            serde_json::to_value(Bit::default())
                .ok()
                .and_then(|value| {
                    value
                        .as_object()
                        .map(|fields| fields.keys().cloned().collect())
                })
                .unwrap_or_default()
        })
        .iter()
        .map(String::as_str)
        .collect()
}

#[cfg(test)]
mod tests;
