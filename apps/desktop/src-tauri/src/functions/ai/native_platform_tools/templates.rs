use super::{NativePlatformSession, boolean, err, limit, required, segment, string};
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn offset(args: &Value, key: &str) -> Result<usize, String> {
    match args.get(key) {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= 9_007_199_254_740_891)
            .map(|n| n as usize)
            .ok_or_else(|| "Template offsets must be nonnegative safe integers.".into()),
    }
}

fn metadata(meta: &Value) -> Value {
    let mut result = json!({});
    let mut truncated = false;
    for (key, limit) in [("name", 240), ("description", 2000), ("use_case", 1000)] {
        if let Some(text) = meta[key].as_str() {
            truncated |= text.encode_utf16().count() > limit;
            result[key] = json!(String::from_utf16_lossy(
                &text.encode_utf16().take(limit).collect::<Vec<_>>()
            ));
        }
    }
    let tags = meta["tags"].as_array().cloned().unwrap_or_default();
    truncated |= tags.len() > 12;
    result["tags"] = json!(
        tags.iter()
            .take(12)
            .filter_map(Value::as_str)
            .map(|tag| {
                truncated |= tag.encode_utf16().count() > 80;
                String::from_utf16_lossy(&tag.encode_utf16().take(80).collect::<Vec<_>>())
            })
            .collect::<Vec<_>>()
    );
    result["metadata_truncated"] = json!(truncated);
    result
}

fn merge(owned: &[Value], public: &[Value], limit: usize) -> (Vec<Value>, usize, usize) {
    let mut output: Vec<Value> = Vec::new();
    let mut consumed = [0, 0];
    loop {
        let mut advanced = false;
        for (index, candidates) in [owned, public].into_iter().enumerate() {
            let Some(candidate) = candidates.get(consumed[index]) else {
                continue;
            };
            let existing = output.iter().position(|entry| {
                entry["app_id"] == candidate["app_id"]
                    && entry["template_id"] == candidate["template_id"]
            });
            if existing.is_none() && output.len() >= limit {
                continue;
            }
            if let Some(existing) = existing {
                let mut sources = output[existing]["sources"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let source = json!(if index == 0 { "owned" } else { "public" });
                if !sources.contains(&source) {
                    sources.push(source);
                }
                output[existing]
                    .as_object_mut()
                    .unwrap()
                    .extend(candidate.as_object().unwrap().clone());
                output[existing]["sources"] = json!(sources);
            } else {
                output.push(candidate.clone());
            }
            consumed[index] += 1;
            advanced = true;
        }
        if !advanced {
            break;
        }
    }
    (output, consumed[0], consumed[1])
}

pub(super) async fn search(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let query = required(args, "query", "query")?;
    let public_offset = offset(args, "public_offset")?;
    let owned_offset = offset(args, "owned_offset")?;
    let limit = limit(args, "limit", "limit", 25, 100);
    let public_limit = (limit + 1).min(100);
    let mut path = format!(
        "apps/templates/search?query={}&limit={public_limit}&offset={public_offset}",
        segment(&query)
    );
    for key in ["category", "tag"] {
        if let Some(value) = string(args, key, key) {
            path.push_str(&format!("&{key}={}", segment(value)));
        }
    }
    let forkable = boolean(args, "forkable_only", "forkableOnly", false);
    if forkable {
        path.push_str("&forkable_only=true");
    }
    let public_read = session
        .request(Method::GET, &path, None)
        .await
        .and_then(|value| {
            value
                .as_array()
                .cloned()
                .ok_or_else(|| "Public template search returned an invalid list.".into())
        });
    let local = crate::functions::flow::template::get_templates(
        session.app_handle.clone(),
        None,
        None,
        Some(true),
        Some(1000),
    )
    .await
    .map_err(err)
    .and_then(|entries| serde_json::to_value(entries).map_err(err));
    let remote = if session.subject.is_some() {
        session
            .request(Method::GET, "user/templates?limit=100&offset=0", None)
            .await
    } else {
        Ok(json!([]))
    };
    let mut owned = BTreeMap::new();
    let mut owned_errors = Vec::new();
    for read in [local, remote] {
        match read {
            Ok(entries) => {
                if let Some(entries) = entries.as_array() {
                    for entry in entries.iter().take(1000) {
                        if entry[0].is_string() && entry[1].is_string() && entry[2].is_object() {
                            let key =
                                serde_json::to_string(&json!([entry[0], entry[1]])).map_err(err)?;
                            if owned.len() < 1000 || owned.contains_key(&key) {
                                owned.insert(key, entry.clone());
                            }
                        }
                    }
                } else {
                    owned_errors.push("Owned template metadata returned an invalid list.".into());
                }
            }
            Err(error) => owned_errors.push(error),
        }
    }
    let tag = string(args, "tag", "tag");
    let category = string(args, "category", "category");
    let query_lower = query.to_lowercase();
    let mut owned: Vec<_> = owned
        .into_values()
        .filter(|entry| {
            ["name", "description"].iter().any(|key| {
                entry[2][*key]
                    .as_str()
                    .is_some_and(|text| text.to_lowercase().contains(&query_lower))
            }) && tag.is_none_or(|tag| {
                entry[2]["tags"]
                    .as_array()
                    .is_some_and(|tags| tags.iter().any(|value| value == tag))
            })
        })
        .collect();
    let mut warnings=vec!["Remote metadata covers the first 100 memberships without exhaustion metadata. Local metadata covers at most 100 profile apps and 1000 template IDs and may omit unavailable records.".to_owned()];

    if category.is_some() || forkable {
        let mut apps = BTreeMap::new();
        for entry in &owned {
            if let Some(id) = entry[0].as_str() {
                apps.entry(id.to_owned()).or_insert(false);
            }
        }
        let ids: Vec<_> = apps.keys().take(25).cloned().collect();
        for id in ids {
            let record = if session.is_local(&id).await? {
                crate::functions::app::get_app(session.app_handle.clone(), id.clone())
                    .await
                    .map_err(err)
                    .and_then(|app| serde_json::to_value(app).map_err(err))
            } else {
                session
                    .request(Method::GET, &format!("apps/{}", segment(&id)), None)
                    .await
            };
            if let Ok(record) = record {
                apps.insert(
                    id,
                    category.is_none_or(|category| {
                        record["primary_category"] == category
                            || record["secondary_category"] == category
                    }) && (!forkable || record["allow_forking"] == true),
                );
            }
        }
        owned.retain(|entry| {
            entry[0]
                .as_str()
                .is_some_and(|id| apps.get(id).copied().unwrap_or(false))
        });
        warnings.push(
            "App-level filters checked at most 25 apps. Unverified matches were omitted.".into(),
        );
    }
    let owned_candidates: Vec<_> = owned
        .iter()
        .skip(owned_offset)
        .map(|entry| {
            let mut value = metadata(&entry[2]);
            value["app_id"] = entry[0].clone();
            value["template_id"] = entry[1].clone();
            value["sources"] = json!(["owned"]);
            value
        })
        .collect();
    let public_entries = public_read.as_ref().cloned().unwrap_or_default();
    let public_candidates: Vec<_> = public_entries
        .iter()
        .take(public_limit)
        .map(|entry| {
            let mut value = metadata(&entry["metadata"]);
            for key in ["app_id", "template_id", "app_allow_forking", "app_price"] {
                value[key] = entry[key].clone();
            }
            if let Some(name) = entry["app_name"].as_str() {
                value["app_name"] = json!(String::from_utf16_lossy(
                    &name.encode_utf16().take(240).collect::<Vec<_>>()
                ));
                value["app_name_truncated"] = json!(name.encode_utf16().count() > 240);
            }
            value["sources"] = json!(["public"]);
            value
        })
        .collect();
    let (templates, consumed_owned, consumed_public) =
        merge(&owned_candidates, &public_candidates, limit);
    session.assert_current_profile().await?;
    Ok(json!({
    "status":if public_read.is_err()&&owned_errors.len()==2&&templates.is_empty(){
    "error"}
    else{
    "ok"}
    ,"templates":templates,"complete":false,"note":"Metadata search only. Reuse both numeric next_offset values to continue. observed_exhausted describes the returned window or owned inventory, not exhaustive account or public search. Public offsets may overlap when the API consolidates metadata rows.","coverage":{
    "public":{
    "offset":public_offset,"next_offset":public_offset+consumed_public,"complete":false,"observed_exhausted":consumed_public==public_entries.len(),"scope":"public_api_window","warning":"The public API does not report a cursor or exhaustion metadata. Short pages can result from metadata consolidation or concurrent deletions.","error":public_read.err()}
    ,"owned":{
    "offset":owned_offset,"next_offset":owned_offset+consumed_owned,"complete":false,"observed_exhausted":consumed_owned==owned_candidates.len(),"scope":"local_cache_and_owned_remote_metadata","warning":warnings.join(" "),"error":if owned_errors.is_empty(){
    None}
    else{
    Some(owned_errors.join(" "))}
    }
    }
    }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_merge_advances_each_source_only_for_consumed_rows() {
        let owned = vec![
            json!({"app_id":"a","template_id":"x","sources":["owned"]}),
            json!({"app_id":"a","template_id":"y","sources":["owned"]}),
        ];
        let public = vec![
            json!({"app_id":"a","template_id":"x","sources":["public"]}),
            json!({"app_id":"b","template_id":"z","sources":["public"]}),
        ];
        let (results, owned_offset, public_offset) = merge(&owned, &public, 2);
        assert_eq!((owned_offset, public_offset), (2, 1));
        assert_eq!(results[0]["sources"], json!(["owned", "public"]));
    }
    #[test]
    fn invalid_template_offsets_fail_before_search() {
        assert!(offset(&json!({"public_offset":-1}), "public_offset").is_err());
        assert!(offset(&json!({"public_offset":1.2}), "public_offset").is_err());
    }
}
