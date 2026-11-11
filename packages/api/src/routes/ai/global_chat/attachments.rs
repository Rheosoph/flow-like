//! Image attachments the browser sends as links (`GlobalChatRequest::attachment_urls`).
//!
//! The browser uploads each file to `/tmp` and sends the signed download link, so the expected
//! link is the caller's own upload on the platform's object store. Any other link must be public
//! HTTPS. A link that passes reaches the model as a link: the model's media handling hands it to
//! a provider that fetches links, signed again for minutes when it points into the platform's
//! storage, and inlines it, fitted to the provider's limits, for any other provider. Links are
//! never logged: a signed link is a credential.

use std::collections::BTreeMap;

use flow_like::copilot::ChatImage;
use flow_like::flow::copilot::{AttachmentManifestEntry, TURN_FILES};
use flow_like::flow_like_model_provider::egress::ProviderEgress;
use flow_like_storage::Path as ObjectPath;
use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::reqwest::Url;

use crate::{
    bot_tools::AttachedFile,
    credentials::storage_path_segment,
    media_links::{StorageScope, route},
    state::AppState,
};

const MAX_ATTACHMENT_URLS: usize = 8;

/// Gives a bot's file tools a handle for each attachment the server can read while the bot
/// answers: one whose name no other attachment shares and whose link is the caller's own upload.
/// A handle an entry already has stays. Returns the attachments and the uploads by attachment
/// name, as the new handles name them.
pub(super) async fn with_turn_files(
    state: &AppState,
    sub: &str,
    mut manifest: Vec<AttachmentManifestEntry>,
) -> (Vec<AttachmentManifestEntry>, BTreeMap<String, AttachedFile>) {
    let own = match manifest.iter().any(lacks_handle) {
        true => own_uploads(state, sub).await,
        false => None,
    };
    let files = match own {
        Some(own) => attach_own_uploads(&own, sub, &mut manifest),
        None => BTreeMap::new(),
    };
    (manifest, files)
}

fn lacks_handle(entry: &AttachmentManifestEntry) -> bool {
    entry.handle.is_none() && entry.url.is_some()
}

fn attach_own_uploads(
    own: &StorageScope,
    sub: &str,
    manifest: &mut [AttachmentManifestEntry],
) -> BTreeMap<String, AttachedFile> {
    let names: Vec<Option<String>> = manifest.iter().map(|entry| entry.name.clone()).collect();
    let shared = |name: &str| {
        names
            .iter()
            .filter(|other| other.as_deref() == Some(name))
            .count()
            > 1
    };
    let mut files = BTreeMap::new();
    for entry in manifest.iter_mut().filter(|entry| lacks_handle(entry)) {
        let Some(name) = entry
            .name
            .clone()
            .filter(|name| !name.is_empty() && !shared(name))
        else {
            continue;
        };
        let Some(key) = entry
            .url
            .as_deref()
            .and_then(|url| upload_key(own, sub, url))
        else {
            continue;
        };
        let mime = entry.mime_type.clone().filter(|mime| !mime.is_empty());
        entry.handle = Some(WorkspaceHandle::conversation_file(
            TURN_FILES,
            &name,
            entry.size.unwrap_or_default(),
            "",
            mime.as_deref().unwrap_or_default(),
        ));
        files.insert(name, AttachedFile { key, mime });
    }
    files
}

/// The object a link into the caller's uploads names.
fn upload_key(own: &StorageScope, sub: &str, url: &str) -> Option<ObjectPath> {
    let url = Url::parse(url).ok()?;
    let object = ObjectPath::from_url_path(own.key(&url)?).ok()?;
    let prefix = ObjectPath::from(upload_prefix(sub));
    Some(prefix.parts().chain(object.parts()).collect())
}

/// The attachment links the model may receive, in request order.
pub(super) async fn resolve_attachment_images(
    state: &AppState,
    sub: &str,
    urls: &[String],
) -> Vec<ChatImage> {
    let urls = http_urls(urls);
    if urls.is_empty() {
        return Vec::new();
    }
    let own = own_uploads(state, sub).await;
    checked_links(own.as_ref(), urls)
}

/// The http(s) links among the first [`MAX_ATTACHMENT_URLS`] entries, with their position. Other
/// schemes (`data:`, `blob:`) never reach the model.
fn http_urls(urls: &[String]) -> Vec<(usize, Url)> {
    urls.iter()
        .take(MAX_ATTACHMENT_URLS)
        .enumerate()
        .filter_map(|(index, raw)| match Url::parse(raw) {
            Ok(url) => matches!(url.scheme(), "http" | "https").then_some((index, url)),
            Err(_) => {
                tracing::warn!(index, "[global_chat] attachment link is not a URL, skipped");
                None
            }
        })
        .collect()
}

/// Where `/tmp` puts the caller's uploads: `tmp/user/{sub segment}/`.
fn upload_prefix(sub: &str) -> String {
    format!("tmp/user/{}/", storage_path_segment(sub, "user"))
}

/// Where `/tmp` points the caller's download links, signed the way `routes::tmp` signs an
/// upload. `None` when the store cannot sign; then only public HTTPS links pass.
async fn own_uploads(state: &AppState, sub: &str) -> Option<StorageScope> {
    let probed = async {
        let store = state.master_credentials().await?.to_store(false).await?;
        StorageScope::signed_by(&store, &upload_prefix(sub)).await
    }
    .await;
    match probed {
        Ok(Some(own)) => Some(own),
        Ok(None) => {
            tracing::warn!(
                "[global_chat] signed upload links do not end in the object key; only public HTTPS attachments pass"
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "[global_chat] could not sign an upload probe; only public HTTPS attachments pass"
            );
            None
        }
    }
}

/// The caller's own uploads and public HTTPS links, as images the model receives by link.
fn checked_links(own: Option<&StorageScope>, urls: Vec<(usize, Url)>) -> Vec<ChatImage> {
    urls.into_iter()
        .filter_map(|(index, url)| match route(&url, own, ProviderEgress::Public) {
            Ok(_) => Some(ChatImage::link(
                url.to_string(),
                attachment_media_type(&url).to_owned(),
            )),
            Err(_) => {
                tracing::warn!(
                    index,
                    host = url.host_str().unwrap_or_default(),
                    "[global_chat] attachment link is neither the caller's upload nor public HTTPS, skipped"
                );
                None
            }
        })
        .collect()
}

/// The image type the link's file extension names, PNG for any other.
fn attachment_media_type(url: &Url) -> &'static str {
    let extension = url
        .path()
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_links::tests::s3_store;
    use flow_like_storage::Path;
    use std::time::Duration;

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    #[flow_like_types::tokio::test]
    async fn the_callers_uploads_are_their_scope_and_nobody_elses() {
        let sub = "auth0|me";
        let store = s3_store("http://rustfs:9000");
        let own = StorageScope::signed_by(&store, &upload_prefix(sub))
            .await
            .unwrap()
            .unwrap();
        let sign = |owner: &str| {
            let key = Path::from(format!("{}2026/10/28/k3x9.png", upload_prefix(owner)));
            let store = &store;
            async move {
                store
                    .sign("GET", &key, Duration::from_secs(600))
                    .await
                    .unwrap()
            }
        };
        assert!(own.contains(&sign(sub).await));
        assert!(!own.contains(&sign("someone-else").await));
    }

    #[flow_like_types::tokio::test]
    async fn only_the_callers_uniquely_named_uploads_get_a_turn_handle() {
        let sub = "auth0|me";
        let store = s3_store("http://rustfs:9000");
        let own = StorageScope::signed_by(&store, &upload_prefix(sub))
            .await
            .unwrap()
            .unwrap();
        let key = |owner: &str, file: &str| {
            Path::from(format!("{}2026/11/11/{file}", upload_prefix(owner)))
        };
        let link = |owner: &'static str, file: &'static str| {
            let store = &store;
            async move {
                let path = key(owner, file);
                let signed = store.sign("GET", &path, Duration::from_secs(600)).await;
                Some(signed.unwrap().to_string())
            }
        };
        let entry = |name: &str, url: Option<String>| AttachmentManifestEntry {
            name: Some(name.to_string()),
            mime_type: Some("application/pdf".to_string()),
            size: Some(7),
            url,
            handle: None,
        };
        let stored = WorkspaceHandle::conversation_file("room1", "f9.pdf", 7, "", "");
        let mut manifest = vec![
            entry("scan.pdf", link(sub, "a1.pdf").await),
            entry("theirs.pdf", link("someone-else", "b2.pdf").await),
            entry(
                "public.pdf",
                Some("https://files.example.com/public.pdf".into()),
            ),
            entry("local.pdf", None),
            entry("twice.pdf", link(sub, "c3.pdf").await),
            entry("twice.pdf", link(sub, "d4.pdf").await),
            AttachmentManifestEntry {
                handle: Some(stored.clone()),
                ..entry("stored.pdf", link(sub, "e5.pdf").await)
            },
        ];

        let files = attach_own_uploads(&own, sub, &mut manifest);

        assert_eq!(files.keys().collect::<Vec<_>>(), ["scan.pdf"]);
        assert_eq!(files["scan.pdf"].key, key(sub, "a1.pdf"));
        assert_eq!(files["scan.pdf"].mime.as_deref(), Some("application/pdf"));
        assert_eq!(
            manifest[0].handle,
            Some(WorkspaceHandle::conversation_file(
                TURN_FILES,
                "scan.pdf",
                7,
                "",
                "application/pdf"
            ))
        );
        assert!(manifest[0].is_turn_file());
        assert!(manifest[1..6].iter().all(|entry| entry.handle.is_none()));
        assert_eq!(manifest[6].handle, Some(stored));
    }

    #[test]
    fn only_http_links_within_the_limit_are_considered() {
        let mut urls: Vec<String> = vec![
            "data:image/png;base64,AAAA".into(),
            "blob:https://app.example/1".into(),
            "not a url".into(),
        ];
        urls.extend((0..10).map(|n| format!("https://images.example.com/{n}.png")));
        let kept = http_urls(&urls);
        assert_eq!(
            kept.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            (3..MAX_ATTACHMENT_URLS).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_media_type_follows_the_link_extension() {
        for (raw, expected) in [
            (
                "https://x.example/a.JPG?X-Amz-Signature=b.png",
                "image/jpeg",
            ),
            ("https://x.example/a.jpeg", "image/jpeg"),
            ("https://x.example/a.gif", "image/gif"),
            ("https://x.example/a.webp", "image/webp"),
            ("https://x.example/a.png", "image/png"),
            ("https://x.example/a", "image/png"),
        ] {
            assert_eq!(attachment_media_type(&url(raw)), expected, "{raw}");
        }
    }

    #[flow_like_types::tokio::test]
    async fn only_own_uploads_and_public_links_reach_the_model() {
        let store = "127.0.0.1:9000";
        let own = StorageScope::signed_by(&s3_store(&format!("http://{store}")), "tmp/user/me/")
            .await
            .unwrap()
            .unwrap();
        let own_upload =
            format!("http://{store}/content/tmp/user/me/2026/10/28/k3x9.jpg?X-Amz-Signature=abc");
        let urls = [
            own_upload.clone(),
            format!("http://{store}/content/tmp/user/other/2026/10/28/k3x9.jpg"),
            format!("https://{store}/content/tmp/user/me/2026/10/28/k3x9.jpg"),
            "https://images.example.com/photo.webp".to_owned(),
        ]
        .iter()
        .enumerate()
        .map(|(index, raw)| (index, url(raw)))
        .collect();
        let images = checked_links(Some(&own), urls);
        let links: Vec<_> = images
            .iter()
            .map(|image| (image.url.as_deref().unwrap(), image.media_type.as_str()))
            .collect();
        assert_eq!(
            links,
            [
                (own_upload.as_str(), "image/jpeg"),
                ("https://images.example.com/photo.webp", "image/webp"),
            ]
        );
        assert!(images.iter().all(|image| image.data.is_empty()));
    }
}
