use crate::{
    entity::{course_asset, sea_orm_active_enums::AssetKind},
    routes::course::assets::course_asset_storage_path,
    state::AppState,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::{collections::HashMap, sync::OnceLock, time::Duration};

const SIGNED_URL_TTL_SECS: u64 = 60 * 60 * 12;

fn reference_pattern() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(^|[^\w\\])@([A-Za-z_][A-Za-z0-9_-]{0,63})")
            .expect("valid regex")
    })
}

fn render_asset_link(asset: &course_asset::Model, presigned_url: &str) -> String {
    let url = escape_url(presigned_url);
    let label = escape_label(&asset.name);
    match asset.kind {
        AssetKind::Image => format!("![{label}]({url})"),
        _ => format!("[{label}]({url})"),
    }
}

fn escape_label(label: &str) -> String {
    label
        .replace('\\', "\\\\")
        .replace(']', "\\]")
        .replace('[', "\\[")
}

fn escape_url(url: &str) -> String {
    url.replace(' ', "%20").replace(')', "%29").replace('(', "%28")
}

/// Replaces `@AssetName` references in markdown content with rendered markdown
/// pointing at presigned URLs for the matching course asset. Unknown references
/// are left untouched. Code-fenced blocks are skipped.
pub async fn resolve_asset_references(
    state: &AppState,
    course_id: &str,
    content: &str,
) -> String {
    if !content.contains('@') {
        return content.to_string();
    }

    let assets = match course_asset::Entity::find()
        .filter(course_asset::Column::CourseId.eq(course_id))
        .all(&state.db)
        .await
    {
        Ok(assets) => assets,
        Err(err) => {
            tracing::warn!(
                "Failed to load course assets for reference resolution (course {}): {:?}",
                course_id,
                err
            );
            return content.to_string();
        }
    };
    if assets.is_empty() {
        return content.to_string();
    }

    let by_name: HashMap<String, course_asset::Model> = assets
        .into_iter()
        .map(|asset| (asset.name.clone(), asset))
        .collect();

    let store = match state.master_credentials().await {
        Ok(creds) => match creds.to_store(false).await {
            Ok(store) => Some(store),
            Err(err) => {
                tracing::warn!(
                    "Failed to open master store for asset signing (course {}): {:?}",
                    course_id,
                    err
                );
                None
            }
        },
        Err(err) => {
            tracing::warn!(
                "Failed to load master credentials for asset signing (course {}): {:?}",
                course_id,
                err
            );
            None
        }
    };

    let mut signed_cache: HashMap<String, String> = HashMap::new();
    let mut output = String::with_capacity(content.len());
    for segment in split_preserving_code_fences(content) {
        if segment.is_code_fence {
            output.push_str(segment.text);
            continue;
        }
        let resolved =
            resolve_in_segment(segment.text, &by_name, store.as_ref(), &mut signed_cache).await;
        output.push_str(&resolved);
    }
    output
}

struct Segment<'a> {
    text: &'a str,
    is_code_fence: bool,
}

fn split_preserving_code_fences(content: &str) -> Vec<Segment<'_>> {
    let fence_re = regex::Regex::new(r"(?ms)^(```|~~~)[^\n]*\n.*?^\1\s*$").expect("valid fence");
    let mut segments = Vec::new();
    let mut last = 0;
    for m in fence_re.find_iter(content) {
        if m.start() > last {
            segments.push(Segment {
                text: &content[last..m.start()],
                is_code_fence: false,
            });
        }
        segments.push(Segment {
            text: &content[m.start()..m.end()],
            is_code_fence: true,
        });
        last = m.end();
    }
    if last < content.len() {
        segments.push(Segment {
            text: &content[last..],
            is_code_fence: false,
        });
    }
    segments
}

async fn resolve_in_segment(
    segment: &str,
    by_name: &HashMap<String, course_asset::Model>,
    store: Option<&flow_like_storage::files::store::FlowLikeStore>,
    signed_cache: &mut HashMap<String, String>,
) -> String {
    let re = reference_pattern();
    let mut result = String::with_capacity(segment.len());
    let mut last = 0;
    for caps in re.captures_iter(segment) {
        let whole = caps.get(0).expect("whole match");
        result.push_str(&segment[last..whole.start()]);
        let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let name = caps.get(2).expect("name capture").as_str();

        if let Some(asset) = by_name.get(name) {
            let signed = match signed_cache.get(&asset.id) {
                Some(url) => Some(url.clone()),
                None => sign_asset(asset, store).await.inspect(|url| {
                    signed_cache.insert(asset.id.clone(), url.clone());
                }),
            };
            if let Some(url) = signed {
                result.push_str(prefix);
                result.push_str(&render_asset_link(asset, &url));
                last = whole.end();
                continue;
            }
        }

        result.push_str(whole.as_str());
        last = whole.end();
    }
    result.push_str(&segment[last..]);
    result
}

async fn sign_asset(
    asset: &course_asset::Model,
    store: Option<&flow_like_storage::files::store::FlowLikeStore>,
) -> Option<String> {
    let store = store?;
    let path = course_asset_storage_path(&asset.course_id, &asset.storage_key);
    match store
        .sign("GET", &path, Duration::from_secs(SIGNED_URL_TTL_SECS))
        .await
    {
        Ok(url) => Some(url.to_string()),
        Err(err) => {
            tracing::warn!(
                "Failed to sign GET URL for asset {} ({}): {:?}",
                asset.id,
                asset.name,
                err
            );
            None
        }
    }
}
