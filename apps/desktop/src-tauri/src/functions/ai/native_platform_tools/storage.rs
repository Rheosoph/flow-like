use flow_like::flow_like_storage::object_store::ObjectStoreExt;
use std::time::Duration;

use flow_like::{
    flow_like_storage::{Path, files::store::FlowLikeStore},
    utils::storage::{construct_storage, construct_user_storage},
};
use reqwest::Method;
use serde_json::{Value, json};

use super::{NativePlatformSession, boolean, err, limit, required, segment, string};
use crate::{functions::flow::storage, state::TauriFlowLikeState};

const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;

async fn local_path(
    session: &NativePlatformSession,
    app_id: &str,
    prefix: &str,
    user: bool,
) -> Result<(FlowLikeStore, Path), String> {
    if app_id.is_empty()
        || !app_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return Err("Invalid storage app ID.".into());
    }
    let state = TauriFlowLikeState::construct(&session.app_handle)
        .await
        .map_err(err)?;
    let sub = session.subject.as_deref().unwrap_or("local");
    let (store, base) = if user {
        construct_user_storage(&state, sub, app_id, "").await
    } else {
        construct_storage(&state, app_id, "").await
    }
    .map_err(err)?;
    let (_, path) = if user {
        construct_user_storage(&state, sub, app_id, prefix).await
    } else {
        construct_storage(&state, app_id, prefix).await
    }
    .map_err(err)?;
    if !path.prefix_matches(&base) {
        return Err("Storage path is outside the current app and user.".into());
    }
    Ok((store, path))
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    args: &Value,
) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    let local = session.is_local(&app_id).await?;
    let user = boolean(args, "user_scoped", "userScoped", false);
    let operation = string(args, "operation", "operation").unwrap_or("list_files");
    let base = format!(
        "apps/{}/data{}",
        segment(&app_id),
        if user { "/user" } else { "" }
    );
    match operation {
        "list_files" => {
            let prefix = string(args, "prefix", "prefix").unwrap_or("");
            let items = if local {
                local_path(session, &app_id, prefix, user).await?;
                serde_json::to_value(
                    if user {
                        storage::storage_user_list(
                            session.app_handle.clone(),
                            app_id,
                            prefix.into(),
                        )
                        .await
                    } else {
                        storage::storage_list(session.app_handle.clone(), app_id, prefix.into())
                            .await
                    }
                    .map_err(err)?,
                )
                .map_err(err)?
            } else {
                session
                    .request(
                        Method::POST,
                        &format!("{base}/list"),
                        Some(&json!({"prefix":prefix})),
                    )
                    .await?
            };
            if !items.is_array() {
                return Err("Storage returned an invalid file list.".into());
            }
            Ok(json!({"status":"ok","prefix":prefix,"user_scoped":user,"items":items}))
        }
        "read_file" => {
            let path = required(args, "path", "path")?;
            let max_chars = limit(args, "max_chars", "maxChars", 20_000, 120_000);
            let (bytes, url) = if local {
                let (store, object_path) = local_path(session, &app_id, &path, user).await?;
                let object = store.as_generic().get(&object_path).await.map_err(err)?;
                let bytes = flow_like::flow_like_storage::files::bounded::read_object(
                    object,
                    MAX_FILE_BYTES as u64,
                )
                .await
                .map_err(err)?;
                let url = store
                    .sign("GET", &object_path, Duration::from_secs(3600))
                    .await
                    .map_err(err)?
                    .to_string();
                (bytes.to_vec(), url)
            } else {
                let downloads = session
                    .request(
                        Method::POST,
                        &format!("{base}/download"),
                        Some(&json!({"prefixes":[path]})),
                    )
                    .await?;
                let file = first_file(&downloads, &path)?;
                let Some(url) = file["url"].as_str() else {
                    return Ok(
                        json!({"status":"ok","path":path,"message":"Storage provider returned no readable URL."}),
                    );
                };
                let mut response = session
                    .client()
                    .get(signed_url(url)?)
                    .send()
                    .await
                    .map_err(|_| "Storage download failed.".to_string())?;
                if !response.status().is_success() {
                    return Err(format!(
                        "Storage download returned HTTP {}.",
                        response.status().as_u16()
                    ));
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| "Storage download was interrupted.".to_string())?
                {
                    if bytes.len().saturating_add(chunk.len()) > MAX_FILE_BYTES {
                        return Err("The file exceeds the native tool read limit of 16 MiB.".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                (bytes, url.into())
            };
            session.assert_current_profile().await?;
            let content = String::from_utf8_lossy(&bytes);
            let chars = content.encode_utf16().count();
            let preview = truncate_utf16(&content, max_chars);
            Ok(
                json!({"status":"ok","path":path,"url":url,"truncated":chars>max_chars,"content":preview,"chars":chars}),
            )
        }
        "create_file" => {
            let path = required(args, "path", "path")?;
            if path.ends_with('/') || path.rsplit('/').next().unwrap_or("").is_empty() {
                return Err("create_file path must include a file name.".into());
            }
            let content = match args.get("content") {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(text)) => text.clone(),
                Some(value) => value.to_string(),
            };
            if content.len() > MAX_FILE_BYTES {
                return Err("The file exceeds the native tool write limit of 16 MiB.".into());
            }
            let mime = string(args, "mime_type", "mimeType").unwrap_or("text/plain");
            if local {
                let (store, object_path) = local_path(session, &app_id, &path, user).await?;
                session.assert_current_profile().await?;
                store
                    .put(&object_path, content.clone().into_bytes())
                    .await
                    .map_err(err)?;
            } else {
                let uploads = session
                    .request(
                        Method::PUT,
                        &base,
                        Some(&json!({"prefixes":[path],"sizes":[content.len()]})),
                    )
                    .await?;
                let file = first_file(&uploads, &path)?;
                let url = signed_url(
                    file["url"]
                        .as_str()
                        .ok_or("Storage returned no upload URL.")?,
                )?;
                session.assert_current_profile().await?;
                let request = if let Some(fields) = file["fields"].as_object() {
                    let mut form = reqwest::multipart::Form::new();
                    for (key, value) in fields {
                        form = form.text(
                            key.clone(),
                            value
                                .as_str()
                                .ok_or("Storage returned an invalid upload policy.")?
                                .to_owned(),
                        );
                    }
                    form = form.part(
                        "file",
                        reqwest::multipart::Part::bytes(content.clone().into_bytes())
                            .file_name(path.rsplit('/').next().unwrap_or("file").to_owned())
                            .mime_str(mime)
                            .map_err(err)?,
                    );
                    session.client().post(url).multipart(form)
                } else {
                    let azure = url.host_str().is_some_and(|host| {
                        host.ends_with(".blob.core.windows.net")
                            || host.ends_with(".blob.core.usgovcloudapi.net")
                            || host.ends_with(".blob.core.chinacloudapi.cn")
                    });
                    let mut request = session.client().put(url).header("Content-Type", mime);
                    if azure {
                        request = request.header("x-ms-blob-type", "BlockBlob");
                    }
                    request.body(content.clone())
                };
                let response = request
                    .send()
                    .await
                    .map_err(|_| "Storage upload failed.".to_string())?;
                if !response.status().is_success() {
                    return Err(format!(
                        "Storage upload returned HTTP {}.",
                        response.status().as_u16()
                    ));
                }
            }
            Ok(json!({"status":"ok","path":path,"bytes":content.len(),"user_scoped":user}))
        }
        "delete_files" => {
            let paths: Vec<String> = if let Some(paths) = args["paths"].as_array() {
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            } else {
                string(args, "path", "path")
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            };
            if paths.is_empty() {
                return Err("delete_files requires paths.".into());
            }
            if local {
                for path in &paths {
                    local_path(session, &app_id, path, user).await?;
                }
                session.assert_current_profile().await?;
                if user {
                    storage::storage_user_remove(session.app_handle.clone(), app_id, paths.clone())
                        .await
                } else {
                    storage::storage_remove(session.app_handle.clone(), app_id, paths.clone()).await
                }
                .map_err(err)?;
            } else {
                session
                    .request(Method::DELETE, &base, Some(&json!({"prefixes":paths})))
                    .await?;
            }
            Ok(json!({"status":"ok","deleted":paths,"user_scoped":user}))
        }
        _ => Err(format!("Unsupported storage_tool operation '{operation}'.")),
    }
}

fn first_file<'a>(value: &'a Value, path: &str) -> Result<&'a Value, String> {
    let file = value
        .as_array()
        .and_then(|files| {
            files
                .iter()
                .find(|file| file["prefix"] == path)
                .or_else(|| files.first())
        })
        .ok_or("Storage returned no file result.")?;
    if file["error"].is_string() {
        return Err("The storage provider could not resolve this file.".into());
    }
    Ok(file)
}

fn signed_url(raw: &str) -> Result<reqwest::Url, String> {
    let url =
        reqwest::Url::parse(raw).map_err(|_| "Storage returned an invalid URL.".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Storage returned an unsupported URL.".into());
    }
    Ok(url)
}

fn truncate_utf16(value: &str, max: usize) -> String {
    String::from_utf16_lossy(&value.encode_utf16().take(max).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_preview_counts_browser_string_units() {
        assert_eq!(truncate_utf16("A😀B", 3), "A😀");
        assert_eq!(truncate_utf16("abc", 2), "ab");
    }
    #[test]
    fn signed_urls_cannot_be_local_files_or_credential_urls() {
        for url in [
            "file:///etc/passwd",
            "https://user:password@store.test/path",
            "asset://localhost/path",
        ] {
            assert!(signed_url(url).is_err());
        }
        assert!(signed_url("https://store.test/path?signature=abc").is_ok());
    }
}
